//! Our log entry as p2panda's message type, and the forge that appends one.

use std::borrow::Borrow;

use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::traits::{Digest, Provenance};
use p2panda_core::{Body, Hash, Header, Operation, SigningKey, VerifyingKey};
use p2panda_spaces::{Forge, SpacesArgs};
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::{SqliteError, SqliteStore};

/// No conditions on access levels.
pub(crate) type Conditions = ();

/// What an entry's body carries: one spaces message.
pub(crate) type Args = SpacesArgs<Conditions>;

/// Each device writes exactly one log.
pub(crate) const LOG_ID: u32 = 0;

/// One entry: a signed header chained to the device's previous entry, and
/// the spaces message as the body the header commits to by hash.
///
/// The message is not put into the header's extensions, as p2panda's own
/// test forge does: the header decoder refuses any byte string or list
/// longer than 512, which a welcome carrying a few rotated keys, or any
/// payload of that size, exceeds.
#[derive(Clone, Debug)]
pub(crate) struct Message {
    pub operation: Operation,
    pub args: Args,
}

impl Message {
    /// Decodes the body. `None` when there is none or it is no spaces message.
    pub fn new(operation: Operation) -> Option<Self> {
        let args = decode_cbor(operation.body.as_ref()?.as_bytes()).ok()?;
        Some(Self { operation, args })
    }
}

// The spaces manager reads earlier entries back from the operation store
// through this conversion, which cannot fail. Only entries that went through
// `Message::new` on ingest, or were forged here, are ever stored.
impl From<Operation> for Message {
    fn from(operation: Operation) -> Self {
        Self::new(operation).expect("stored entries were decoded before they were stored")
    }
}

impl Borrow<Args> for Message {
    fn borrow(&self) -> &Args {
        &self.args
    }
}

impl Provenance<VerifyingKey> for Message {
    fn author(&self) -> VerifyingKey {
        self.operation.header.verifying_key
    }

    fn verify(&self) -> bool {
        self.operation.header.verify()
    }
}

impl Digest<Hash> for Message {
    fn hash(&self) -> Hash {
        self.operation.hash
    }
}

/// Signs spaces messages into this device's hash-chained log.
#[derive(Clone, Debug)]
pub(crate) struct LogForge {
    pub signing_key: SigningKey,
    pub store: SqliteStore,
}

/// Why forging an entry failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ForgeError {
    #[error(transparent)]
    Store(#[from] SqliteError),
    #[error("encoding the message: {0}")]
    Encode(String),
}

impl Forge<Conditions> for LogForge {
    type Message = Message;

    type Error = ForgeError;

    fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    // Runs inside the transaction of the spaces manager that calls it.
    async fn forge(&self, args: Args) -> Result<Message, ForgeError> {
        let author = self.signing_key.verifying_key();
        let (seq_num, backlink) = self
            .store
            .get_latest_entry_tx(&author, &LOG_ID)
            .await?
            .map(|latest| (latest.header.seq_num + 1, Some(latest.hash)))
            .unwrap_or((0, None));

        let body = encode_cbor(&args).map_err(|error| ForgeError::Encode(error.to_string()))?;
        let header = Header::builder()
            .seq_num(seq_num)
            .backlink(backlink)
            .body(&body)
            .build(&self.signing_key, ());
        let operation = Operation::from_parts(header, Some(Body::from_bytes(&body)));

        self.store
            .insert_operation_tx(&operation.hash, &operation, &LOG_ID)
            .await?;

        Ok(Message { operation, args })
    }
}

//! Team keys: making one, sealing it to a device, and encrypting payloads.
//!
//! A team key is 32 random bytes, named by its hash. It reaches a device as
//! a lockbox: HPKE (RFC 9180) in base mode with X25519, HKDF-SHA256 and
//! ChaCha20-Poly1305, to the sealing key the device announced. Payloads are
//! XChaCha20-Poly1305 under the team key with a random nonce; team, author
//! and position are authenticated with them.
//!
//! What this gives: whoever holds a key reads everything written under it,
//! before or after they got it. A removed device keeps what it could read,
//! and reads nothing written under a later key.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::SigningKey;
use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};
use sha2::{Digest, Sha256};
use teamlog::{DeviceId, TeamId};

use crate::Error;
use crate::rules::KeyId;

const KEY_ID_CONTEXT: &[u8] = b"rerusted/key-id/1";
const LOCKBOX_CONTEXT: &[u8] = b"rerusted/lockbox/1";
const PAYLOAD_CONTEXT: &[u8] = b"rerusted/payload/1";
const SEALING_CONTEXT: &[u8] = b"rerusted/sealing/1";
const SIGNING_CONTEXT: &[u8] = b"rerusted/signing/1";

/// The length of X25519's encapsulated key, the head of a lockbox.
const ENCAPPED_LEN: usize = 32;

pub type TeamKey = [u8; 32];

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the system has a random source");
    bytes
}

pub fn new_key() -> TeamKey {
    random()
}

pub fn new_seed() -> [u8; 32] {
    random()
}

/// The key a device signs its entries with, derived from its seed.
pub fn signing_key(seed: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(
        &Sha256::new()
            .chain_update(SIGNING_CONTEXT)
            .chain_update(seed)
            .finalize()
            .into(),
    )
}

pub fn key_id(key: &TeamKey) -> KeyId {
    KeyId(
        Sha256::new()
            .chain_update(KEY_ID_CONTEXT)
            .chain_update(key)
            .finalize()
            .into(),
    )
}

/// The key pair lockboxes are sealed to, derived from the device's seed.
pub struct Sealing {
    secret: <X25519HkdfSha256 as Kem>::PrivateKey,
    pub public: [u8; 32],
}

impl Sealing {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let (secret, public) =
            X25519HkdfSha256::derive_keypair(&[SEALING_CONTEXT, seed.as_slice()].concat());
        Self {
            secret,
            public: public.to_bytes().into(),
        }
    }

    /// Opens a lockbox addressed to `device`. `None` when it does not open
    /// or holds another key than it names.
    pub fn open(&self, id: KeyId, device: DeviceId, sealed: &[u8]) -> Option<TeamKey> {
        let (encapped, text) = sealed.split_at_checked(ENCAPPED_LEN)?;
        let encapped = <X25519HkdfSha256 as Kem>::EncappedKey::from_bytes(encapped).ok()?;
        let key = hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeR::Base,
            &self.secret,
            &encapped,
            &info(id, device),
            text,
            &[],
        )
        .ok()?;
        let key: TeamKey = key.try_into().ok()?;
        (key_id(&key) == id).then_some(key)
    }
}

fn info(id: KeyId, device: DeviceId) -> Vec<u8> {
    [LOCKBOX_CONTEXT, &id.0, &device.0].concat()
}

/// Seals `key` to the sealing key `public` that `device` announced.
pub fn seal(key: &TeamKey, device: DeviceId, public: &[u8; 32]) -> Result<Vec<u8>, Error> {
    let public = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(public)
        .map_err(|_| Error::Sealing(device))?;
    let (encapped, text) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeS::Base,
            &public,
            &info(key_id(key), device),
            key,
            &[],
        )
        .map_err(|_| Error::Sealing(device))?;
    Ok([encapped.to_bytes().as_slice(), &text].concat())
}

fn associated(team: TeamId, author: DeviceId, seq: u64) -> Vec<u8> {
    [PAYLOAD_CONTEXT, &team.0, &author.0, &seq.to_be_bytes()].concat()
}

pub fn encrypt(
    key: &TeamKey,
    team: TeamId,
    author: DeviceId,
    seq: u64,
    data: &[u8],
) -> ([u8; 24], Vec<u8>) {
    let nonce: [u8; 24] = random();
    let text = XChaCha20Poly1305::new(key.into())
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: data,
                aad: &associated(team, author, seq),
            },
        )
        .expect("a payload fits the cipher's limit");
    (nonce, text)
}

pub fn decrypt(
    key: &TeamKey,
    team: TeamId,
    author: DeviceId,
    seq: u64,
    nonce: &[u8; 24],
    text: &[u8],
) -> Option<Vec<u8>> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: text,
                aad: &associated(team, author, seq),
            },
        )
        .ok()
}

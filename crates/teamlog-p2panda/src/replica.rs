//! One replica: p2panda's spaces manager behind `teamlog::TeamLog`.

use std::collections::BTreeMap;
use std::path::Path;

use p2panda_auth::Access;
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::operation::{validate_backlink, validate_operation};
use p2panda_core::{Body, Hash, Header, Operation, VerifyingKey};
use p2panda_encryption::Rng;
use p2panda_spaces::manager::Manager;
use p2panda_spaces::space::Space;
use p2panda_spaces::{Credentials, Event as SpacesEvent, SpaceEvent, SpacesArgs};
use p2panda_store::operations::OperationStore;
use p2panda_store::spaces::{SpacesStore, SqliteSpacesStore};
use p2panda_store::{SqliteStore, SqliteStoreBuilder, Transaction};
use sqlx::SqlitePool;
use teamlog::{DeviceId, ENTRY_VERSION, Entry, EntryId, Event, Payload, Role, TeamId, TeamLog};

use crate::forge::{Conditions, LOG_ID, LogForge, Message};
use crate::tables::{self, APPLIED, FAILED, PENDING};

type Store = SqliteSpacesStore<()>;
type SpacesManager = Manager<Store, LogForge, Conditions>;
type SpacesState = p2panda_spaces::SpacesStoreState<Conditions>;

/// The version of the envelope around a payload before it is encrypted.
const PAYLOAD_VERSION: u8 = 1;
/// version, author, n.
const PAYLOAD_HEADER: usize = 1 + 32 + 8;

/// Everything that can go wrong in the adapter. p2panda's own error types are
/// generic over the store and the forge, so they are carried as text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("entry version {0} is not supported (this replica writes {ENTRY_VERSION})")]
    UnsupportedVersion(u8),
    #[error("entry does not decode or verify: {0}")]
    Invalid(String),
    #[error("entry {seq} of {author:?} arrived but the log is held up to {held}")]
    Gap {
        author: DeviceId,
        seq: u64,
        held: u64,
    },
    #[error("{author:?} signed two different entries at position {seq}")]
    Fork { author: DeviceId, seq: u64 },
    #[error("this device is not a member of {0:?}")]
    NotMember(TeamId),
    #[error("storage: {0}")]
    Storage(String),
    #[error("p2panda: {0}")]
    Backend(String),
}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<p2panda_store::SqliteError> for Error {
    fn from(error: p2panda_store::SqliteError) -> Self {
        Self::Storage(error.to_string())
    }
}

fn backend(error: impl std::fmt::Display) -> Error {
    Error::Backend(error.to_string())
}

fn device(key: VerifyingKey) -> DeviceId {
    DeviceId(*key.as_bytes())
}

fn key(device: DeviceId) -> Result<VerifyingKey, Error> {
    VerifyingKey::from_bytes(&device.0).map_err(|error| Error::Invalid(error.to_string()))
}

fn team(space: Hash) -> TeamId {
    TeamId(*space.as_bytes())
}

fn space_id(team: TeamId) -> Hash {
    Hash::from(team.0)
}

fn role(access: &Access<Conditions>) -> Role {
    if access.is_manage() {
        Role::Admin
    } else {
        Role::Member
    }
}

fn access(role: Role) -> Access<Conditions> {
    match role {
        Role::Admin => Access::manage(),
        Role::Member => Access::write(),
    }
}

/// `Entry.bytes`: the header's length as four bytes, the header, the body.
fn entry(message: &Message) -> Entry {
    let header = &message.operation.header;
    let encoded = header.encode();
    let mut bytes = (encoded.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(&encoded);
    if let Some(body) = &message.operation.body {
        bytes.extend_from_slice(body.as_bytes());
    }
    Entry {
        version: ENTRY_VERSION,
        author: device(header.verifying_key),
        seq: u64::from(header.seq_num),
        id: EntryId(*message.operation.hash.as_bytes()),
        bytes,
    }
}

/// A replica backed by one SQLite file.
pub struct Replica {
    device: DeviceId,
    manager: SpacesManager,
    store: SqliteStore,
    spaces: Store,
    pool: SqlitePool,
    /// Payloads p2panda decrypted whose envelope did not match the entry.
    forged: u64,
}

impl Replica {
    /// Opens the replica stored at `path`, creating it with fresh keys when
    /// the file does not exist.
    ///
    /// Spike only: the device's secret keys lie unprotected in the same file.
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let url = format!("sqlite://{}", path.display());
        let store = SqliteStoreBuilder::new()
            .database_url(&url)
            .create_database(true)
            .build()
            .await?;
        let pool = store.pool().clone();
        tables::create(&pool).await?;

        let rng = Rng::default();
        // The identity secret has no public constructor from bytes, so the
        // credentials are kept in p2panda's own serde form.
        let credentials: Credentials = match tables::meta(&pool, "credentials").await? {
            Some(bytes) => decode_cbor(&bytes[..]).map_err(|e| Error::Storage(e.to_string()))?,
            None => {
                let credentials = Credentials::from_rng(&rng).map_err(backend)?;
                let bytes = encode_cbor(&credentials).map_err(|e| Error::Storage(e.to_string()))?;
                tables::set_meta(&pool, "credentials", &bytes).await?;
                credentials
            }
        };
        let signing_key = credentials.signing_key();
        let spaces = Store::new(store.clone());
        let forge = LogForge {
            signing_key: signing_key.clone(),
            store: store.clone(),
        };
        let manager =
            SpacesManager::new(spaces.clone(), forge, credentials, rng).map_err(backend)?;

        Ok(Self {
            device: device(signing_key.verifying_key()),
            manager,
            store,
            spaces,
            pool,
            forged: 0,
        })
    }

    /// Entries waiting for an entry they depend on.
    pub async fn waiting(&self) -> Result<Vec<Entry>, Error> {
        let waiting = tables::in_state(&self.pool, PENDING).await?;
        Ok(waiting.into_iter().map(|(entry, _)| entry).collect())
    }

    /// Entries p2panda refused, each with its reason.
    pub async fn failed(&self) -> Result<Vec<(Entry, String)>, Error> {
        let failed = tables::in_state(&self.pool, FAILED).await?;
        Ok(failed
            .into_iter()
            .map(|(entry, reason)| (entry, reason.unwrap_or_default()))
            .collect())
    }

    /// Payloads dropped because the author named inside the ciphertext was
    /// not the author who signed the entry.
    pub fn forged_payloads(&self) -> u64 {
        self.forged
    }

    /// Members that were removed from the team and not added again.
    pub async fn removed(&self, team: TeamId) -> Result<Vec<DeviceId>, Error> {
        let mut removed: Vec<_> = self
            .manager
            .removed_space_members(space_id(team))
            .await
            .map_err(backend)?
            .into_iter()
            .map(device)
            .collect();
        removed.sort();
        Ok(removed)
    }

    /// Records entries this device just forged. They are applied already.
    async fn record(&self, messages: Vec<Message>) -> Result<Vec<Entry>, Error> {
        let mut entries = Vec::with_capacity(messages.len());
        for message in &messages {
            let entry = entry(message);
            tables::insert_entry(&self.pool, &entry, APPLIED).await?;
            entries.push(entry);
        }
        Ok(entries)
    }

    /// The team's space when this device may read and write it.
    async fn joined(&self, team: TeamId) -> Result<Space<Store, LogForge, Conditions>, Error> {
        let id = space_id(team);
        if !self.manager.has_joined_space(id).await.map_err(backend)? {
            return Err(Error::NotMember(team));
        }
        // Resolves at once: the space is known.
        self.manager.space(id).joined().await.map_err(backend)
    }

    /// Translates p2panda's events. `source` is the entry being applied when
    /// the events came from one; payloads are then checked against its author.
    async fn translate(
        &mut self,
        events: Vec<SpacesEvent<Conditions>>,
        source: Option<&Message>,
    ) -> Result<Vec<Event>, Error> {
        let mut out = Vec::new();
        for event in events {
            match event {
                SpacesEvent::Member(_) | SpacesEvent::Groups(_) => {}
                SpacesEvent::Application { space_id, data } => {
                    let team = team(space_id);
                    let Some(payload) = open_payload(&data) else {
                        self.forged += 1;
                        continue;
                    };
                    // Only checkable when the payload came straight out of
                    // its own entry. Payloads p2panda had buffered until the
                    // welcome arrived come without their entry (finding 5).
                    if let Some(message) = source
                        && matches!(message.args, SpacesArgs::Application { .. })
                        && device(message.operation.header.verifying_key) != payload.author
                    {
                        self.forged += 1;
                        continue;
                    }
                    if tables::insert_payload(&self.pool, team, &payload).await? {
                        out.push(Event::Payload { team, payload });
                    }
                }
                SpacesEvent::Spaces(event) => match event {
                    SpaceEvent::Created {
                        space_id,
                        initial_members,
                        ..
                    } => {
                        out.extend(initial_members.iter().map(|(member, access)| {
                            Event::MemberAdded {
                                team: team(space_id),
                                member: device(*member),
                                role: role(access),
                            }
                        }));
                    }
                    SpaceEvent::Added {
                        space_id, added, ..
                    } => {
                        out.extend(added.iter().map(|(member, access)| Event::MemberAdded {
                            team: team(space_id),
                            member: device(*member),
                            role: role(access),
                        }));
                    }
                    SpaceEvent::Removed {
                        space_id, removed, ..
                    } => {
                        out.extend(removed.iter().map(|(member, _)| Event::MemberRemoved {
                            team: team(space_id),
                            member: device(*member),
                        }));
                    }
                    SpaceEvent::Promoted {
                        space_id,
                        promoted: changed,
                        ..
                    }
                    | SpaceEvent::Demoted {
                        space_id,
                        demoted: changed,
                        ..
                    } => {
                        out.extend(changed.iter().map(|(member, access)| Event::RoleChanged {
                            team: team(space_id),
                            member: device(*member),
                            role: role(access),
                        }));
                    }
                    SpaceEvent::Ejected { space_id } => {
                        out.push(Event::Ejected {
                            team: team(space_id),
                        });
                    }
                },
            }
        }
        Ok(out)
    }

    async fn change(
        &mut self,
        output: p2panda_spaces::space::SpaceOutput<Conditions, Message>,
    ) -> Result<Vec<Entry>, Error> {
        let messages = output
            .messages
            .into_iter()
            .map(|(message, _)| message)
            .collect();
        self.record(messages).await
    }

    /// Applies every pending entry whose dependencies were tried, until
    /// nothing moves any more.
    ///
    /// p2panda returns an error for an entry it cannot use, for example a
    /// payload under a key this device never got. Holding such an entry back
    /// would hold back everything that names it as a dependency, so it counts
    /// as tried, and it is tried again each time another entry was applied: a
    /// later welcome can bring the missing key.
    async fn drain(&mut self) -> Result<Vec<Event>, Error> {
        let mut events = Vec::new();
        loop {
            let mut progress = false;
            let mut candidates = tables::in_state(&self.pool, PENDING).await?;
            candidates.extend(tables::in_state(&self.pool, FAILED).await?);
            for (entry, reason) in candidates {
                let failed_before = reason.is_some();
                let message = decode(&entry)?;
                let mut dependencies = message.args.dependencies();
                dependencies.extend(message.operation.header.backlink);
                let mut ready = true;
                for dependency in dependencies {
                    let state = tables::state(&self.pool, dependency.as_bytes()).await?;
                    if !matches!(state, Some(APPLIED | FAILED)) {
                        ready = false;
                        break;
                    }
                }
                if !ready {
                    continue;
                }
                // p2panda 0.8 panics on this message kind (`unimplemented!()`
                // in `Manager::process`); nothing here forges it, so one that
                // arrives is refused instead of taking the process down.
                let result = if matches!(message.args, SpacesArgs::SpaceUpdate { .. }) {
                    Err("key rotation without a membership change is not implemented".to_string())
                } else {
                    self.manager
                        .process(&message)
                        .await
                        .map_err(|error| error.to_string())
                };
                match result {
                    Ok(processed) => {
                        tables::set_state(&self.pool, entry.id, APPLIED, None).await?;
                        events.extend(self.translate(processed, Some(&message)).await?);
                        progress = true;
                    }
                    Err(reason) => {
                        tables::set_state(&self.pool, entry.id, FAILED, Some(&reason)).await?;
                        progress |= !failed_before;
                    }
                }
            }
            if !progress {
                return Ok(events);
            }
        }
    }

    /// p2panda keeps one shared membership state and one copy per space. A
    /// membership change made by someone who could not write into the space
    /// at that moment reaches only the shared state; a current member then
    /// has to append a pointer to it. This does that for every joined space.
    async fn repair(&mut self) -> Result<(Vec<Event>, Vec<Entry>), Error> {
        let mut events = Vec::new();
        let mut entries = Vec::new();
        let ids = SpacesStore::<SpacesState>::space_ids(&self.spaces).await?;
        for id in ids {
            if !self.manager.has_joined_space(id).await.map_err(backend)? {
                continue;
            }
            let space = self.manager.space(id).joined().await.map_err(backend)?;
            let group = space.group_id().await.map_err(backend)?;
            if !self
                .manager
                .space_repair_required(id, &[group])
                .await
                .map_err(backend)?
            {
                continue;
            }
            let output = self
                .manager
                .repair_space(id, &[group])
                .await
                .map_err(backend)?;
            for (message, repaired) in output.messages {
                // The output repeats the membership entries it points at;
                // only the entries this device forged are new.
                if device(message.operation.header.verifying_key) == self.device
                    && tables::state(&self.pool, message.operation.hash.as_bytes())
                        .await?
                        .is_none()
                {
                    entries.extend(self.record(vec![message]).await?);
                }
                events.extend(self.translate(repaired, None).await?);
            }
        }
        Ok((events, entries))
    }
}

fn decode(entry: &Entry) -> Result<Message, Error> {
    let invalid = |reason: &str| Error::Invalid(reason.to_string());
    let (len, rest) = entry
        .bytes
        .split_at_checked(4)
        .ok_or_else(|| invalid("too short"))?;
    let len = u32::from_be_bytes(len.try_into().expect("four bytes")) as usize;
    let (header, body) = rest
        .split_at_checked(len)
        .ok_or_else(|| invalid("header cut off"))?;
    let header = Header::<()>::decode(header).map_err(|e| Error::Invalid(e.to_string()))?;
    let operation = Operation::from_parts(header, Some(Body::from_bytes(body)));
    // Signature, and that the body is the one the header commits to.
    validate_operation(&operation).map_err(|e| Error::Invalid(e.to_string()))?;
    Message::new(operation).ok_or_else(|| invalid("the body is not a spaces message"))
}

fn seal_payload(author: DeviceId, n: u64, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PAYLOAD_HEADER + data.len());
    bytes.push(PAYLOAD_VERSION);
    bytes.extend_from_slice(&author.0);
    bytes.extend_from_slice(&n.to_be_bytes());
    bytes.extend_from_slice(data);
    bytes
}

fn open_payload(bytes: &[u8]) -> Option<Payload> {
    if bytes.len() < PAYLOAD_HEADER || bytes[0] != PAYLOAD_VERSION {
        return None;
    }
    Some(Payload {
        author: DeviceId(bytes[1..33].try_into().ok()?),
        n: u64::from_be_bytes(bytes[33..PAYLOAD_HEADER].try_into().ok()?),
        data: bytes[PAYLOAD_HEADER..].to_vec(),
    })
}

impl TeamLog for Replica {
    type Error = Error;

    fn device(&self) -> DeviceId {
        self.device
    }

    async fn announce(&mut self) -> Result<Vec<Entry>, Error> {
        let message = self.manager.rotate_key_bundle().await.map_err(backend)?;
        self.record(message.into_iter().collect()).await
    }

    async fn create_team(&mut self) -> Result<(TeamId, Vec<Entry>), Error> {
        let id: [u8; 32] = Rng::default().random_array().map_err(backend)?;
        let (_, output) = self
            .manager
            .create_space(Hash::from(id), &[])
            .await
            .map_err(backend)?;
        Ok((TeamId(id), self.change(output).await?))
    }

    async fn add_member(
        &mut self,
        team: TeamId,
        member: DeviceId,
        role: Role,
    ) -> Result<Vec<Entry>, Error> {
        let space = self.joined(team).await?;
        let output = space
            .add(key(member)?, access(role))
            .await
            .map_err(backend)?;
        self.change(output).await
    }

    async fn remove_member(&mut self, team: TeamId, member: DeviceId) -> Result<Vec<Entry>, Error> {
        let space = self.joined(team).await?;
        let output = space.remove(key(member)?).await.map_err(backend)?;
        self.change(output).await
    }

    async fn set_role(
        &mut self,
        team: TeamId,
        member: DeviceId,
        role: Role,
    ) -> Result<Vec<Entry>, Error> {
        let space = self.joined(team).await?;
        let member = key(member)?;
        let output = match role {
            Role::Admin => space.promote(member, access(role)).await,
            Role::Member => space.demote(member, access(role)).await,
        }
        .map_err(backend)?;
        self.change(output).await
    }

    async fn members(&self, team: TeamId) -> Result<Vec<(DeviceId, Role)>, Error> {
        let id = space_id(team);
        if !self.manager.has_space(id).await.map_err(backend)? {
            return Ok(Vec::new());
        }
        let space = self.manager.space(id).created().await.map_err(backend)?;
        let mut members: Vec<_> = space
            .members()
            .await
            .map_err(backend)?
            .iter()
            .map(|(member, access)| (device(*member), role(access)))
            .collect();
        members.sort();
        Ok(members)
    }

    async fn write(&mut self, team: TeamId, data: &[u8]) -> Result<Vec<Entry>, Error> {
        let space = self.joined(team).await?;
        let n = tables::payloads_by(&self.pool, team, self.device).await?;
        let sealed = seal_payload(self.device, n, data);
        let (message, _) = space.publish(&sealed).await.map_err(backend)?;
        let payload = Payload {
            author: self.device,
            n,
            data: data.to_vec(),
        };
        tables::insert_payload(&self.pool, team, &payload).await?;
        self.record(vec![message]).await
    }

    async fn history(&self, team: TeamId) -> Result<Vec<Payload>, Error> {
        Ok(tables::history(&self.pool, team).await?)
    }

    async fn heights(&self) -> Result<BTreeMap<DeviceId, u64>, Error> {
        Ok(tables::heights(&self.pool).await?.into_iter().collect())
    }

    async fn entries_after(&self, device: DeviceId, from: u64) -> Result<Vec<Entry>, Error> {
        Ok(tables::entries_after(&self.pool, device, from).await?)
    }

    async fn ingest(&mut self, incoming: Entry) -> Result<(Vec<Event>, Vec<Entry>), Error> {
        if incoming.version != ENTRY_VERSION {
            return Err(Error::UnsupportedVersion(incoming.version));
        }
        let message = decode(&incoming)?;
        // The outer fields are a convenience for transports; what counts is
        // what the signed bytes say.
        let incoming = entry(&message);

        if tables::state(&self.pool, &incoming.id.0).await?.is_some() {
            return Ok((Vec::new(), Vec::new()));
        }
        if tables::entry_at(&self.pool, incoming.author, incoming.seq)
            .await?
            .is_some()
        {
            return Err(Error::Fork {
                author: incoming.author,
                seq: incoming.seq,
            });
        }
        let held = tables::entries_after(&self.pool, incoming.author, 0)
            .await?
            .len() as u64;
        if incoming.seq != held {
            return Err(Error::Gap {
                author: incoming.author,
                seq: incoming.seq,
                held,
            });
        }
        if incoming.seq > 0 {
            let previous = tables::entry_at(&self.pool, incoming.author, incoming.seq - 1)
                .await?
                .expect("held without gaps");
            validate_backlink(&decode(&previous)?.operation, &message.operation)
                .map_err(|e| Error::Invalid(e.to_string()))?;
        }

        // Into p2panda's operation store, where the spaces manager looks
        // entries up, and into our index.
        let permit = self.store.begin().await?;
        self.store
            .insert_operation_tx(&message.operation.hash, &message.operation, &LOG_ID)
            .await?;
        self.store.commit(permit).await?;
        tables::insert_entry(&self.pool, &incoming, PENDING).await?;

        let mut events = self.drain().await?;
        let mut entries = Vec::new();
        if tables::in_state(&self.pool, PENDING).await?.is_empty() {
            let (repaired, forged) = self.repair().await?;
            events.extend(repaired);
            entries = forged;
        }
        Ok((events, entries))
    }
}

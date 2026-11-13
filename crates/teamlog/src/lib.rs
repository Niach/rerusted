//! The trait a team's sync log has to fulfil, in our own types only.
//!
//! Spike for #RUST-30: a signed append-only log per device, team
//! membership, and a team key that is rotated when a member is removed. This
//! crate names no implementation; `teamlog-p2panda` is the adapter under test.
//!
//! A replica is driven in two directions:
//!
//! - local actions (`create_team`, `add_member`, `write`, ...) return the
//!   [`Entry`]s they appended to this device's own log;
//! - [`TeamLog::ingest`] takes an entry from another device and returns the
//!   [`Event`]s it caused, possibly none while it waits for entries it
//!   depends on.
//!
//! Transport is not part of the trait: [`TeamLog::heights`] and
//! [`TeamLog::entries_after`] are all a transport needs to bring two replicas
//! to the same set of entries.

use std::collections::BTreeMap;
use std::fmt;

/// The version of [`Entry`] as it is exchanged and stored (#RUST-29).
pub const ENTRY_VERSION: u8 = 1;

macro_rules! bytes32 {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; 32]);

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}(", stringify!($name))?;
                for byte in &self.0[..4] {
                    write!(f, "{byte:02x}")?;
                }
                write!(f, "…)")
            }
        }
    };
}

bytes32!(
    /// A device's public signing key. In the spike one member is one device.
    DeviceId
);
bytes32!(
    /// A team, chosen at random by the device that creates it.
    TeamId
);
bytes32!(
    /// The hash of an entry's signed bytes.
    EntryId
);

/// What a member may do in a team.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Reads and writes payloads.
    Member,
    /// Also adds, removes and changes the role of members.
    Admin,
}

/// One signed entry of one device's log, as it travels between replicas.
///
/// `bytes` are opaque to everything but the implementation that wrote them;
/// `author`, `seq` and `id` are repeated outside so a transport can order and
/// deduplicate without decoding. An implementation must not trust the outer
/// fields: it recomputes them from `bytes` on ingest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// [`ENTRY_VERSION`] of the writer.
    pub version: u8,
    pub author: DeviceId,
    /// Position in the author's log, starting at 0, without gaps.
    pub seq: u64,
    pub id: EntryId,
    pub bytes: Vec<u8>,
}

/// One decrypted payload of a team's history.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Payload {
    pub author: DeviceId,
    /// The author's own count of payloads written to this team, from 0.
    pub n: u64,
    pub data: Vec<u8>,
}

/// What ingesting an entry changed on this replica.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    MemberAdded {
        team: TeamId,
        member: DeviceId,
        role: Role,
    },
    MemberRemoved {
        team: TeamId,
        member: DeviceId,
    },
    RoleChanged {
        team: TeamId,
        member: DeviceId,
        role: Role,
    },
    /// A payload became readable.
    Payload {
        team: TeamId,
        payload: Payload,
    },
    /// This device was removed from the team.
    Ejected {
        team: TeamId,
    },
}

/// A replica of the logs of every device it has heard of.
#[allow(async_fn_in_trait, reason = "spike: single-threaded callers only")]
pub trait TeamLog {
    type Error: std::error::Error;

    /// This replica's device.
    fn device(&self) -> DeviceId;

    /// Publishes what others need before they can add this device to a team.
    /// Returns nothing when that is already published and still valid.
    async fn announce(&mut self) -> Result<Vec<Entry>, Self::Error>;

    /// Creates a team with this device as its only member, an admin.
    async fn create_team(&mut self) -> Result<(TeamId, Vec<Entry>), Self::Error>;

    async fn add_member(
        &mut self,
        team: TeamId,
        member: DeviceId,
        role: Role,
    ) -> Result<Vec<Entry>, Self::Error>;

    /// Removes a member and rotates the team key.
    async fn remove_member(
        &mut self,
        team: TeamId,
        member: DeviceId,
    ) -> Result<Vec<Entry>, Self::Error>;

    async fn set_role(
        &mut self,
        team: TeamId,
        member: DeviceId,
        role: Role,
    ) -> Result<Vec<Entry>, Self::Error>;

    /// The member list as this replica sees it now, sorted by device.
    async fn members(&self, team: TeamId) -> Result<Vec<(DeviceId, Role)>, Self::Error>;

    /// Encrypts `data` to the team's current key and appends it.
    async fn write(&mut self, team: TeamId, data: &[u8]) -> Result<Vec<Entry>, Self::Error>;

    /// Every payload this replica could decrypt, sorted by author, then `n`.
    async fn history(&self, team: TeamId) -> Result<Vec<Payload>, Self::Error>;

    /// The number of entries held of each device's log.
    async fn heights(&self) -> Result<BTreeMap<DeviceId, u64>, Self::Error>;

    /// The entries of `device`'s log from position `from` on, in order.
    async fn entries_after(&self, device: DeviceId, from: u64) -> Result<Vec<Entry>, Self::Error>;

    /// Verifies and stores an entry from another replica and applies whatever
    /// became applicable. Ingesting a known entry changes nothing.
    ///
    /// Besides events it returns entries this device appended in reaction
    /// (an implementation may have to acknowledge a membership change).
    async fn ingest(&mut self, entry: Entry) -> Result<(Vec<Event>, Vec<Entry>), Self::Error>;
}

//! A team's signed log, its rules and its keys. Spike for #RUST-17 after the
//! research on #RUST-3: our own log instead of p2panda.
//!
//! Pure logic and cryptography: no network, no storage, nothing async but
//! the signatures of the [`teamlog::TeamLog`] trait, whose futures here are
//! always ready.
//!
//! - [`rules`]: who is a member and which key is current, as a function of
//!   the operations held. This is where the decisions live.
//! - [`format`]: the bytes of an entry and its signature.
//! - [`keys`]: team keys, lockboxes, payload encryption.
//! - [`Replica`]: the three together behind the `TeamLog` trait.
//!
//! - [`user`]: the user's seed, its 24 words and the user key.
//! - [`certificate`]: the signatures that lead from the user key to a
//!   device.
//!
//! In the log one member is still one device key, as in the p2panda spike:
//! user keys and certificates exist beside it and the rules do not read
//! them yet.
//!
//! A device key must never be restored from a backup and used again: the
//! restored device continues its log from an old position, which is a fork,
//! and the device is out of every team for good.

pub mod certificate;
pub mod format;
pub mod keys;
mod replica;
pub mod rules;
pub mod user;

use std::fmt;

use teamlog::DeviceId;

pub use replica::Replica;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The entry is of a format version this replica does not read.
    UnsupportedVersion(u8),
    Malformed(&'static str),
    BadSignature,
    /// The fields outside the entry's bytes are not those inside.
    Mismatch,
    /// The entry is not the next one of its author's log.
    Gap {
        have: u64,
        got: u64,
    },
    /// The entry does not continue the author's log as held.
    Broken,
    /// This device's own log is forked: nothing it writes counts any more.
    Forked,
    UnknownTeam,
    /// This device, or the one named, is no member of the team.
    NotMember,
    AlreadyMember,
    /// The rules give this device no right to do that.
    NotAllowed,
    /// The device has not published a sealing key yet.
    NotAnnounced(DeviceId),
    /// The device's sealing key cannot be used.
    Sealing(DeviceId),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(f, "unsupported entry version {version}"),
            Self::Malformed(what) => write!(f, "malformed entry: {what}"),
            Self::BadSignature => write!(f, "the signature does not verify"),
            Self::Mismatch => write!(f, "outer fields differ from the signed entry"),
            Self::Gap { have, got } => write!(f, "entry {got} of a log held up to {have}"),
            Self::Broken => write!(f, "the entry does not continue the log"),
            Self::Forked => write!(f, "this device's log is forked"),
            Self::UnknownTeam => write!(f, "unknown team"),
            Self::NotMember => write!(f, "not a member"),
            Self::AlreadyMember => write!(f, "already a member"),
            Self::NotAllowed => write!(f, "not allowed by the team's rules"),
            Self::NotAnnounced(device) => write!(f, "{device:?} has not announced"),
            Self::Sealing(device) => write!(f, "cannot seal to {device:?}"),
        }
    }
}

impl std::error::Error for Error {}

//! Invites: how somebody who is in no team yet asks to be let into one
//! (#RUST-17, #RUST-44).
//!
//! An invite is a secret that an admin hands over outside the log, as a
//! link or a QR code. Whoever holds it can ask once, with one device, to be
//! let in. The asking is not an entry: it goes to the inviter, whose device
//! writes the admission. Nobody is in a team before that entry.
//!
//! ```text
//! invite     = version team inviter expires secret          105 bytes
//! version    = 1 byte, INVITE_VERSION
//! team       = 32 bytes
//! inviter    = 32 bytes, the device that wrote the invite into its log and
//!              that the request goes to
//! expires    = u64, seconds since 1970-01-01 UTC: the first second the
//!              invite no longer works in
//! secret     = 32 bytes, random
//!
//! invite key = Ed25519, secret = SHA-256("rerusted/invite/1" secret)
//!
//! request    = body signature                               161 bytes
//! body       = version team invite device                   97 bytes
//! invite     = 32 bytes, the invite key's public key
//! device     = 32 bytes, the device that asks to be let in
//! signature  = 64 bytes, Ed25519 by the invite key over
//!              "rerusted/join/1" body
//! ```
//!
//! Integers are big-endian. A decoder accepts exactly these bytes.
//!
//! In the log the invite is its public key and its expiry (entry kind 13),
//! so the secret is known to the inviter and the invited only. The
//! admission (kind 14) carries the request's signature: every replica
//! checks that the device was asked in with that invite, and that the
//! invite was not used before.
//!
//! The expiry is the one place a clock is read, and it is read once, by the
//! inviter's device when it is asked. The log's rules know no time: an
//! admission that an admin wrote counts, late or not, as any admission by
//! an admin does.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use teamlog::{DeviceId, TeamId};

use crate::{Error, keys};

/// The version of the invite and of the request, the first byte of each.
pub const INVITE_VERSION: u8 = 1;

pub const INVITE_LEN: usize = 1 + 32 + 32 + 8 + 32;
pub const REQUEST_LEN: usize = BODY_LEN + 64;

const INVITE_CONTEXT: &[u8] = b"rerusted/invite/1";
const JOIN_CONTEXT: &[u8] = b"rerusted/join/1";
const BODY_LEN: usize = 1 + 32 + 32 + 32;

/// What an invite link carries. Whoever holds it can ask to be let in.
#[derive(Clone, PartialEq, Eq)]
pub struct Invite {
    pub team: TeamId,
    pub inviter: DeviceId,
    pub expires: u64,
    secret: [u8; 32],
}

impl std::fmt::Debug for Invite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invite({:?}, {:?}, …)", self.team, self.inviter)
    }
}

fn signed(team: TeamId, invite: &[u8; 32], device: DeviceId) -> Vec<u8> {
    let mut body = vec![INVITE_VERSION];
    body.extend_from_slice(&team.0);
    body.extend_from_slice(invite);
    body.extend_from_slice(&device.0);
    body
}

impl Invite {
    /// A new invite with a random secret.
    pub fn new(team: TeamId, inviter: DeviceId, expires: u64) -> Self {
        Self::from_secret(team, inviter, expires, keys::new_seed())
    }

    pub fn from_secret(team: TeamId, inviter: DeviceId, expires: u64, secret: [u8; 32]) -> Self {
        Self {
            team,
            inviter,
            expires,
            secret,
        }
    }

    fn signing(&self) -> SigningKey {
        SigningKey::from_bytes(
            &Sha256::new()
                .chain_update(INVITE_CONTEXT)
                .chain_update(self.secret)
                .finalize()
                .into(),
        )
    }

    /// The invite's public key: what the log knows the invite by.
    pub fn key(&self) -> [u8; 32] {
        self.signing().verifying_key().to_bytes()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![INVITE_VERSION];
        out.extend_from_slice(&self.team.0);
        out.extend_from_slice(&self.inviter.0);
        out.extend_from_slice(&self.expires.to_be_bytes());
        out.extend_from_slice(&self.secret);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let version = *bytes.first().ok_or(Error::Malformed("empty invite"))?;
        if version != INVITE_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        if bytes.len() != INVITE_LEN {
            return Err(Error::Malformed("invite length"));
        }
        let field = |from: usize| -> [u8; 32] { bytes[from..from + 32].try_into().expect("32") };
        Ok(Self {
            team: TeamId(field(1)),
            inviter: DeviceId(field(33)),
            expires: u64::from_be_bytes(bytes[65..73].try_into().expect("8")),
            secret: field(73),
        })
    }

    /// The request of `device` to be let in with this invite, for the
    /// inviter.
    pub fn request(&self, device: DeviceId) -> Vec<u8> {
        let mut out = signed(self.team, &self.key(), device);
        let signature = self.signing().sign(&[JOIN_CONTEXT, &out].concat());
        out.extend_from_slice(&signature.to_bytes());
        out
    }
}

/// A device's request to be let into a team, once its signature is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub team: TeamId,
    /// The public key of the invite it was made with.
    pub invite: [u8; 32],
    pub device: DeviceId,
    pub signature: [u8; 64],
}

/// Whether `signature` is the invite key's over this team and device.
pub fn asked(team: TeamId, invite: &[u8; 32], device: DeviceId, signature: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(invite).is_ok_and(|key| {
        key.verify_strict(
            &[JOIN_CONTEXT, &signed(team, invite, device)].concat(),
            &Signature::from_bytes(signature),
        )
        .is_ok()
    })
}

impl Request {
    pub fn open(bytes: &[u8]) -> Result<Self, Error> {
        let version = *bytes.first().ok_or(Error::Malformed("empty request"))?;
        if version != INVITE_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        if bytes.len() != REQUEST_LEN {
            return Err(Error::Malformed("request length"));
        }
        let field = |from: usize| -> [u8; 32] { bytes[from..from + 32].try_into().expect("32") };
        let request = Self {
            team: TeamId(field(1)),
            invite: field(33),
            device: DeviceId(field(65)),
            signature: bytes[BODY_LEN..].try_into().expect("64"),
        };
        if !asked(
            request.team,
            &request.invite,
            request.device,
            &request.signature,
        ) {
            return Err(Error::BadSignature);
        }
        Ok(request)
    }
}

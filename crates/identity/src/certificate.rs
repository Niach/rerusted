//! A device certificate: the user key's word that a device is the user's,
//! for a time (#RUST-17).
//!
//! ```text
//! certificate = body signature                              145 bytes
//! signature   = 64 bytes, Ed25519 by `user` over
//!               "rerusted/device-certificate/1" body
//!
//! body        = version user device not_before not_after    81 bytes
//! version     = 1 byte, CERTIFICATE_VERSION                 (signed, #RUST-29)
//! user        = 32 bytes, Ed25519 public key of the user
//! device      = 32 bytes, Ed25519 public key of the device
//! not_before  = u64, seconds since 1970-01-01 UTC
//! not_after   = u64, seconds since 1970-01-01 UTC, greater than not_before
//! ```
//!
//! Integers are big-endian. A decoder accepts exactly these bytes.
//!
//! The certificate is valid at `now` when `not_before <= now < not_after`.
//! The clock is the caller's: this crate reads none.
//!
//! A certificate says nothing about a team and is not taken back by a later
//! one. A device is revoked before its certificate runs out by an operation
//! in each team's log (decided 2026-11-14), which is not part of this module.

use ed25519_dalek::{Signature, Signer, VerifyingKey};
use teamlog::DeviceId;

use crate::Error;
use crate::user::{UserId, UserKey};

/// The version of the certificate layout, its first signed byte.
pub const CERTIFICATE_VERSION: u8 = 1;

/// The length of a certificate of [`CERTIFICATE_VERSION`].
pub const CERTIFICATE_LEN: usize = BODY_LEN + SIGNATURE_LEN;

const SIGNATURE_CONTEXT: &[u8] = b"rerusted/device-certificate/1";
const BODY_LEN: usize = 1 + 32 + 32 + 8 + 8;
const SIGNATURE_LEN: usize = 64;

/// What a certificate says, once its signature is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub user: UserId,
    pub device: DeviceId,
    /// The first second the certificate is valid in.
    pub not_before: u64,
    /// The first second it is no longer valid in.
    pub not_after: u64,
}

fn signed(body: &[u8]) -> Vec<u8> {
    [SIGNATURE_CONTEXT, body].concat()
}

impl UserKey {
    /// Signs that `device` is this user's from `not_before` until
    /// `not_after`. Refuses a window that holds no second.
    pub fn certify(
        &self,
        device: DeviceId,
        not_before: u64,
        not_after: u64,
    ) -> Result<Vec<u8>, Error> {
        if not_before >= not_after {
            return Err(Error::Malformed("certificate window"));
        }
        let mut out = vec![CERTIFICATE_VERSION];
        out.extend_from_slice(&self.id().0);
        out.extend_from_slice(&device.0);
        out.extend_from_slice(&not_before.to_be_bytes());
        out.extend_from_slice(&not_after.to_be_bytes());
        let signature = self.0.sign(&signed(&out));
        out.extend_from_slice(&signature.to_bytes());
        Ok(out)
    }
}

impl Certificate {
    /// Decodes a certificate and checks its signature. It does not look at
    /// the time: see [`Certificate::check`].
    pub fn open(bytes: &[u8]) -> Result<Self, Error> {
        let version = *bytes.first().ok_or(Error::Malformed("empty certificate"))?;
        if version != CERTIFICATE_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        if bytes.len() != CERTIFICATE_LEN {
            return Err(Error::Malformed("certificate length"));
        }
        let (body, signature) = bytes.split_at(BODY_LEN);
        let field = |from: usize| -> [u8; 32] { body[from..from + 32].try_into().expect("32") };
        let time = |from: usize| u64::from_be_bytes(body[from..from + 8].try_into().expect("8"));
        let certificate = Self {
            user: UserId(field(1)),
            device: DeviceId(field(33)),
            not_before: time(65),
            not_after: time(73),
        };
        if certificate.not_before >= certificate.not_after {
            return Err(Error::Malformed("certificate window"));
        }
        let key = VerifyingKey::from_bytes(&certificate.user.0).map_err(|_| Error::BadSignature)?;
        let signature: &[u8; SIGNATURE_LEN] = signature.try_into().expect("64");
        key.verify_strict(&signed(body), &Signature::from_bytes(signature))
            .map_err(|_| Error::BadSignature)?;
        Ok(certificate)
    }

    /// Whether the certificate is valid at `now`, in seconds since 1970.
    pub fn check(&self, now: u64) -> Result<(), Error> {
        if now < self.not_before {
            Err(Error::NotYetValid)
        } else if now >= self.not_after {
            Err(Error::Expired)
        } else {
            Ok(())
        }
    }

    /// Decodes, checks the signature and the time, and that the certificate
    /// is `user`'s for `device`.
    pub fn verify(bytes: &[u8], user: UserId, device: DeviceId, now: u64) -> Result<Self, Error> {
        let certificate = Self::open(bytes)?;
        if certificate.user != user || certificate.device != device {
            return Err(Error::Mismatch);
        }
        certificate.check(now)?;
        Ok(certificate)
    }
}

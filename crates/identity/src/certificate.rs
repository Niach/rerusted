//! A device certificate: a signature that says a device is a user's
//! (#RUST-17).
//!
//! The user's seed is on no device (decided 2026-11-17). The user key signs
//! the certificate of the first device, and of a device set up from the 24
//! words; every later device is vouched for by a device the user already
//! has. A device proves itself with the chain of certificates that leads
//! from the user key to it.
//!
//! ```text
//! certificate = body signature                              161 bytes
//! signature   = 64 bytes, Ed25519 by `signer` over
//!               "rerusted/device-certificate/1" body
//!
//! body        = version user device signer                  97 bytes
//! version     = 1 byte, CERTIFICATE_VERSION                 (signed, #RUST-29)
//! user        = 32 bytes, Ed25519 public key of the user
//! device      = 32 bytes, Ed25519 public key of the device
//! signer      = 32 bytes, Ed25519 public key: `user`, or a device of the
//!               user's; never `device` itself
//!
//! chain       = count certificate*                          1 + count * 161
//! count       = 1 byte, 1 to MAX_CHAIN
//! ```
//!
//! A decoder accepts exactly these bytes. In a chain the first certificate
//! is signed by the user key, each later one by the device of the one before
//! it, all name the same user, and no device comes twice.
//!
//! A certificate has no validity window and reads no clock: it never runs
//! out (decided 2026-11-17). A device is taken out by a revocation in each
//! team's log (decided 2026-11-14), which is not part of this module, and a
//! chain is not checked against revocations here. Devices that a revoked
//! device vouched for stay, unless they are revoked too.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use teamlog::DeviceId;

use crate::Error;
use crate::user::{UserId, UserKey};

/// The version of the certificate layout, its first signed byte.
pub const CERTIFICATE_VERSION: u8 = 1;

/// The length of a certificate of [`CERTIFICATE_VERSION`].
pub const CERTIFICATE_LEN: usize = BODY_LEN + SIGNATURE_LEN;

/// The most certificates a chain may hold: the user key's and fifteen
/// devices vouching one after the other.
pub const MAX_CHAIN: usize = 16;

const SIGNATURE_CONTEXT: &[u8] = b"rerusted/device-certificate/1";
const BODY_LEN: usize = 1 + 32 + 32 + 32;
const SIGNATURE_LEN: usize = 64;

/// What a certificate says, once its signature is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub user: UserId,
    pub device: DeviceId,
    /// The key that signed: the user's, or that of a device of the user's.
    pub signer: [u8; 32],
}

impl Certificate {
    /// Whether the user key itself signed, not a device.
    pub fn by_user(&self) -> bool {
        self.signer == self.user.0
    }
}

fn signed(body: &[u8]) -> Vec<u8> {
    [SIGNATURE_CONTEXT, body].concat()
}

fn sign(key: &SigningKey, user: UserId, device: DeviceId) -> Result<Vec<u8>, Error> {
    let signer = key.verifying_key().to_bytes();
    if device.0 == signer || device.0 == user.0 {
        return Err(Error::Malformed("certificate for its signer"));
    }
    let mut out = vec![CERTIFICATE_VERSION];
    out.extend_from_slice(&user.0);
    out.extend_from_slice(&device.0);
    out.extend_from_slice(&signer);
    let signature = key.sign(&signed(&out));
    out.extend_from_slice(&signature.to_bytes());
    Ok(out)
}

impl UserKey {
    /// Signs that `device` is this user's: for the first device and for one
    /// set up from the 24 words.
    pub fn certify(&self, device: DeviceId) -> Result<Vec<u8>, Error> {
        sign(&self.0, self.id(), device)
    }
}

/// Signs, with the key of a device of `user`'s, that `device` is `user`'s
/// too. The certificate is worth what the signing device's own chain is.
pub fn vouch(signer: &SigningKey, user: UserId, device: DeviceId) -> Result<Vec<u8>, Error> {
    sign(signer, user, device)
}

impl Certificate {
    /// Decodes one certificate and checks its signature. Whether the signer
    /// may sign for the user is the chain's to say: see [`verify`].
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
        let certificate = Self {
            user: UserId(field(1)),
            device: DeviceId(field(33)),
            signer: field(65),
        };
        if certificate.device.0 == certificate.signer || certificate.device.0 == certificate.user.0
        {
            return Err(Error::Malformed("certificate for its signer"));
        }
        let key = VerifyingKey::from_bytes(&certificate.signer).map_err(|_| Error::BadSignature)?;
        let signature: &[u8; SIGNATURE_LEN] = signature.try_into().expect("64");
        key.verify_strict(&signed(body), &Signature::from_bytes(signature))
            .map_err(|_| Error::BadSignature)?;
        Ok(certificate)
    }
}

/// Puts certificates into one chain, the user key's first.
pub fn chain(certificates: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    if certificates.is_empty() || certificates.len() > MAX_CHAIN {
        return Err(Error::Malformed("chain length"));
    }
    let mut out = vec![certificates.len() as u8];
    for certificate in certificates {
        if certificate.len() != CERTIFICATE_LEN {
            return Err(Error::Malformed("certificate length"));
        }
        out.extend_from_slice(certificate);
    }
    Ok(out)
}

/// Checks that `chain` leads from the key of `user` to `device`, and returns
/// its certificates in order.
pub fn verify(chain: &[u8], user: UserId, device: DeviceId) -> Result<Vec<Certificate>, Error> {
    let (count, rest) = chain.split_first().ok_or(Error::Malformed("empty chain"))?;
    let count = usize::from(*count);
    if count == 0 || count > MAX_CHAIN || rest.len() != count * CERTIFICATE_LEN {
        return Err(Error::Malformed("chain length"));
    }
    let mut certificates: Vec<Certificate> = Vec::with_capacity(count);
    for bytes in rest.as_chunks::<CERTIFICATE_LEN>().0 {
        let certificate = Certificate::open(bytes)?;
        let signer = certificates.last().map_or(user.0, |last| last.device.0);
        if certificate.user != user
            || certificate.signer != signer
            || certificates
                .iter()
                .any(|earlier| earlier.device == certificate.device)
        {
            return Err(Error::Mismatch);
        }
        certificates.push(certificate);
    }
    if certificates.last().map(|last| last.device) != Some(device) {
        return Err(Error::Mismatch);
    }
    Ok(certificates)
}

//! The bytes of an entry.
//!
//! ```text
//! entry      = body signature
//! signature  = 64 bytes, Ed25519 by `author` over "rerusted/entry/1" body
//! id         = SHA-256(entry)
//!
//! body       = version author seq prev kind fields
//! version    = 1 byte, FORMAT_VERSION                       (signed, #RUST-29)
//! author     = 32 bytes, Ed25519 public key
//! seq        = u64                                          position in the author's log
//! prev       = 32 bytes, id of the author's entry seq - 1; zero at seq 0
//! kind       = 1 byte
//!
//! 0 announce = sealing                                      32 bytes, X25519 public key
//! 1 create   = key boxes
//! 2 set      = team under parents member role cut flag [key] boxes
//! 3 remove   = team under parents member cut flag [key] boxes
//! 4 rotate   = team under parents key boxes
//! 5 payload  = team under key nonce text
//! 6 handover = team under parents member cut
//! 7 leave    = team under parents
//! 8 accept   = team under parents u32 length, entry       the handover, whole
//! 9 claim    = team under parents owner                     owner: 32 bytes, the forked owner
//!
//! team, under, member, key = 32 bytes each
//! parents    = u16 count, then (device, seq, id) of 32, 8 and 32 bytes,
//!              ascending by device, one per device, at least one
//! role       = 1 byte: 0 member, 1 admin
//! cut        = u64
//! flag       = 1 byte: 1 when a key follows, else 0
//! boxes      = u16 count, then (key, device, u16 length, sealed)
//! nonce      = 24 bytes
//! text       = u32 length, then XChaCha20-Poly1305 ciphertext with tag
//! ```
//!
//! Integers are big-endian. A decoder accepts exactly these bytes: nothing
//! may follow, and parents out of order are refused.
//!
//! The proof that a device forked is not an entry and belongs to no log:
//!
//! ```text
//! proof      = u32 length, entry, u32 length, entry
//! ```
//!
//! Two entries of one author for one position, each with its signature, the
//! one with the lower id first.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use teamlog::{DeviceId, EntryId, Role, TeamId};

use crate::Error;
use crate::rules::{KeyId, Parent};

/// The version of the body layout, the first signed byte of every entry.
pub const FORMAT_VERSION: u8 = 1;

const SIGNATURE_CONTEXT: &[u8] = b"rerusted/entry/1";
const SIGNATURE_LEN: usize = 64;

/// A team key sealed to one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lockbox {
    pub key: KeyId,
    pub device: DeviceId,
    pub sealed: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Announce {
        sealing: [u8; 32],
    },
    Create {
        key: KeyId,
        boxes: Vec<Lockbox>,
    },
    Set {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
        member: DeviceId,
        role: Role,
        cut: u64,
        key: Option<KeyId>,
        boxes: Vec<Lockbox>,
    },
    Remove {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
        member: DeviceId,
        cut: u64,
        key: Option<KeyId>,
        boxes: Vec<Lockbox>,
    },
    Rotate {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
        key: KeyId,
        boxes: Vec<Lockbox>,
    },
    Payload {
        team: TeamId,
        under: EntryId,
        key: KeyId,
        nonce: [u8; 24],
        text: Vec<u8>,
    },
    Handover {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
        member: DeviceId,
        cut: u64,
    },
    Leave {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
    },
    Accept {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
        /// The handover entry with its signature.
        handover: Vec<u8>,
    },
    Claim {
        team: TeamId,
        under: EntryId,
        parents: Vec<Parent>,
        /// The owner whose fork left the team without one.
        owner: DeviceId,
    },
}

/// The signed part of an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub author: DeviceId,
    pub seq: u64,
    pub prev: EntryId,
    pub kind: Kind,
}

fn put_boxes(out: &mut Vec<u8>, boxes: &[Lockbox]) {
    out.extend_from_slice(&(boxes.len() as u16).to_be_bytes());
    for lockbox in boxes {
        out.extend_from_slice(&lockbox.key.0);
        out.extend_from_slice(&lockbox.device.0);
        out.extend_from_slice(&(lockbox.sealed.len() as u16).to_be_bytes());
        out.extend_from_slice(&lockbox.sealed);
    }
}

fn put_parents(out: &mut Vec<u8>, parents: &[Parent]) {
    out.extend_from_slice(&(parents.len() as u16).to_be_bytes());
    for parent in parents {
        out.extend_from_slice(&parent.device.0);
        out.extend_from_slice(&parent.seq.to_be_bytes());
        out.extend_from_slice(&parent.id.0);
    }
}

fn put_key(out: &mut Vec<u8>, key: &Option<KeyId>) {
    match key {
        Some(key) => {
            out.push(1);
            out.extend_from_slice(&key.0);
        }
        None => out.push(0),
    }
}

impl Body {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![FORMAT_VERSION];
        out.extend_from_slice(&self.author.0);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.prev.0);
        match &self.kind {
            Kind::Announce { sealing } => {
                out.push(0);
                out.extend_from_slice(sealing);
            }
            Kind::Create { key, boxes } => {
                out.push(1);
                out.extend_from_slice(&key.0);
                put_boxes(&mut out, boxes);
            }
            Kind::Set {
                team,
                under,
                parents,
                member,
                role,
                cut,
                key,
                boxes,
            } => {
                out.push(2);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
                out.extend_from_slice(&member.0);
                out.push(*role as u8);
                out.extend_from_slice(&cut.to_be_bytes());
                put_key(&mut out, key);
                put_boxes(&mut out, boxes);
            }
            Kind::Remove {
                team,
                under,
                parents,
                member,
                cut,
                key,
                boxes,
            } => {
                out.push(3);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
                out.extend_from_slice(&member.0);
                out.extend_from_slice(&cut.to_be_bytes());
                put_key(&mut out, key);
                put_boxes(&mut out, boxes);
            }
            Kind::Rotate {
                team,
                under,
                parents,
                key,
                boxes,
            } => {
                out.push(4);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
                out.extend_from_slice(&key.0);
                put_boxes(&mut out, boxes);
            }
            Kind::Payload {
                team,
                under,
                key,
                nonce,
                text,
            } => {
                out.push(5);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                out.extend_from_slice(&key.0);
                out.extend_from_slice(nonce);
                out.extend_from_slice(&(text.len() as u32).to_be_bytes());
                out.extend_from_slice(text);
            }
            Kind::Handover {
                team,
                under,
                parents,
                member,
                cut,
            } => {
                out.push(6);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
                out.extend_from_slice(&member.0);
                out.extend_from_slice(&cut.to_be_bytes());
            }
            Kind::Leave {
                team,
                under,
                parents,
            } => {
                out.push(7);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
            }
            Kind::Accept {
                team,
                under,
                parents,
                handover,
            } => {
                out.push(8);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
                out.extend_from_slice(&(handover.len() as u32).to_be_bytes());
                out.extend_from_slice(handover);
            }
            Kind::Claim {
                team,
                under,
                parents,
                owner,
            } => {
                out.push(9);
                out.extend_from_slice(&team.0);
                out.extend_from_slice(&under.0);
                put_parents(&mut out, parents);
                out.extend_from_slice(&owner.0);
            }
        }
        out
    }
}

/// Reads fields off the front of a slice.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let (head, tail) = self
            .0
            .split_at_checked(len)
            .ok_or(Error::Malformed("too short"))?;
        self.0 = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        Ok(self.take(N)?.try_into().expect("take returns N bytes"))
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<usize, Error> {
        Ok(u16::from_be_bytes(self.array()?) as usize)
    }

    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn parents(&mut self) -> Result<Vec<Parent>, Error> {
        let count = self.u16()?;
        let mut parents = Vec::new();
        for _ in 0..count {
            parents.push(Parent {
                device: DeviceId(self.array()?),
                seq: self.u64()?,
                id: EntryId(self.array()?),
            });
        }
        if parents.is_empty() {
            return Err(Error::Malformed("no parents"));
        }
        if !parents.is_sorted_by(|a, b| a.device < b.device) {
            return Err(Error::Malformed("parents out of order"));
        }
        Ok(parents)
    }

    fn key(&mut self) -> Result<Option<KeyId>, Error> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(KeyId(self.array()?))),
            _ => Err(Error::Malformed("key flag")),
        }
    }

    fn boxes(&mut self) -> Result<Vec<Lockbox>, Error> {
        let count = self.u16()?;
        let mut boxes = Vec::new();
        for _ in 0..count {
            let key = KeyId(self.array()?);
            let device = DeviceId(self.array()?);
            let len = self.u16()?;
            boxes.push(Lockbox {
                key,
                device,
                sealed: self.take(len)?.to_vec(),
            });
        }
        Ok(boxes)
    }
}

fn decode_body(bytes: &[u8]) -> Result<Body, Error> {
    let mut r = Reader(bytes);
    match r.u8()? {
        FORMAT_VERSION => {}
        other => return Err(Error::UnsupportedVersion(other)),
    }
    let author = DeviceId(r.array()?);
    let seq = r.u64()?;
    let prev = EntryId(r.array()?);
    let kind = match r.u8()? {
        0 => Kind::Announce {
            sealing: r.array()?,
        },
        1 => Kind::Create {
            key: KeyId(r.array()?),
            boxes: r.boxes()?,
        },
        2 => Kind::Set {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
            member: DeviceId(r.array()?),
            role: match r.u8()? {
                0 => Role::Member,
                1 => Role::Admin,
                _ => return Err(Error::Malformed("role")),
            },
            cut: r.u64()?,
            key: r.key()?,
            boxes: r.boxes()?,
        },
        3 => Kind::Remove {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
            member: DeviceId(r.array()?),
            cut: r.u64()?,
            key: r.key()?,
            boxes: r.boxes()?,
        },
        4 => Kind::Rotate {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
            key: KeyId(r.array()?),
            boxes: r.boxes()?,
        },
        5 => Kind::Payload {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            key: KeyId(r.array()?),
            nonce: r.array()?,
            text: {
                let len = u32::from_be_bytes(r.array()?) as usize;
                r.take(len)?.to_vec()
            },
        },
        6 => Kind::Handover {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
            member: DeviceId(r.array()?),
            cut: r.u64()?,
        },
        7 => Kind::Leave {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
        },
        8 => Kind::Accept {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
            handover: {
                let len = u32::from_be_bytes(r.array()?) as usize;
                r.take(len)?.to_vec()
            },
        },
        9 => Kind::Claim {
            team: TeamId(r.array()?),
            under: EntryId(r.array()?),
            parents: r.parents()?,
            owner: DeviceId(r.array()?),
        },
        _ => return Err(Error::Malformed("kind")),
    };
    if !r.0.is_empty() {
        return Err(Error::Malformed("trailing bytes"));
    }
    Ok(Body {
        author,
        seq,
        prev,
        kind,
    })
}

fn signed(body: &[u8]) -> Vec<u8> {
    [SIGNATURE_CONTEXT, body].concat()
}

pub fn id(entry: &[u8]) -> EntryId {
    EntryId(Sha256::digest(entry).into())
}

/// Encodes and signs. The key has to be the author's.
pub fn seal(body: &Body, key: &SigningKey) -> Vec<u8> {
    let mut bytes = body.encode();
    let signature = key.sign(&signed(&bytes));
    bytes.extend_from_slice(&signature.to_bytes());
    bytes
}

/// Decodes an entry that was verified before.
pub fn read(entry: &[u8]) -> Result<Body, Error> {
    let (bytes, _) = entry
        .split_last_chunk::<SIGNATURE_LEN>()
        .ok_or(Error::Malformed("too short"))?;
    decode_body(bytes)
}

/// Decodes an entry and verifies its signature.
pub fn open(entry: &[u8]) -> Result<Body, Error> {
    if let Some(version) = entry.first()
        && *version != FORMAT_VERSION
    {
        return Err(Error::UnsupportedVersion(*version));
    }
    let (bytes, signature) = entry
        .split_last_chunk::<SIGNATURE_LEN>()
        .ok_or(Error::Malformed("too short"))?;
    let body = decode_body(bytes)?;
    let key =
        VerifyingKey::from_bytes(&body.author.0).map_err(|_| Error::Malformed("author key"))?;
    key.verify_strict(&signed(bytes), &Signature::from_bytes(signature))
        .map_err(|_| Error::BadSignature)?;
    Ok(body)
}

/// The proof that the author of `a` and `b` forked: two different entries
/// of one device for one position.
pub fn proof(a: &[u8], b: &[u8]) -> Vec<u8> {
    let (first, second) = if id(a) < id(b) { (a, b) } else { (b, a) };
    let mut out = Vec::new();
    for entry in [first, second] {
        out.extend_from_slice(&(entry.len() as u32).to_be_bytes());
        out.extend_from_slice(entry);
    }
    out
}

/// Verifies a proof and says which device forked at which position.
pub fn open_proof(proof: &[u8]) -> Result<(DeviceId, u64), Error> {
    let mut r = Reader(proof);
    let mut entries = Vec::new();
    for _ in 0..2 {
        let len = u32::from_be_bytes(r.array()?) as usize;
        let entry = r.take(len)?;
        entries.push((id(entry), open(entry)?));
    }
    if !r.0.is_empty() {
        return Err(Error::Malformed("trailing bytes"));
    }
    let (first, second) = (&entries[0], &entries[1]);
    if first.0 >= second.0 {
        return Err(Error::Malformed("proof out of order"));
    }
    if (first.1.author, first.1.seq) != (second.1.author, second.1.seq) {
        return Err(Error::Malformed("proof of nothing"));
    }
    Ok((first.1.author, first.1.seq))
}

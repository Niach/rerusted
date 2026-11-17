//! The signed log on its own: what `ingest` refuses, and the bytes of an
//! entry.

mod common;

use common::{Node, now};
use identity::format::{self, Body, Kind};
use identity::{Error, Replica, keys};
use teamlog::{DeviceId, ENTRY_VERSION, EntryId, TeamLog};

#[test]
fn refuses_what_is_not_a_valid_next_entry() {
    let mut a = Node::new("a");
    let mut b = Replica::generate();
    let (_, create) = now(a.log.create_team()).unwrap();
    let first = now(a.log.entries_after(a.id(), 0)).unwrap()[0].clone();
    let second = create[0].clone();
    assert_eq!((first.seq, second.seq), (0, 1));
    assert_eq!(first.version, ENTRY_VERSION);
    assert_eq!(first.version, format::FORMAT_VERSION);

    // A version this replica does not write, outside and inside the
    // signature.
    let mut newer = first.clone();
    newer.version += 1;
    assert_eq!(now(b.ingest(newer)), Err(Error::UnsupportedVersion(2)));
    let mut newer = first.clone();
    newer.bytes[0] += 1;
    assert_eq!(now(b.ingest(newer)), Err(Error::UnsupportedVersion(2)));

    // One flipped bit anywhere.
    for position in 1..first.bytes.len() {
        let mut tampered = first.clone();
        tampered.bytes[position] ^= 1;
        assert!(
            matches!(
                now(b.ingest(tampered)),
                Err(Error::BadSignature | Error::Malformed(_))
            ),
            "{position}"
        );
    }
    // Bytes after the entry.
    let mut longer = first.clone();
    longer.bytes.push(0);
    assert!(now(b.ingest(longer)).is_err());

    // Outer fields that are not the entry's.
    let mut other = first.clone();
    other.seq = 1;
    assert_eq!(now(b.ingest(other)), Err(Error::Mismatch));
    let mut other = first.clone();
    other.id = EntryId([0; 32]);
    assert_eq!(now(b.ingest(other)), Err(Error::Mismatch));
    let mut other = first.clone();
    other.author = DeviceId([0; 32]);
    assert_eq!(now(b.ingest(other)), Err(Error::Mismatch));

    // The second entry before the first.
    assert_eq!(
        now(b.ingest(second.clone())),
        Err(Error::Gap { have: 0, got: 1 })
    );
    assert!(
        now(b.heights())
            .unwrap()
            .values()
            .all(|height| *height == 0)
    );

    // In order they are taken, and taking them again changes nothing.
    now(b.ingest(first.clone())).unwrap();
    now(b.ingest(second.clone())).unwrap();
    assert_eq!(now(b.ingest(first.clone())), Ok((Vec::new(), Vec::new())));
    assert_eq!(now(b.heights()).unwrap()[&a.id()], 2);

    // A second entry for a position that is taken.
    let key = keys::signing_key(&a.log.seed());
    let forked = Body {
        author: a.id(),
        seq: 1,
        prev: second.id,
        kind: Kind::Announce { sealing: [1; 32] },
    };
    let mut entry = second.clone();
    entry.bytes = format::seal(&forked, &key);
    entry.id = format::id(&entry.bytes);
    // That is a fork: the log is cut back to it and nothing more is taken.
    now(b.ingest(entry)).unwrap();
    assert_eq!(now(b.forks()).unwrap()[&a.id()], 1);
    assert_eq!(now(b.heights()).unwrap()[&a.id()], 1);

    // An entry that does not name the one before it.
    let mut c = Replica::generate();
    now(c.ingest(first)).unwrap();
    let broken = Body {
        author: a.id(),
        seq: 1,
        prev: EntryId([9; 32]),
        kind: Kind::Announce { sealing: [1; 32] },
    };
    let mut entry = second;
    entry.bytes = format::seal(&broken, &key);
    entry.id = format::id(&entry.bytes);
    assert_eq!(now(c.ingest(entry)), Err(Error::Broken));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Test vector for format version 1: the first entry of the device with
/// the seed 00 01 02 … 1f, which announces its sealing key. Ed25519
/// signatures are deterministic, so are these bytes.
#[test]
fn announce_vector() {
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let mut replica = Replica::from_seed(seed);
    let entry = now(replica.announce()).unwrap().remove(0);
    assert_eq!(hex(&entry.author.0), DEVICE);
    assert_eq!(hex(&entry.bytes), ENTRY);
    assert_eq!(hex(&entry.id.0), ID);
    assert_eq!(entry.bytes.len(), 1 + 32 + 8 + 32 + 1 + 32 + 64);
}

const DEVICE: &str = "01c08ad63554bafdca11353a3f4552cce15b05e033361589594423cb7d9295ad";
const ENTRY: &str = "\
    01\
    01c08ad63554bafdca11353a3f4552cce15b05e033361589594423cb7d9295ad\
    0000000000000000\
    0000000000000000000000000000000000000000000000000000000000000000\
    00\
    211d9204d3648bfea59bf4f205946b8e84aebad27c0518b4e01a386723ad213f\
    4e18b9ba11d31d93f706c6fbdba10c3d48a3ce65576048954437ad19d762bc59\
    15ca1af168d7b948cfeccc8e172c6c0768fcc0f681b0a47e67df21e85103b60f";
const ID: &str = "26957736c4f3aa02c5edb76f75f87854945e2543aac1716bd115606ed4f50881";

/// Test vector for kind 9, the claim: the second entry of the same device.
/// The fields are fixed bytes, not a real team. Computed outside this crate
/// (Python: hashlib, cryptography).
#[test]
fn claim_vector() {
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let mut replica = Replica::from_seed(seed);
    let announce = now(replica.announce()).unwrap().remove(0);
    let body = Body {
        author: announce.author,
        seq: 1,
        prev: announce.id,
        kind: Kind::Claim {
            team: teamlog::TeamId([1; 32]),
            under: EntryId([2; 32]),
            parents: vec![identity::rules::Parent {
                device: DeviceId([3; 32]),
                seq: 4,
                id: EntryId([5; 32]),
            }],
            owner: DeviceId([6; 32]),
        },
    };
    let bytes = format::seal(&body, &keys::signing_key(&seed));
    assert_eq!(hex(&bytes), CLAIM);
    assert_eq!(hex(&format::id(&bytes).0), CLAIM_ID);
    assert_eq!(format::open(&bytes), Ok(body));
}

const CLAIM: &str = "\
    01\
    01c08ad63554bafdca11353a3f4552cce15b05e033361589594423cb7d9295ad\
    0000000000000001\
    26957736c4f3aa02c5edb76f75f87854945e2543aac1716bd115606ed4f50881\
    09\
    0101010101010101010101010101010101010101010101010101010101010101\
    0202020202020202020202020202020202020202020202020202020202020202\
    0001\
    0303030303030303030303030303030303030303030303030303030303030303\
    0000000000000004\
    0505050505050505050505050505050505050505050505050505050505050505\
    0606060606060606060606060606060606060606060606060606060606060606\
    88eedc66e930aa2a5f331ded72fc66aea2b5649bc24e58db2d1ddbbf6577cbb6\
    8c721e236512c792c223f25591608e876ebb27f0bebaaf057992b49eb9a5ca04";
const CLAIM_ID: &str = "c3b9e04fceeef5fc269bcb758baf643e1d8187155bebb339f56033e548df4b3f";

/// Test vectors for the entries among one user's devices: kinds 10, 11
/// and 12.
#[test]
fn device_entry_vectors() {
    use identity::format::Lockbox;
    use identity::rules::{KeyId, Parent};
    use teamlog::{Role, TeamId};

    let unhex = |text: &str| -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    };
    let certificates = include_str!("vectors/certificate-v1.txt");
    let chain = unhex(certificates.lines().last().unwrap().trim());
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let mut replica = Replica::from_seed(seed);
    let announce = now(replica.announce()).unwrap().remove(0);
    let (team, under) = (TeamId([1; 32]), EntryId([2; 32]));
    let parents = vec![Parent {
        device: DeviceId([3; 32]),
        seq: 4,
        id: EntryId([5; 32]),
    }];
    let boxes = vec![Lockbox {
        key: KeyId([8; 32]),
        device: DeviceId([6; 32]),
        sealed: vec![0xaa, 0xbb, 0xcc],
    }];
    let kinds = [
        ("certify", Kind::Certify { chain }),
        (
            "admit",
            Kind::Admit {
                team,
                under,
                parents: parents.clone(),
                member: DeviceId([6; 32]),
                role: Role::Admin,
                cut: 7,
                key: Some(KeyId([8; 32])),
                boxes: boxes.clone(),
            },
        ),
        (
            "revoke",
            Kind::Revoke {
                team,
                under,
                parents,
                member: DeviceId([6; 32]),
                cut: 7,
                key: Some(KeyId([8; 32])),
                boxes,
            },
        ),
    ];
    let vectors: Vec<Vec<&str>> = include_str!("vectors/devices-v1.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.split('|').map(str::trim).collect())
        .collect();
    assert_eq!(vectors.len(), kinds.len());
    for ((name, kind), vector) in kinds.into_iter().zip(vectors) {
        assert_eq!(name, vector[0]);
        let body = Body {
            author: announce.author,
            seq: 1,
            prev: announce.id,
            kind,
        };
        let bytes = format::seal(&body, &keys::signing_key(&seed));
        assert_eq!(hex(&bytes), vector[1], "{name}");
        assert_eq!(hex(&format::id(&bytes).0), vector[2], "{name}");
        assert_eq!(format::open(&bytes), Ok(body), "{name}");
    }
}

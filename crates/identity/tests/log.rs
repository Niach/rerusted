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

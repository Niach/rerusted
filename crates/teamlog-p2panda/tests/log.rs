//! The signed log on its own: what `ingest` refuses, and what survives a
//! restart. No network; entries are handed over directly.

use teamlog::{ENTRY_VERSION, Entry, Role, TeamLog};
use teamlog_p2panda::{Error, Replica};

/// See `scenario.rs`: p2panda's futures need more than a test thread's stack.
fn run(body: impl Future<Output = ()> + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(body);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Hands entries over and returns what the receiver appended in reaction.
async fn give(to: &mut Replica, entries: &[Entry]) -> Vec<Entry> {
    let mut reactions = Vec::new();
    for entry in entries {
        reactions.extend(to.ingest(entry.clone()).await.unwrap().1);
    }
    reactions
}

#[test]
fn refuses_what_is_not_a_valid_next_entry() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Replica::open(&dir.path().join("a.sqlite")).await.unwrap();
        let mut b = Replica::open(&dir.path().join("b.sqlite")).await.unwrap();

        let announce = a.announce().await.unwrap();
        let (_, create) = a.create_team().await.unwrap();
        let first = announce[0].clone();
        let second = create[0].clone();
        assert_eq!((first.seq, second.seq), (0, 1));
        assert_eq!(first.version, ENTRY_VERSION);

        // A version this replica does not write.
        let mut newer = first.clone();
        newer.version = ENTRY_VERSION + 1;
        assert!(matches!(
            b.ingest(newer).await,
            Err(Error::UnsupportedVersion(2))
        ));

        // One flipped bit, in the signed header and in the body.
        for position in [10, first.bytes.len() - 1] {
            let mut tampered = first.clone();
            tampered.bytes[position] ^= 1;
            assert!(
                matches!(b.ingest(tampered).await, Err(Error::Invalid(_))),
                "{position}"
            );
        }

        // The second entry before the first.
        assert!(matches!(
            b.ingest(second.clone()).await,
            Err(Error::Gap {
                seq: 1,
                held: 0,
                ..
            })
        ));
        assert!(b.heights().await.unwrap().is_empty());

        // The outer fields may say anything: what is stored is what the
        // signed bytes say.
        let mut lying = first.clone();
        lying.author = b.device();
        lying.seq = 7;
        b.ingest(lying).await.unwrap();
        assert_eq!(
            b.entries_after(a.device(), 0).await.unwrap(),
            std::slice::from_ref(&first)
        );

        // Twice is once.
        b.ingest(first.clone()).await.unwrap();
        give(&mut b, &create).await;
        give(&mut b, &create).await;
        assert_eq!(b.heights().await.unwrap(), a.heights().await.unwrap());
    });
}

#[test]
fn refuses_a_fork() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("a.sqlite");
        let copy = dir.path().join("a-copy.sqlite");

        // The same device, restored from a copy of its file, continues its
        // log in two different ways.
        let mut a = Replica::open(&original).await.unwrap();
        let announce = a.announce().await.unwrap();
        drop(a);
        // SQLite keeps recent writes in side files next to the database.
        for suffix in ["", "-wal", "-shm"] {
            let from = dir.path().join(format!("a.sqlite{suffix}"));
            if from.exists() {
                std::fs::copy(from, dir.path().join(format!("a-copy.sqlite{suffix}"))).unwrap();
            }
        }
        let mut a = Replica::open(&original).await.unwrap();
        let mut twin = Replica::open(&copy).await.unwrap();
        assert_eq!(a.device(), twin.device());
        let (_, one) = a.create_team().await.unwrap();
        let (_, other) = twin.create_team().await.unwrap();
        assert_eq!(one[0].seq, other[0].seq);
        assert_ne!(one[0].id, other[0].id);

        let mut b = Replica::open(&dir.path().join("b.sqlite")).await.unwrap();
        give(&mut b, &announce).await;
        give(&mut b, &one).await;
        assert!(matches!(
            b.ingest(other[0].clone()).await,
            Err(Error::Fork { seq: 1, .. })
        ));
    });
}

#[test]
fn survives_a_restart_and_carries_large_payloads() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Replica::open(&dir.path().join("a.sqlite")).await.unwrap();
        let mut b = Replica::open(&dir.path().join("b.sqlite")).await.unwrap();
        let mut entries = a.announce().await.unwrap();
        let from_b = b.announce().await.unwrap();
        give(&mut a, &from_b).await;

        let (team, created) = a.create_team().await.unwrap();
        entries.extend(created);
        entries.extend(a.add_member(team, b.device(), Role::Member).await.unwrap());
        // Far beyond the 512 bytes a p2panda header may carry per field.
        let large = vec![7u8; 1 << 16];
        entries.extend(a.write(team, &large).await.unwrap());
        // Enough removals that a welcome carries a dozen keys.
        for _ in 0..12 {
            entries.extend(a.remove_member(team, b.device()).await.unwrap());
            entries.extend(a.add_member(team, b.device(), Role::Member).await.unwrap());
        }
        entries.extend(a.write(team, b"last").await.unwrap());

        // b takes half, restarts, takes the rest.
        let (early, late) = entries.split_at(entries.len() / 2);
        let mut reactions = give(&mut b, early).await;
        let device = b.device();
        drop(b);
        let mut b = Replica::open(&dir.path().join("b.sqlite")).await.unwrap();
        assert_eq!(b.device(), device);
        reactions.extend(give(&mut b, late).await);

        // Each time b was added again it appended one entry of its own, a
        // pointer p2panda calls a repair. a needs them to hold the same logs.
        assert_eq!(reactions.len(), 12);
        assert!(reactions.iter().all(|entry| entry.author == device));
        assert!(give(&mut a, &reactions).await.is_empty());

        assert_eq!(b.heights().await.unwrap(), a.heights().await.unwrap());
        assert_eq!(
            b.members(team).await.unwrap(),
            a.members(team).await.unwrap()
        );
        let history = b.history(team).await.unwrap();
        assert_eq!(history, a.history(team).await.unwrap());
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].data, large);
        assert!(b.failed().await.unwrap().is_empty());
        assert!(b.waiting().await.unwrap().is_empty());
    });
}

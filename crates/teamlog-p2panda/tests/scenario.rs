//! The #RUST-30 scenario: full devices that only ever talk over iroh.
//!
//! Each node is a `Replica` with its own SQLite file and its own iroh
//! endpoint on localhost, relays off. Nothing reaches another node except
//! through `sync`, which runs the exchange below over one QUIC stream.
//! The test bodies use the `teamlog` trait only; `Replica` appears where a
//! node is opened and for the two diagnostics `failed` and `waiting`.
//!
//! The assertions state what p2panda 0.8.0 does, also where that is not what
//! #RUST-3 asks for. Those places are marked FINDING and are in the report.

use std::collections::BTreeMap;
use std::path::PathBuf;

use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, RelayMode, SecretKey};
use teamlog::{DeviceId, Entry, EntryId, Event, Role, TeamId, TeamLog};
use teamlog_p2panda::Replica;

const ALPN: &[u8] = b"teamlog/spike/1";

struct Node {
    name: &'static str,
    path: PathBuf,
    log: Replica,
    key: SecretKey,
    endpoint: Option<Endpoint>,
    events: Vec<Event>,
}

impl Node {
    async fn start(dir: &std::path::Path, name: &'static str) -> Self {
        let path = dir.join(format!("{name}.sqlite"));
        let log = Replica::open(&path).await.unwrap();
        let mut node = Self {
            name,
            path,
            log,
            key: SecretKey::generate(),
            endpoint: None,
            events: Vec::new(),
        };
        node.online().await;
        node
    }

    async fn online(&mut self) {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(self.key.clone())
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        self.endpoint = Some(endpoint);
    }

    async fn offline(&mut self) {
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.close().await;
        }
    }

    /// Drops the replica and opens it again from its file.
    async fn restart(&mut self) {
        let dir = self.path.parent().unwrap().to_owned();
        let scratch = Replica::open(&dir.join(format!("{}-scratch.sqlite", self.name)))
            .await
            .unwrap();
        drop(std::mem::replace(&mut self.log, scratch));
        self.log = Replica::open(&self.path).await.unwrap();
    }

    fn id(&self) -> DeviceId {
        self.log.device()
    }

    async fn texts(&self, team: TeamId) -> Vec<String> {
        let mut texts: Vec<_> = self
            .log
            .history(team)
            .await
            .unwrap()
            .into_iter()
            .map(|payload| String::from_utf8(payload.data).unwrap())
            .collect();
        texts.sort();
        texts
    }
}

// ── the exchange ────────────────────────────────────────────────────────────
//
// Both sides run the same rounds until a round moves nothing:
//   send my heights, read theirs, send what they lack, read and ingest.

async fn send_frame(send: &mut SendStream, bytes: &[u8]) {
    send.write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    send.write_all(bytes).await.unwrap();
}

async fn recv_frame(recv: &mut RecvStream) -> Vec<u8> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.unwrap();
    let mut bytes = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut bytes).await.unwrap();
    bytes
}

fn encode_heights(heights: &BTreeMap<DeviceId, u64>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (device, height) in heights {
        bytes.extend_from_slice(&device.0);
        bytes.extend_from_slice(&height.to_be_bytes());
    }
    bytes
}

fn decode_heights(bytes: &[u8]) -> BTreeMap<DeviceId, u64> {
    bytes
        .as_chunks::<40>()
        .0
        .iter()
        .map(|chunk| {
            (
                DeviceId(chunk[..32].try_into().unwrap()),
                u64::from_be_bytes(chunk[32..].try_into().unwrap()),
            )
        })
        .collect()
}

fn encode_entry(entry: &Entry) -> Vec<u8> {
    let mut bytes = vec![entry.version];
    bytes.extend_from_slice(&entry.author.0);
    bytes.extend_from_slice(&entry.seq.to_be_bytes());
    bytes.extend_from_slice(&entry.id.0);
    bytes.extend_from_slice(&entry.bytes);
    bytes
}

fn decode_entry(bytes: &[u8]) -> Entry {
    Entry {
        version: bytes[0],
        author: DeviceId(bytes[1..33].try_into().unwrap()),
        seq: u64::from_be_bytes(bytes[33..41].try_into().unwrap()),
        id: EntryId(bytes[41..73].try_into().unwrap()),
        bytes: bytes[73..].to_vec(),
    }
}

async fn exchange(node: &mut Node, connection: Connection, dialer: bool) {
    let (mut send, mut recv) = if dialer {
        connection.open_bi().await.unwrap()
    } else {
        connection.accept_bi().await.unwrap()
    };
    loop {
        let mine = node.log.heights().await.unwrap();
        send_frame(&mut send, &encode_heights(&mine)).await;
        let theirs = decode_heights(&recv_frame(&mut recv).await);

        let mut outgoing = Vec::new();
        for (device, height) in &mine {
            let from = theirs.get(device).copied().unwrap_or(0);
            if from < *height {
                outgoing.extend(node.log.entries_after(*device, from).await.unwrap());
            }
        }
        send_frame(&mut send, &(outgoing.len() as u32).to_be_bytes()).await;
        for entry in &outgoing {
            send_frame(&mut send, &encode_entry(entry)).await;
        }

        let incoming = u32::from_be_bytes(recv_frame(&mut recv).await.try_into().unwrap());
        for _ in 0..incoming {
            let entry = decode_entry(&recv_frame(&mut recv).await);
            let (events, _reaction) = node.log.ingest(entry).await.unwrap();
            node.events.extend(events);
        }

        if outgoing.is_empty() && incoming == 0 {
            break;
        }
    }
    // Closing drops what is still in flight. The listener ends its stream
    // only after it has read everything, so the dialer waits for that end.
    send.finish().unwrap();
    if dialer {
        assert!(recv.read_to_end(0).await.unwrap().is_empty());
        connection.close(0u32.into(), b"done");
    } else {
        connection.closed().await;
    }
}

/// One sync between two nodes that are both online.
async fn sync(a: &mut Node, b: &mut Node) {
    let a_endpoint = a.endpoint.clone().expect("dialer is online");
    let b_endpoint = b.endpoint.clone().expect("listener is online");
    let address = b_endpoint.addr();
    let dial = async {
        let connection = a_endpoint.connect(address, ALPN).await.unwrap();
        exchange(a, connection, true).await;
    };
    let listen = async {
        let connection = b_endpoint.accept().await.unwrap().await.unwrap();
        exchange(b, connection, false).await;
    };
    tokio::join!(dial, listen);
}

/// Syncs every pair among the nodes that are online, until nothing moves.
async fn settle(nodes: &mut [&mut Node]) {
    loop {
        let mut before = Vec::new();
        for node in nodes.iter() {
            before.push(node.log.heights().await.unwrap());
        }
        for i in 0..nodes.len() {
            for j in i + 1..nodes.len() {
                let (left, right) = nodes.split_at_mut(j);
                if left[i].endpoint.is_some() && right[0].endpoint.is_some() {
                    sync(left[i], right[0]).await;
                }
            }
        }
        let mut after = Vec::new();
        for node in nodes.iter() {
            after.push(node.log.heights().await.unwrap());
        }
        if before == after {
            return;
        }
    }
}

/// The member list as names, in a fixed order.
async fn members(node: &Node, team: TeamId, all: &[&Node]) -> Vec<(&'static str, Role)> {
    let mut members: Vec<_> = node
        .log
        .members(team)
        .await
        .unwrap()
        .into_iter()
        .map(|(device, role)| {
            let name = all
                .iter()
                .find(|node| node.id() == device)
                .expect("known device")
                .name;
            (name, role)
        })
        .collect();
    members.sort();
    members
}

/// Entries the replica holds but p2panda refused, as `author#seq`.
async fn failed(node: &Node, all: &[&Node]) -> Vec<String> {
    let failed = node.log.failed().await.unwrap();
    failed
        .iter()
        .map(|(entry, reason)| {
            assert!(reason.contains("unknown group secret"), "{reason}");
            let name = all
                .iter()
                .find(|node| node.id() == entry.author)
                .unwrap()
                .name;
            format!("{name}#{}", entry.seq)
        })
        .collect()
}

async fn nothing_waits(nodes: &[&Node]) {
    for node in nodes {
        assert!(
            node.log.waiting().await.unwrap().is_empty(),
            "{} waits",
            node.name
        );
    }
}

/// Runs a test body on a thread with a large stack. p2panda's futures hold
/// whole group states by value; in a debug build they overflow the 2 MB
/// stack of a test thread.
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

use Role::{Admin, Member};

/// Create a team, add members, exchange, remove a member who keeps writing
/// offline, add the member again, rejoin. Concurrent admin operations twice.
#[test]
fn remove_offline_writes_readd_rejoin() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Node::start(dir.path(), "a").await;
        let mut b = Node::start(dir.path(), "b").await;
        let mut c = Node::start(dir.path(), "c").await;
        let mut d = Node::start(dir.path(), "d").await;

        // Every device publishes what is needed to add it, once.
        for node in [&mut a, &mut b, &mut c, &mut d] {
            assert_eq!(node.log.announce().await.unwrap().len(), 1);
            assert!(node.log.announce().await.unwrap().is_empty());
        }
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;

        // ── 1: a team of three, everybody writes, everybody reads ──────────
        let (team, _) = a.log.create_team().await.unwrap();
        a.log.add_member(team, b.id(), Admin).await.unwrap();
        a.log.add_member(team, c.id(), Member).await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        a.log.write(team, b"a1").await.unwrap();
        b.log.write(team, b"b1").await.unwrap();
        c.log.write(team, b"c1").await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        for node in [&a, &b, &c, &d] {
            let all = [&a, &b, &c, &d];
            assert_eq!(
                members(node, team, &all).await,
                [("a", Admin), ("b", Admin), ("c", Member)]
            );
        }
        for node in [&a, &b, &c] {
            assert_eq!(node.texts(team).await, ["a1", "b1", "c1"]);
        }
        // d holds every entry but is no member: it sees who is in the team
        // (the member log is public to whoever holds it) and reads nothing.
        assert!(d.texts(team).await.is_empty());

        // ── 2: concurrent admin operations on two devices ──────────────────
        // a promotes c while b adds d; neither has seen the other's entry.
        a.log.set_role(team, c.id(), Admin).await.unwrap();
        b.log.add_member(team, d.id(), Member).await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        d.log.write(team, b"d1").await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        for node in [&a, &b, &c, &d] {
            let all = [&a, &b, &c, &d];
            assert_eq!(
                members(node, team, &all).await,
                [("a", Admin), ("b", Admin), ("c", Admin), ("d", Member)]
            );
            // d was added after a1, b1 and c1 were written and reads them.
            assert_eq!(node.texts(team).await, ["a1", "b1", "c1", "d1"]);
        }

        // ── 3: b is removed while offline and keeps writing ────────────────
        b.offline().await;
        a.log.remove_member(team, b.id()).await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        a.log.write(team, b"a2-after-removal").await.unwrap();
        c.log.write(team, b"c2-after-removal").await.unwrap();
        // b does not know. It writes, and as an admin it removes d: a second
        // pair of concurrent admin operations, this time in conflict.
        b.log.write(team, b"b2-offline").await.unwrap();
        b.log.remove_member(team, d.id()).await.unwrap();
        b.log.write(team, b"b3-offline").await.unwrap();
        b.restart().await;
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        {
            let all = [&a, &b, &c, &d];
            assert_eq!(
                members(&b, team, &all).await,
                [("a", Admin), ("b", Admin), ("c", Admin)]
            );
            assert_eq!(
                b.texts(team).await,
                ["a1", "b1", "b2-offline", "b3-offline", "c1", "d1"]
            );
        }

        // ── 4: b reconnects ────────────────────────────────────────────────
        b.online().await;
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        {
            let all = [&a, &b, &c, &d];
            // The removal of b wins over b's concurrent removal of d, on
            // every replica, b's included.
            for node in all {
                assert_eq!(
                    members(node, team, &all).await,
                    [("a", Admin), ("c", Admin), ("d", Member)],
                    "{}",
                    node.name
                );
            }
            nothing_waits(&all).await;
            assert!(b.events.contains(&Event::Ejected { team }));

            // The removed member holds the ciphertext of everything written
            // after its removal and cannot read it: the key was rotated and
            // not sealed to it.
            assert_eq!(
                b.texts(team).await,
                ["a1", "b1", "b2-offline", "b3-offline", "c1", "d1"]
            );
            assert_eq!(failed(&b, &all).await.len(), 2);
            // FINDING: what b wrote offline is lost for everybody else. a and
            // c hold the key b used and accept the entries, but p2panda emits
            // no payload for an author who has been removed meanwhile.
            for node in [&a, &c, &d] {
                assert_eq!(
                    node.texts(team).await,
                    [
                        "a1",
                        "a2-after-removal",
                        "b1",
                        "c1",
                        "c2-after-removal",
                        "d1"
                    ]
                );
            }
            // d cannot even decrypt b3: b rotated the key when it "removed"
            // d. The entry stays on d as refused, for good.
            assert_eq!(failed(&d, &all).await.len(), 1);
            assert!(failed(&a, &all).await.is_empty());
            assert!(failed(&c, &all).await.is_empty());
        }

        assert!(matches!(
            b.log.write(team, b"b-ejected").await,
            Err(teamlog_p2panda::Error::NotMember(_))
        ));

        // ── 5: b is added again and rejoins ────────────────────────────────
        a.log.add_member(team, b.id(), Member).await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
        a.log.write(team, b"a3-after-readd").await.unwrap();
        b.log.write(team, b"b4-after-readd").await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;

        // ── 6: what every replica ends up with ─────────────────────────────
        let all = [&a, &b, &c, &d];
        nothing_waits(&all).await;
        for node in all {
            assert_eq!(
                members(node, team, &all).await,
                [("a", Admin), ("b", Member), ("c", Admin), ("d", Member)],
                "{}",
                node.name
            );
            assert_eq!(
                node.log.heights().await.unwrap(),
                a.log.heights().await.unwrap()
            );
            assert_eq!(node.log.forged_payloads(), 0);
        }
        // a and c read everything written by members.
        let history = [
            "a1",
            "a2-after-removal",
            "a3-after-readd",
            "b1",
            "b4-after-readd",
            "c1",
            "c2-after-removal",
            "d1",
        ];
        for node in [&a, &c] {
            assert_eq!(node.texts(team).await, history, "{}", node.name);
            assert!(failed(node, &all).await.is_empty());
        }
        // FINDING: the readable history does not always converge. b's removal
        // of d was overruled, but the key b made for it can end up as the
        // team's current key. Then everybody writes under a key that d, a
        // member all along, was never given: d reads nothing written after b
        // came back, until somebody is removed again. Whether it happens
        // differs from run to run (the keys and with them the hashes that
        // order concurrent operations are random); both outcomes are
        // accepted here and the test prints which one it saw.
        let d_reads = d.texts(team).await;
        if d_reads == history {
            println!("OUTCOME d reads everything");
            assert_eq!(failed(&d, &all).await.len(), 1);
        } else {
            println!("OUTCOME d is locked out");
            assert_eq!(
                d_reads,
                [
                    "a1",
                    "a2-after-removal",
                    "b1",
                    "c1",
                    "c2-after-removal",
                    "d1"
                ]
            );
            assert_eq!(failed(&d, &all).await.len(), 3);
        }
        // FINDING: the welcome of the re-add carries every earlier team key,
        // so b now also reads what was written while it was removed. Its own
        // offline writes remain readable to b alone.
        assert_eq!(
            b.texts(team).await,
            [
                "a1",
                "a2-after-removal",
                "a3-after-readd",
                "b1",
                "b2-offline",
                "b3-offline",
                "b4-after-readd",
                "c1",
                "c2-after-removal",
                "d1",
            ]
        );
        assert!(failed(&b, &all).await.is_empty());
    });
}

/// Two admins remove each other at the same time.
#[test]
fn mutual_removal() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Node::start(dir.path(), "a").await;
        let mut b = Node::start(dir.path(), "b").await;
        let mut c = Node::start(dir.path(), "c").await;
        for node in [&mut a, &mut b, &mut c] {
            node.log.announce().await.unwrap();
        }
        settle(&mut [&mut a, &mut b, &mut c]).await;
        let (team, _) = a.log.create_team().await.unwrap();
        a.log.add_member(team, b.id(), Admin).await.unwrap();
        a.log.add_member(team, c.id(), Member).await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c]).await;

        a.log.remove_member(team, b.id()).await.unwrap();
        b.log.remove_member(team, a.id()).await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c]).await;

        // FINDING: both removals stand. Every replica agrees, and the team
        // is left without an admin: nobody can ever add or remove again.
        // #RUST-3 proposed "the owner wins, then seniority"; p2panda has no
        // owner, and the spaces manager offers no other rule.
        let all = [&a, &b, &c];
        for node in all {
            assert_eq!(
                members(node, team, &all).await,
                [("c", Member)],
                "{}",
                node.name
            );
        }
        assert!(c.log.add_member(team, b.id(), Member).await.is_err());
        assert!(a.log.add_member(team, b.id(), Member).await.is_err());

        // FINDING: c got one new key from each removal and writes under one
        // of them. The admin who made that key was removed by the other
        // removal and still reads what c writes now. Nobody rotates the key
        // after the two removals were merged, and p2panda 0.8 has no working
        // call to do it (`SpaceUpdate` is `unimplemented!()`).
        c.log.write(team, b"c-after-both-removals").await.unwrap();
        settle(&mut [&mut a, &mut b, &mut c]).await;
        assert_eq!(c.texts(team).await, ["c-after-both-removals"]);
        let a_reads = !a.texts(team).await.is_empty();
        let b_reads = !b.texts(team).await.is_empty();
        println!("OUTCOME removed admin reading on: a {a_reads}, b {b_reads}");
        assert!(a_reads ^ b_reads, "exactly one removed admin reads it");
    });
}

/// The device that created the team has no special standing.
#[test]
fn creator_can_be_removed() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Node::start(dir.path(), "a").await;
        let mut b = Node::start(dir.path(), "b").await;
        for node in [&mut a, &mut b] {
            node.log.announce().await.unwrap();
        }
        settle(&mut [&mut a, &mut b]).await;
        let (team, _) = a.log.create_team().await.unwrap();
        a.log.add_member(team, b.id(), Admin).await.unwrap();
        settle(&mut [&mut a, &mut b]).await;

        b.log.remove_member(team, a.id()).await.unwrap();
        settle(&mut [&mut a, &mut b]).await;
        let all = [&a, &b];
        for node in all {
            assert_eq!(members(node, team, &all).await, [("b", Admin)]);
        }
    });
}

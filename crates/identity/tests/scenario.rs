//! The #RUST-30 scenario on our own log: full devices that only ever talk
//! over iroh.
//!
//! Each node is a `Replica` with its own iroh endpoint on localhost, relays
//! off. Nothing reaches another node except through `sync`, which runs the
//! exchange below over one QUIC stream. The steps are those of the p2panda
//! spike (`teamlog-p2panda/tests/scenario.rs`); where that test marked a
//! FINDING, this one asserts the behaviour decided on 2026-11-14.

use std::collections::BTreeMap;

use identity::{Error, Replica};
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, RelayMode, SecretKey};
use teamlog::{DeviceId, Entry, EntryId, Event, Role, TeamId, TeamLog};

const ALPN: &[u8] = b"teamlog/spike/2";

struct Node {
    name: &'static str,
    log: Replica,
    key: SecretKey,
    endpoint: Option<Endpoint>,
    events: Vec<Event>,
}

impl Node {
    async fn start(name: &'static str) -> Self {
        let mut log = Replica::generate();
        // Every device publishes what is needed to add it, once.
        assert_eq!(log.announce().await.unwrap().len(), 1);
        assert!(log.announce().await.unwrap().is_empty());
        let mut node = Self {
            name,
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

    /// Drops the replica and builds it again from its seed and its entries,
    /// which is all a store would have to keep.
    fn restart(&mut self) {
        self.log = Replica::restore(self.log.seed(), self.log.entries()).unwrap();
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

use Role::{Admin, Member, Owner};

/// Create a team, add members, exchange, remove a member who keeps writing
/// offline, add the member again, rejoin. Concurrent admin operations twice.
#[tokio::test]
async fn remove_offline_writes_readd_rejoin() {
    let mut a = Node::start("a").await;
    let mut b = Node::start("b").await;
    let mut c = Node::start("c").await;
    let mut d = Node::start("d").await;
    settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;

    // ── 1: a team of three, everybody writes, everybody reads ──────────────
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
            [("a", Owner), ("b", Admin), ("c", Member)]
        );
    }
    for node in [&a, &b, &c] {
        assert_eq!(node.texts(team).await, ["a1", "b1", "c1"]);
    }
    // d holds every entry but is no member: it sees who is in the team (the
    // member log is public to whoever holds it) and reads nothing.
    assert!(d.texts(team).await.is_empty());

    // ── 2: concurrent admin operations on two devices ──────────────────────
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
            [("a", Owner), ("b", Admin), ("c", Admin), ("d", Member)]
        );
        // d is new to the team and reads what was written before it came.
        assert_eq!(node.texts(team).await, ["a1", "b1", "c1", "d1"]);
    }

    // ── 3: b is removed while offline and keeps writing ────────────────────
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
    b.restart();
    settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
    {
        let all = [&a, &b, &c, &d];
        assert_eq!(
            members(&b, team, &all).await,
            [("a", Owner), ("b", Admin), ("c", Admin)]
        );
        assert_eq!(
            b.texts(team).await,
            ["a1", "b1", "b2-offline", "b3-offline", "c1", "d1"]
        );
    }

    // ── 4: b reconnects ────────────────────────────────────────────────────
    b.online().await;
    settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
    {
        let all = [&a, &b, &c, &d];
        // The removal of b stands and b's removal of d is void, on every
        // replica, b's included.
        for node in all {
            assert_eq!(
                members(node, team, &all).await,
                [("a", Owner), ("c", Admin), ("d", Member)],
                "{}",
                node.name
            );
            // What b wrote after the removal had been written is void for
            // everybody (decided: drop). The entries stay in b's log.
            assert_eq!(node.log.void(team).len(), 2, "{}", node.name);
            assert_eq!(
                node.log.team(team).unwrap().current_key(),
                a.log.team(team).unwrap().current_key()
            );
        }
        assert!(b.events.contains(&Event::Ejected { team }));
        // b is told to take back what it had shown as its own.
        let dropped = b
            .events
            .iter()
            .filter(|event| matches!(event, Event::Dropped { .. }));
        assert_eq!(dropped.count(), 2);

        // The removed member holds the ciphertext of everything written
        // after its removal and cannot read it: the key was rotated and not
        // sealed to it.
        assert_eq!(b.texts(team).await, ["a1", "b1", "c1", "d1"]);
        // d was a member all along and reads everything. The key b made
        // when it "removed" d is nobody's current key. (p2panda: d locked
        // out in 4 of 12 runs.)
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
                ],
                "{}",
                node.name
            );
        }
    }
    assert_eq!(b.log.write(team, b"b-ejected").await, Err(Error::NotMember));

    // ── 5: b is added again and rejoins ────────────────────────────────────
    a.log.add_member(team, b.id(), Member).await.unwrap();
    settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;
    a.log.write(team, b"a3-after-readd").await.unwrap();
    b.log.write(team, b"b4-after-readd").await.unwrap();
    settle(&mut [&mut a, &mut b, &mut c, &mut d]).await;

    // ── 6: what every replica ends up with ─────────────────────────────────
    let all = [&a, &b, &c, &d];
    for node in all {
        assert_eq!(
            members(node, team, &all).await,
            [("a", Owner), ("b", Member), ("c", Admin), ("d", Member)],
            "{}",
            node.name
        );
        assert_eq!(
            node.log.heights().await.unwrap(),
            a.log.heights().await.unwrap()
        );
        assert_eq!(node.log.void(team).len(), 2);
    }
    // a, c and d read everything written by members.
    for node in [&a, &c, &d] {
        assert_eq!(
            node.texts(team).await,
            [
                "a1",
                "a2-after-removal",
                "a3-after-readd",
                "b1",
                "b4-after-readd",
                "c1",
                "c2-after-removal",
                "d1",
            ],
            "{}",
            node.name
        );
    }
    // b reads what it could before and what is written since it is back,
    // not what was written while it was out (decided: no). Its offline
    // writes stay void.
    assert_eq!(
        b.texts(team).await,
        ["a1", "a3-after-readd", "b1", "b4-after-readd", "c1", "d1"]
    );
}

/// Two admins try to remove each other, and one tries the owner.
#[tokio::test]
async fn admins_remove_neither_each_other_nor_the_owner() {
    let mut a = Node::start("a").await;
    let mut b = Node::start("b").await;
    let mut c = Node::start("c").await;
    settle(&mut [&mut a, &mut b, &mut c]).await;
    let (team, _) = a.log.create_team().await.unwrap();
    a.log.add_member(team, b.id(), Admin).await.unwrap();
    a.log.add_member(team, c.id(), Admin).await.unwrap();
    settle(&mut [&mut a, &mut b, &mut c]).await;

    // p2panda: both removals stood and the team had no admin for good. Here
    // neither can be written. What happens when a device writes them anyway
    // is in `cases.rs`.
    let refused = Err(Error::NotAllowed);
    assert_eq!(b.log.remove_member(team, c.id()).await, refused);
    assert_eq!(c.log.remove_member(team, b.id()).await, refused);
    assert_eq!(b.log.remove_member(team, a.id()).await, refused);
    assert_eq!(b.log.set_role(team, a.id(), Member).await, refused);

    // The owner removes an admin, who at the same time adds nobody new but
    // writes; the team goes on with an owner and an admin.
    a.log.remove_member(team, b.id()).await.unwrap();
    settle(&mut [&mut a, &mut b, &mut c]).await;
    c.log.write(team, b"c-after").await.unwrap();
    settle(&mut [&mut a, &mut b, &mut c]).await;
    let all = [&a, &b, &c];
    for node in all {
        assert_eq!(
            members(node, team, &all).await,
            [("a", Owner), ("c", Admin)],
            "{}",
            node.name
        );
    }
    assert_eq!(a.texts(team).await, ["c-after"]);
    assert_eq!(c.texts(team).await, ["c-after"]);
    assert!(b.texts(team).await.is_empty());
}

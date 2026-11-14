//! Random runs: devices that follow the rules and devices that do not, in
//! random order, with random partial syncs. Whatever happened, replicas
//! that hold the same entries must agree, in whatever order they got them.

mod common;

use std::collections::BTreeMap;

use common::{Node, now, settle, sync};
use identity::Replica;
use identity::format::Kind;
use identity::rules::KeyId;
use teamlog::Role::{Admin, Member};
use teamlog::{DeviceId, Entry, EntryId, Event, Payload, TeamId, TeamLog};

/// xorshift64*: the runs have to be the same every time.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const NAMES: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

/// One thing a device does. Honest calls go through the rules and may be
/// refused; forged entries are written whatever the rules say.
fn act(random: &mut Random, team: TeamId, nodes: &mut [Node], step: usize) {
    let i = random.below(nodes.len());
    let j = random.below(nodes.len());
    let other = nodes[j].id();
    let role = if random.below(3) == 0 { Admin } else { Member };
    let node = &mut nodes[i];
    match random.below(12) {
        0..3 => {
            let _ = now(node
                .log
                .write(team, format!("{}{step}", node.name).as_bytes()));
        }
        3 | 4 => {
            let _ = now(node.log.add_member(team, other, role));
        }
        5 | 6 => {
            let _ = now(node.log.remove_member(team, other));
        }
        7 => {
            let _ = now(node.log.set_role(team, other, role));
        }
        8..11 => {
            let (left, right) = nodes.split_at_mut(i.max(j));
            if i != j {
                sync(&mut left[i.min(j)], &mut right[0]);
            }
        }
        // A device that ignores the rules: it removes, admits or promotes
        // whoever, under whatever operation it can name.
        _ => {
            let Some(resolved) = node.log.team(team) else {
                return;
            };
            let under = [
                resolved.admin_grant(node.id()),
                resolved.member_grant(node.id()),
                Some(EntryId(team.0)),
            ][random.below(3)]
            .unwrap_or(EntryId(team.0));
            let kind = match random.below(3) {
                0 => node.removal(team, other, under),
                1 => node.setting(team, other, role, under),
                _ => Kind::Rotate {
                    team,
                    under,
                    parents: resolved.heads().to_vec(),
                    key: KeyId([step as u8; 32]),
                    boxes: Vec::new(),
                },
            };
            node.forge(kind);
        }
    }
}

/// The entries in another order: each device's log stays in order, the logs
/// are interleaved at random.
fn shuffled(random: &mut Random, entries: Vec<Entry>) -> Vec<Entry> {
    let mut logs: BTreeMap<DeviceId, Vec<Entry>> = BTreeMap::new();
    for entry in entries.into_iter().rev() {
        logs.entry(entry.author).or_default().push(entry);
    }
    let mut logs: Vec<_> = logs.into_values().collect();
    let mut out = Vec::new();
    while !logs.is_empty() {
        let i = random.below(logs.len());
        out.push(logs[i].pop().expect("empty logs are removed"));
        if logs[i].is_empty() {
            logs.swap_remove(i);
        }
    }
    out
}

fn run(seed: u64) {
    let mut random = Random(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut nodes = NAMES.map(Node::new);
    {
        let [a, b, c, d, e, f] = &mut nodes;
        settle(&mut [a, b, c, d, e, f]);
    }
    let (team, _) = now(nodes[0].log.create_team()).unwrap();
    let (first, rest) = nodes.split_at_mut(1);
    now(first[0].log.add_member(team, rest[0].id(), Admin)).unwrap();
    now(first[0].log.add_member(team, rest[1].id(), Admin)).unwrap();
    now(first[0].log.add_member(team, rest[2].id(), Member)).unwrap();
    for step in 0..60 {
        act(&mut random, team, &mut nodes, step);
    }
    {
        let [a, b, c, d, e, f] = &mut nodes;
        settle(&mut [a, b, c, d, e, f]);
    }

    // 1. Everybody agrees on the members, the key and what is void.
    let members = now(nodes[0].log.members(team)).unwrap();
    let key = nodes[0].log.team(team).unwrap().current_key();
    let void = nodes[0].log.void(team);
    assert!(members.contains(&(nodes[0].id(), teamlog::Role::Owner)));
    for node in &nodes {
        assert_eq!(now(node.log.members(team)).unwrap(), members, "{seed}");
        assert_eq!(node.log.team(team).unwrap().current_key(), key, "{seed}");
        assert_eq!(node.log.void(team), void, "{seed}");
    }

    // 2. A replica that gets the same entries in another order, in one go,
    //    ends up with the same, and its events add up to its history.
    for node in &nodes {
        let mut other = Replica::from_seed(node.log.seed());
        let mut told: BTreeMap<(DeviceId, u64), Vec<u8>> = BTreeMap::new();
        for entry in shuffled(&mut random, node.log.entries()) {
            for event in now(other.ingest(entry)).unwrap().0 {
                match event {
                    Event::Payload { payload, .. } => {
                        assert!(
                            told.insert((payload.author, payload.n), payload.data)
                                .is_none()
                        );
                    }
                    Event::Dropped { payload, .. } => {
                        assert!(told.remove(&(payload.author, payload.n)).is_some());
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(now(other.members(team)).unwrap(), members, "{seed}");
        assert_eq!(other.team(team).unwrap().current_key(), key, "{seed}");
        assert_eq!(other.void(team), void, "{seed}");
        let history = now(other.history(team)).unwrap();
        assert_eq!(history, now(node.log.history(team)).unwrap(), "{seed}");
        let told: Vec<_> = told
            .into_iter()
            .map(|((author, n), data)| Payload { author, n, data })
            .collect();
        assert_eq!(told, history, "{seed}");
    }

    // 3. What members write now, every member reads and nobody else does,
    //    though everybody holds the entries.
    let is_member = |node: &Node| members.iter().any(|(device, _)| *device == node.id());
    let mut written = Vec::new();
    for node in nodes.iter_mut().filter(|node| is_member(node)) {
        let text = format!("final {}", node.name);
        node.write(team, &text);
        written.push(text);
    }
    {
        let [a, b, c, d, e, f] = &mut nodes;
        settle(&mut [a, b, c, d, e, f]);
    }
    let key = nodes[0].log.team(team).unwrap().current_key();
    assert!(key.is_some(), "{seed}");
    for node in &nodes {
        assert_eq!(now(node.log.members(team)).unwrap(), members, "{seed}");
        assert_eq!(node.log.team(team).unwrap().current_key(), key, "{seed}");
        let read: Vec<_> = node
            .texts(team)
            .into_iter()
            .filter(|text| text.starts_with("final "))
            .collect();
        if is_member(node) {
            assert_eq!(read, written, "{seed} {}", node.name);
        } else {
            assert!(read.is_empty(), "{seed} {}", node.name);
        }
    }
}

#[test]
fn replicas_agree_whatever_happened() {
    // More with `SEEDS=5000 cargo test --release -p identity --test converge`.
    let seeds = std::env::var("SEEDS").map_or(40, |seeds| seeds.parse().unwrap());
    for seed in 0..seeds {
        run(seed);
    }
}

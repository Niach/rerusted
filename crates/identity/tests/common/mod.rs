//! Replicas that hand entries to each other directly, no network.

#![allow(dead_code, reason = "each test file uses its own part")]

use std::pin::pin;
use std::task::{Context, Poll, Waker};

use identity::format::{self, Body, FORMAT_VERSION, Kind};
use identity::{Replica, keys};
use teamlog::{DeviceId, Entry, EntryId, Event, Role, TeamId, TeamLog};

/// The value of a future that is ready at once, as all of `Replica`'s are.
pub fn now<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a replica's future is always ready"),
    }
}

pub struct Node {
    pub name: &'static str,
    pub log: Replica,
    pub events: Vec<Event>,
}

impl Node {
    /// A new device that has announced itself.
    pub fn new(name: &'static str) -> Self {
        let mut log = Replica::generate();
        now(log.announce()).unwrap();
        Self {
            name,
            log,
            events: Vec::new(),
        }
    }

    pub fn id(&self) -> DeviceId {
        self.log.device()
    }

    pub fn texts(&self, team: TeamId) -> Vec<String> {
        let mut texts: Vec<_> = now(self.log.history(team))
            .unwrap()
            .into_iter()
            .map(|payload| String::from_utf8(payload.data).unwrap())
            .collect();
        texts.sort();
        texts
    }

    /// The member list as names, in a fixed order.
    pub fn roles(&self, team: TeamId, all: &[&Node]) -> Vec<(&'static str, Role)> {
        let mut roles: Vec<_> = now(self.log.members(team))
            .unwrap()
            .into_iter()
            .map(|(device, role)| {
                let node = all.iter().find(|node| node.id() == device);
                (node.expect("known device").name, role)
            })
            .collect();
        roles.sort();
        roles
    }

    pub fn write(&mut self, team: TeamId, text: &str) -> Vec<Entry> {
        now(self.log.write(team, text.as_bytes())).unwrap()
    }

    pub fn take(&mut self, entry: Entry) {
        let (events, reaction) = now(self.log.ingest(entry)).unwrap();
        assert!(reaction.is_empty());
        self.events.extend(events);
    }

    /// Signs `kind` as this device's next entry without asking the rules,
    /// as a client that does not follow them would, and applies it here.
    pub fn forge(&mut self, kind: Kind) -> Entry {
        let entry = self.sign(kind);
        self.take(entry.clone());
        entry
    }

    /// Signs `kind` as this device's next entry and keeps it out of the log.
    pub fn sign(&self, kind: Kind) -> Entry {
        let height = now(self.log.heights()).unwrap()[&self.id()];
        let last = now(self.log.entries_after(self.id(), height - 1)).unwrap();
        let body = Body {
            author: self.id(),
            seq: height,
            prev: last[0].id,
            kind,
        };
        let bytes = format::seal(&body, &keys::signing_key(&self.log.seed()));
        Entry {
            version: FORMAT_VERSION,
            author: body.author,
            seq: body.seq,
            id: format::id(&bytes),
            bytes,
        }
    }

    pub fn height_of(&self, device: DeviceId) -> u64 {
        now(self.log.heights())
            .unwrap()
            .get(&device)
            .copied()
            .unwrap_or(0)
    }

    /// A removal as an admin would write it now, whether or not this device
    /// may.
    pub fn removal(&self, team: TeamId, member: DeviceId, under: EntryId) -> Kind {
        Kind::Remove {
            team,
            under,
            parents: self.log.team(team).unwrap().parents().to_vec(),
            member,
            cut: self.height_of(member),
            key: None,
            boxes: Vec::new(),
        }
    }

    /// An admission or change of role, likewise.
    pub fn setting(&self, team: TeamId, member: DeviceId, role: Role, under: EntryId) -> Kind {
        Kind::Set {
            team,
            under,
            parents: self.log.team(team).unwrap().parents().to_vec(),
            member,
            role,
            cut: self.height_of(member),
            key: None,
            boxes: Vec::new(),
        }
    }
}

/// What a store would keep of a replica: its seed, entries and proofs.
pub struct Backup {
    seed: [u8; 32],
    entries: Vec<Entry>,
    proofs: Vec<Vec<u8>>,
}

impl Node {
    pub fn backup(&self) -> Backup {
        Backup {
            seed: self.log.seed(),
            entries: self.log.entries(),
            proofs: self.log.proofs(),
        }
    }

    /// Puts the device back to a backup, as a restored disk would. What it
    /// wrote since is forgotten here and lives on wherever it was synced
    /// to: the next entry forks the device's log.
    pub fn restore(&mut self, backup: &Backup) {
        self.log =
            Replica::restore(backup.seed, backup.entries.clone(), backup.proofs.clone()).unwrap();
    }
}

/// Gives `to` every proof and entry `from` holds and `to` lacks. Where the
/// two hold different logs of a device, `to` gets the whole of `from`'s and
/// finds where they part.
fn give(from: &Node, to: &mut Node) {
    let known = now(to.log.forks()).unwrap();
    for (device, at) in now(from.log.forks()).unwrap() {
        if known.get(&device).is_none_or(|known| *known > at) {
            let proof = now(from.log.fork_proof(device)).unwrap().unwrap();
            let events = now(to.log.ingest_proof(&proof)).unwrap();
            to.events.extend(events);
        }
    }
    let theirs = now(to.log.tips()).unwrap();
    for (device, (height, _)) in now(from.log.tips()).unwrap() {
        let start = match theirs.get(&device) {
            None => 0,
            Some((have, _)) if *have > height => continue,
            Some((have, tip)) => {
                let mine = now(from.log.entries_after(device, have - 1)).unwrap();
                if mine[0].id == *tip { *have } else { 0 }
            }
        };
        for entry in now(from.log.entries_after(device, start)).unwrap() {
            to.take(entry);
        }
    }
}

pub fn sync(a: &mut Node, b: &mut Node) {
    give(a, b);
    give(b, a);
}

/// Syncs every pair until nothing moves.
pub fn settle(nodes: &mut [&mut Node]) {
    loop {
        let state = |node: &&mut Node| {
            (
                now(node.log.tips()).unwrap(),
                now(node.log.forks()).unwrap(),
            )
        };
        let before: Vec<_> = nodes.iter().map(state).collect();
        for i in 0..nodes.len() {
            for j in i + 1..nodes.len() {
                let (left, right) = nodes.split_at_mut(j);
                sync(left[i], right[0]);
            }
        }
        let after: Vec<_> = nodes.iter().map(state).collect();
        if before == after {
            return;
        }
    }
}

/// Makes the nodes the devices of one new user: the user key certifies the
/// first, each later one is vouched for by the one before it. Every device
/// publishes its chain.
pub fn one_user(nodes: &mut [&mut Node]) -> identity::user::UserId {
    use identity::certificate;
    let key = identity::user::Seed::generate().user_key();
    let mut certificates: Vec<Vec<u8>> = Vec::new();
    for at in 0..nodes.len() {
        let certificate = match at {
            0 => key.certify(nodes[0].id()),
            _ => certificate::vouch(
                &keys::signing_key(&nodes[at - 1].log.seed()),
                key.id(),
                nodes[at].id(),
            ),
        };
        certificates.push(certificate.unwrap());
        let chain = certificate::chain(&certificates).unwrap();
        nodes[at].log.certify(&chain).unwrap();
    }
    key.id()
}

impl Node {
    /// A revocation as a device of the same user would write it now,
    /// whether or not this device may.
    pub fn revocation(&self, team: TeamId, member: DeviceId, under: EntryId) -> Kind {
        Kind::Revoke {
            team,
            under,
            parents: self.log.team(team).unwrap().parents().to_vec(),
            member,
            cut: self.height_of(member),
            key: None,
            boxes: Vec::new(),
        }
    }

    /// An admission of one's own device, likewise.
    pub fn admission(&self, team: TeamId, member: DeviceId, role: Role, under: EntryId) -> Kind {
        Kind::Admit {
            team,
            under,
            parents: self.log.team(team).unwrap().parents().to_vec(),
            member,
            role,
            cut: self.height_of(member),
            key: None,
            boxes: Vec::new(),
        }
    }
}

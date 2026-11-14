//! A replica: the logs of every device it has heard of, held in memory, and
//! what they resolve to.
//!
//! Everything but the device's seed is rebuilt from the entries and the
//! proofs of forks: member lists, team keys (from the lockboxes addressed to
//! this device) and the readable history. So a replica is restored from its
//! seed, its entries and its proofs.
//!
//! The log of a device that forked is held up to the fork and no further.
//! Replicas that held different branches then hold the same again.

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use teamlog::{DeviceId, Entry, EntryId, Event, Payload, Role, TeamId, TeamLog};

use crate::Error;
use crate::format::{self, Body, FORMAT_VERSION, Kind, Lockbox};
use crate::keys::{self, Sealing, TeamKey};
use crate::rules::{self, Action, Handed, KeyId, Op, Team};

/// A device that signed two entries for one position.
struct Fork {
    /// The lowest such position known.
    at: u64,
    proof: Vec<u8>,
}

/// One payload entry of a team, readable or not.
struct Written {
    author: DeviceId,
    seq: u64,
    /// The author's count of payloads to this team.
    n: u64,
    under: EntryId,
    key: KeyId,
    nonce: [u8; 24],
    text: Vec<u8>,
}

#[derive(Default)]
struct TeamState {
    ops: Vec<Op>,
    keys: BTreeMap<KeyId, TeamKey>,
    written: Vec<Written>,
    resolved: Option<Team>,
    /// What was last reported through events.
    members: BTreeMap<DeviceId, Role>,
    history: BTreeMap<(DeviceId, u64), Vec<u8>>,
}

impl TeamState {
    /// The payload if its author was a member where it stands and this
    /// replica holds its key.
    fn read(&self, team: TeamId, written: &Written) -> Option<Vec<u8>> {
        let resolved = self.resolved.as_ref()?;
        if !resolved.writes(written.author, written.seq, written.under) {
            return None;
        }
        keys::decrypt(
            self.keys.get(&written.key)?,
            team,
            written.author,
            written.seq,
            &written.nonce,
            &written.text,
        )
    }
}

/// Reads the handover an acceptance by `acceptor` carries. It has to be a
/// signed handover of this team to the acceptor.
fn handed(team: TeamId, acceptor: DeviceId, handover: &[u8]) -> Result<Handed, Error> {
    let body = format::open(handover)?;
    match body.kind {
        Kind::Handover {
            team: of,
            under,
            member,
            ..
        } if of == team && member == acceptor => Ok(Handed {
            id: format::id(handover),
            author: body.author,
            seq: body.seq,
            under,
            member,
        }),
        _ => Err(Error::Malformed("acceptance without its handover")),
    }
}

/// What an entry added to a team.
enum Change {
    Nothing,
    Op(TeamId),
    Payload(TeamId),
}

pub struct Replica {
    seed: [u8; 32],
    signing: SigningKey,
    device: DeviceId,
    sealing: Sealing,
    logs: BTreeMap<DeviceId, Vec<Entry>>,
    forks: BTreeMap<DeviceId, Fork>,
    /// The handovers that acceptances carry, by their place in the old
    /// owner's log. An entry that takes such a place has to be that one.
    anchors: BTreeMap<(DeviceId, u64), (EntryId, Vec<u8>)>,
    /// The sealing key each device announced.
    announced: BTreeMap<DeviceId, [u8; 32]>,
    teams: BTreeMap<TeamId, TeamState>,
}

impl Replica {
    /// A new device with a random seed.
    pub fn generate() -> Self {
        Self::from_seed(keys::new_seed())
    }

    /// The device of this seed, holding no entries.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let signing = keys::signing_key(&seed);
        Self {
            seed,
            device: DeviceId(signing.verifying_key().to_bytes()),
            signing,
            sealing: Sealing::from_seed(&seed),
            logs: BTreeMap::new(),
            forks: BTreeMap::new(),
            anchors: BTreeMap::new(),
            announced: BTreeMap::new(),
            teams: BTreeMap::new(),
        }
    }

    /// The device of this seed with the entries and proofs it held, each
    /// device's entries in the order of its log. Everything is verified
    /// again.
    pub fn restore(
        seed: [u8; 32],
        entries: impl IntoIterator<Item = Entry>,
        proofs: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<Self, Error> {
        let mut replica = Self::from_seed(seed);
        for proof in proofs {
            replica.learn(&proof)?;
        }
        for entry in entries {
            replica.take(entry)?;
        }
        Ok(replica)
    }

    /// All this device needs kept secret.
    pub fn seed(&self) -> [u8; 32] {
        self.seed
    }

    /// Every entry held, each device's in the order of its log.
    pub fn entries(&self) -> Vec<Entry> {
        self.logs.values().flatten().cloned().collect()
    }

    /// The proof of every fork known.
    pub fn proofs(&self) -> Vec<Vec<u8>> {
        self.forks.values().map(|fork| fork.proof.clone()).collect()
    }

    /// The team as the entries held resolve it.
    pub fn team(&self, team: TeamId) -> Option<&Team> {
        self.teams.get(&team)?.resolved.as_ref()
    }

    /// The payloads of the team that do not count: their author was no
    /// member where they stand in its log. As (author, position), sorted.
    pub fn void(&self, team: TeamId) -> Vec<(DeviceId, u64)> {
        let Some(state) = self.teams.get(&team) else {
            return Vec::new();
        };
        let mut void: Vec<_> = state
            .written
            .iter()
            .filter(|written| {
                !state.resolved.as_ref().is_some_and(|resolved| {
                    resolved.writes(written.author, written.seq, written.under)
                })
            })
            .map(|written| (written.author, written.seq))
            .collect();
        void.sort();
        void
    }

    fn height(&self, device: DeviceId) -> u64 {
        self.logs.get(&device).map_or(0, |log| log.len() as u64)
    }

    fn resolved(&self, team: TeamId) -> Result<&Team, Error> {
        self.teams
            .get(&team)
            .and_then(|state| state.resolved.as_ref())
            .ok_or(Error::UnknownTeam)
    }

    /// Files a verified entry under what it is about.
    fn file(&mut self, id: EntryId, body: Body) -> Change {
        let Body {
            author, seq, kind, ..
        } = body;
        let (team, op, boxes) = match kind {
            Kind::Announce { sealing } => {
                self.announced.entry(author).or_insert(sealing);
                return Change::Nothing;
            }
            Kind::Payload {
                team,
                under,
                key,
                nonce,
                text,
            } => {
                let state = self.teams.entry(team).or_default();
                let n = state
                    .written
                    .iter()
                    .filter(|written| written.author == author)
                    .count() as u64;
                state.written.push(Written {
                    author,
                    seq,
                    n,
                    under,
                    key,
                    nonce,
                    text,
                });
                return Change::Payload(team);
            }
            Kind::Create { key, boxes } => {
                (TeamId(id.0), (id, vec![], Action::Create, Some(key)), boxes)
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
            } => (
                team,
                (under, parents, Action::Set { member, role, cut }, key),
                boxes,
            ),
            Kind::Remove {
                team,
                under,
                parents,
                member,
                cut,
                key,
                boxes,
            } => (
                team,
                (under, parents, Action::Remove { member, cut }, key),
                boxes,
            ),
            Kind::Rotate {
                team,
                under,
                parents,
                key,
                boxes,
            } => (team, (under, parents, Action::Rotate, Some(key)), boxes),
            Kind::Handover {
                team,
                under,
                parents,
                member,
                cut,
            } => (
                team,
                (under, parents, Action::Handover { member, cut }, None),
                Vec::new(),
            ),
            Kind::Leave {
                team,
                under,
                parents,
            } => (team, (under, parents, Action::Leave, None), Vec::new()),
            Kind::Accept {
                team,
                under,
                parents,
                handover,
            } => {
                let handed = handed(team, author, &handover).expect("checked when taken");
                self.anchors
                    .insert((handed.author, handed.seq), (handed.id, handover));
                let action = Action::Accept { handover: handed };
                (team, (under, parents, action, None), Vec::new())
            }
        };
        let (under, parents, action, new_key) = op;
        let state = self.teams.entry(team).or_default();
        for lockbox in &boxes {
            if lockbox.device == self.device
                && !state.keys.contains_key(&lockbox.key)
                && let Some(key) = self.sealing.open(lockbox.key, self.device, &lockbox.sealed)
            {
                state.keys.insert(lockbox.key, key);
            }
        }
        state.ops.push(Op {
            id,
            author,
            seq,
            under,
            parents,
            action,
            new_key,
            sealed: boxes
                .into_iter()
                .map(|lockbox| (lockbox.key, lockbox.device))
                .collect(),
        });
        Change::Op(team)
    }

    /// Brings a team up to date after `change` and says what is different.
    fn refresh(&mut self, change: Change) -> Vec<Event> {
        let mut events = Vec::new();
        let team = match change {
            Change::Nothing => return events,
            Change::Payload(team) => {
                let state = self.teams.get_mut(&team).expect("filed under the team");
                let written = state.written.last().expect("just filed");
                if let Some(data) = state.read(team, written) {
                    let (author, n) = (written.author, written.n);
                    state.history.insert((author, n), data.clone());
                    events.push(Event::Payload {
                        team,
                        payload: Payload { author, n, data },
                    });
                }
                return events;
            }
            Change::Op(team) => team,
        };
        self.resolve(team)
    }

    /// Resolves a team again from its operations and the forks known and
    /// says what is different.
    fn resolve(&mut self, team: TeamId) -> Vec<Event> {
        let mut events = Vec::new();
        let forks = self.forks.iter().map(|(device, fork)| (*device, fork.at));
        let forks: BTreeMap<DeviceId, u64> = forks.collect();
        let state = self.teams.get_mut(&team).expect("filed under the team");
        state.resolved = rules::resolve(team, &state.ops, &forks);

        let members = state
            .resolved
            .as_ref()
            .map(|resolved| resolved.members().clone())
            .unwrap_or_default();
        for (member, role) in &members {
            match state.members.get(member) {
                None => events.push(Event::MemberAdded {
                    team,
                    member: *member,
                    role: *role,
                }),
                Some(before) if before != role => events.push(Event::RoleChanged {
                    team,
                    member: *member,
                    role: *role,
                }),
                Some(_) => {}
            }
        }
        for member in state.members.keys() {
            if !members.contains_key(member) {
                events.push(Event::MemberRemoved {
                    team,
                    member: *member,
                });
                if *member == self.device {
                    events.push(Event::Ejected { team });
                }
            }
        }
        state.members = members;

        let before = std::mem::take(&mut state.history);
        let mut history = BTreeMap::new();
        for written in &state.written {
            let at = (written.author, written.n);
            let data = match before.get(&at) {
                // Decrypted before: only whether it still counts, and whether
                // the key is still held after a fork, can change.
                Some(data) => state
                    .resolved
                    .as_ref()
                    .is_some_and(|team| team.writes(written.author, written.seq, written.under))
                    .then(|| data.clone())
                    .filter(|_| state.keys.contains_key(&written.key)),
                None => state.read(team, written),
            };
            let payload = |data| Payload {
                author: at.0,
                n: at.1,
                data,
            };
            if let Some(data) = data {
                if !before.contains_key(&at) {
                    events.push(Event::Payload {
                        team,
                        payload: payload(data.clone()),
                    });
                }
                history.insert(at, data);
            }
        }
        // What was readable and is not any more: void now, or gone with
        // the cut end of a forked log.
        for ((author, n), data) in before {
            if !history.contains_key(&(author, n)) {
                events.push(Event::Dropped {
                    team,
                    payload: Payload { author, n, data },
                });
            }
        }
        state.history = history;
        events
    }

    /// Verifies an entry from anywhere, stores it and applies it.
    fn take(&mut self, entry: Entry) -> Result<Vec<Event>, Error> {
        if entry.version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(entry.version));
        }
        let body = format::open(&entry.bytes)?;
        let id = format::id(&entry.bytes);
        if (entry.author, entry.seq, entry.id) != (body.author, body.seq, id) {
            return Err(Error::Mismatch);
        }
        // What a forked device wrote from the fork on is not held.
        if self
            .forks
            .get(&body.author)
            .is_some_and(|fork| fork.at <= body.seq)
        {
            return Ok(Vec::new());
        }
        let log = self.logs.entry(body.author).or_default();
        let have = log.len() as u64;
        if body.seq < have {
            let held = &log[body.seq as usize];
            if held.id == id {
                return Ok(Vec::new());
            }
            // Two entries for one position: the author forked.
            let proof = format::proof(&held.bytes, &entry.bytes);
            return self.learn(&proof);
        }
        if body.seq > have {
            return Err(Error::Gap {
                have,
                got: body.seq,
            });
        }
        let prev = log.last().map_or(EntryId([0; 32]), |last| last.id);
        if body.prev != prev {
            return Err(Error::Broken);
        }
        // An acceptance carries a handover. If another entry is held, or
        // arrives, for the handover's place, its author forked.
        let carried = match &body.kind {
            Kind::Accept { team, handover, .. } => {
                let handed = handed(*team, body.author, handover)?;
                let held = self.logs.get(&handed.author);
                held.and_then(|log| log.get(handed.seq as usize))
                    .filter(|held| held.id != handed.id)
                    .map(|held| format::proof(&held.bytes, handover))
            }
            _ => None,
        };
        let anchored = self
            .anchors
            .get(&(body.author, body.seq))
            .filter(|(anchored, _)| *anchored != id)
            .map(|(_, handover)| format::proof(handover, &entry.bytes));
        if let Some(proof) = anchored {
            return self.learn(&proof);
        }
        self.logs.entry(body.author).or_default().push(entry);
        let change = self.file(id, body);
        let mut events = self.refresh(change);
        if let Some(proof) = carried {
            events.extend(self.learn(&proof)?);
        }
        Ok(events)
    }

    /// Verifies the proof of a fork. If it is news, cuts the device's log
    /// back to the fork and resolves everything again without what is gone.
    fn learn(&mut self, proof: &[u8]) -> Result<Vec<Event>, Error> {
        let (device, at) = format::open_proof(proof)?;
        if self.forks.get(&device).is_some_and(|fork| fork.at <= at) {
            return Ok(Vec::new());
        }
        let proof = proof.to_vec();
        self.forks.insert(device, Fork { at, proof });
        if let Some(log) = self.logs.get_mut(&device) {
            log.truncate(at as usize);
        }

        self.announced.clear();
        self.anchors.clear();
        for state in self.teams.values_mut() {
            state.ops.clear();
            state.written.clear();
            state.keys.clear();
        }
        let entries: Vec<Entry> = self.logs.values().flatten().cloned().collect();
        for entry in entries {
            let body = format::read(&entry.bytes).expect("held entries were verified");
            self.file(entry.id, body);
        }
        let teams: Vec<TeamId> = self.teams.keys().copied().collect();
        Ok(teams
            .into_iter()
            .flat_map(|team| self.resolve(team))
            .collect())
    }

    /// Refuses to write once this device's own log is known to be forked:
    /// nothing it wrote would count.
    fn ready(&self) -> Result<(), Error> {
        match self.forks.contains_key(&self.device) {
            true => Err(Error::Forked),
            false => Ok(()),
        }
    }

    /// Signs the next entry of this device's log and applies it.
    fn append(&mut self, kind: Kind) -> Entry {
        let log = self.logs.entry(self.device).or_default();
        let body = Body {
            author: self.device,
            seq: log.len() as u64,
            prev: log.last().map_or(EntryId([0; 32]), |last| last.id),
            kind,
        };
        let bytes = format::seal(&body, &self.signing);
        let entry = Entry {
            version: FORMAT_VERSION,
            author: body.author,
            seq: body.seq,
            id: format::id(&bytes),
            bytes,
        };
        log.push(entry.clone());
        let change = self.file(entry.id, body);
        self.refresh(change);
        entry
    }

    /// Lockboxes of `key` for each of `devices`.
    fn boxes(
        &self,
        key: &TeamKey,
        devices: impl IntoIterator<Item = DeviceId>,
    ) -> Result<Vec<Lockbox>, Error> {
        devices
            .into_iter()
            .map(|device| {
                let public = if device == self.device {
                    &self.sealing.public
                } else {
                    self.announced
                        .get(&device)
                        .ok_or(Error::NotAnnounced(device))?
                };
                Ok(Lockbox {
                    key: keys::key_id(key),
                    device,
                    sealed: keys::seal(key, device, public)?,
                })
            })
            .collect()
    }

    /// Accepts a handover to this device that is still open, so that what
    /// it does next is the owner's.
    fn accept(&mut self, team: TeamId) -> Vec<Entry> {
        let Some(resolved) = self.team(team) else {
            return Vec::new();
        };
        let handover = resolved
            .unaccepted()
            .filter(|_| resolved.owner() == Some(self.device))
            .and_then(|id| self.logs.values().flatten().find(|entry| entry.id == id));
        let Some(handover) = handover else {
            return Vec::new();
        };
        let kind = Kind::Accept {
            team,
            under: handover.id,
            parents: resolved.parents().to_vec(),
            handover: handover.bytes.clone(),
        };
        vec![self.append(kind)]
    }

    /// What this device names as `under` in an operation on members.
    fn admin_grant(&self, team: TeamId) -> Result<EntryId, Error> {
        self.resolved(team)?
            .admin_grant(self.device)
            .ok_or(Error::NotAllowed)
    }
}

impl TeamLog for Replica {
    type Error = Error;

    fn device(&self) -> DeviceId {
        self.device
    }

    async fn announce(&mut self) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        if self.announced.contains_key(&self.device) {
            return Ok(Vec::new());
        }
        Ok(vec![self.append(Kind::Announce {
            sealing: self.sealing.public,
        })])
    }

    async fn create_team(&mut self) -> Result<(TeamId, Vec<Entry>), Error> {
        self.ready()?;
        let key = keys::new_key();
        let entry = self.append(Kind::Create {
            key: keys::key_id(&key),
            boxes: self.boxes(&key, [self.device])?,
        });
        Ok((TeamId(entry.id.0), vec![entry]))
    }

    async fn add_member(
        &mut self,
        team: TeamId,
        member: DeviceId,
        role: Role,
    ) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        let mut entries = self.accept(team);
        let resolved = self.resolved(team)?;
        if resolved.role(member).is_some() {
            return Err(Error::AlreadyMember);
        }
        // A device that forked is out for good.
        if !resolved.may(self.device, member, Some(role)) || self.forks.contains_key(&member) {
            return Err(Error::NotAllowed);
        }
        // A device that was a member before gets a new key and none of the
        // old ones: it cannot read what was written while it was out. A new
        // device gets every key this one holds and reads the whole history.
        let (key, boxes) = if resolved.was_member(member) {
            let key = keys::new_key();
            let devices = resolved.members().keys().copied().chain([member]);
            (Some(keys::key_id(&key)), self.boxes(&key, devices)?)
        } else {
            let mut boxes = Vec::new();
            for key in self.teams[&team].keys.values() {
                boxes.extend(self.boxes(key, [member])?);
            }
            (None, boxes)
        };
        let kind = Kind::Set {
            team,
            under: self.admin_grant(team)?,
            parents: resolved.parents().to_vec(),
            member,
            role,
            cut: self.height(member),
            key,
            boxes,
        };
        entries.push(self.append(kind));
        Ok(entries)
    }

    async fn remove_member(&mut self, team: TeamId, member: DeviceId) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        let mut entries = self.accept(team);
        let resolved = self.resolved(team)?;
        if resolved.role(member).is_none() {
            return Err(Error::NotMember);
        }
        if !resolved.may(self.device, member, None) {
            return Err(Error::NotAllowed);
        }
        let key = keys::new_key();
        let remaining = resolved
            .members()
            .keys()
            .copied()
            .filter(|device| *device != member);
        let kind = Kind::Remove {
            team,
            under: self.admin_grant(team)?,
            parents: resolved.parents().to_vec(),
            member,
            cut: self.height(member),
            key: Some(keys::key_id(&key)),
            boxes: self.boxes(&key, remaining)?,
        };
        entries.push(self.append(kind));
        Ok(entries)
    }

    async fn set_role(
        &mut self,
        team: TeamId,
        member: DeviceId,
        role: Role,
    ) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        let mut entries = self.accept(team);
        let resolved = self.resolved(team)?;
        match resolved.role(member) {
            None => return Err(Error::NotMember),
            Some(now) if now == role => return Ok(Vec::new()),
            Some(_) => {}
        }
        if !resolved.may(self.device, member, Some(role)) {
            return Err(Error::NotAllowed);
        }
        let kind = Kind::Set {
            team,
            under: self.admin_grant(team)?,
            parents: resolved.parents().to_vec(),
            member,
            role,
            cut: self.height(member),
            key: None,
            boxes: Vec::new(),
        };
        entries.push(self.append(kind));
        Ok(entries)
    }

    async fn hand_over(&mut self, team: TeamId, member: DeviceId) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        let mut entries = self.accept(team);
        let resolved = self.resolved(team)?;
        if resolved.owner() != Some(self.device) || member == self.device {
            return Err(Error::NotAllowed);
        }
        if resolved.role(member).is_none() {
            return Err(Error::NotMember);
        }
        let kind = Kind::Handover {
            team,
            under: self.admin_grant(team)?,
            parents: resolved.parents().to_vec(),
            member,
            cut: self.height(member),
        };
        entries.push(self.append(kind));
        Ok(entries)
    }

    async fn leave(&mut self, team: TeamId) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        let resolved = self.resolved(team)?;
        let under = resolved.member_grant(self.device).ok_or(Error::NotMember)?;
        // The owner hands the team over first.
        if resolved.owner() == Some(self.device) {
            return Err(Error::NotAllowed);
        }
        // No new key comes with a leave: the leaver would know it. The old
        // one is no longer held by exactly the members, so the next writer
        // makes one.
        let kind = Kind::Leave {
            team,
            under,
            parents: resolved.parents().to_vec(),
        };
        Ok(vec![self.append(kind)])
    }

    async fn members(&self, team: TeamId) -> Result<Vec<(DeviceId, Role)>, Error> {
        Ok(self
            .resolved(team)?
            .members()
            .iter()
            .map(|(device, role)| (*device, *role))
            .collect())
    }

    async fn write(&mut self, team: TeamId, data: &[u8]) -> Result<Vec<Entry>, Error> {
        self.ready()?;
        let mut entries = self.accept(team);
        let resolved = self.resolved(team)?;
        let under = resolved.member_grant(self.device).ok_or(Error::NotMember)?;
        let held = resolved
            .current_key()
            .and_then(|id| self.teams[&team].keys.get(&id).copied());
        let key = match held {
            Some(key) => key,
            // No key is held by exactly the members, as after two removals
            // that had not seen each other. The writer makes one.
            None => {
                let key = keys::new_key();
                let kind = Kind::Rotate {
                    team,
                    under,
                    parents: resolved.parents().to_vec(),
                    key: keys::key_id(&key),
                    boxes: self.boxes(&key, resolved.members().keys().copied())?,
                };
                entries.push(self.append(kind));
                key
            }
        };
        let seq = self.height(self.device);
        let (nonce, text) = keys::encrypt(&key, team, self.device, seq, data);
        entries.push(self.append(Kind::Payload {
            team,
            under,
            key: keys::key_id(&key),
            nonce,
            text,
        }));
        Ok(entries)
    }

    async fn history(&self, team: TeamId) -> Result<Vec<Payload>, Error> {
        let state = self.teams.get(&team).ok_or(Error::UnknownTeam)?;
        Ok(state
            .history
            .iter()
            .map(|((author, n), data)| Payload {
                author: *author,
                n: *n,
                data: data.clone(),
            })
            .collect())
    }

    async fn heights(&self) -> Result<BTreeMap<DeviceId, u64>, Error> {
        Ok(self
            .logs
            .iter()
            .map(|(device, log)| (*device, log.len() as u64))
            .collect())
    }

    async fn entries_after(&self, device: DeviceId, from: u64) -> Result<Vec<Entry>, Error> {
        Ok(self
            .logs
            .get(&device)
            .and_then(|log| log.get(from as usize..))
            .unwrap_or_default()
            .to_vec())
    }

    async fn tips(&self) -> Result<BTreeMap<DeviceId, (u64, EntryId)>, Error> {
        let tips = self.logs.iter().filter_map(|(device, log)| {
            let last = log.last()?;
            Some((*device, (log.len() as u64, last.id)))
        });
        Ok(tips.collect())
    }

    async fn forks(&self) -> Result<BTreeMap<DeviceId, u64>, Error> {
        let forks = self.forks.iter().map(|(device, fork)| (*device, fork.at));
        Ok(forks.collect())
    }

    async fn fork_proof(&self, device: DeviceId) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.forks.get(&device).map(|fork| fork.proof.clone()))
    }

    async fn ingest_proof(&mut self, proof: &[u8]) -> Result<Vec<Event>, Error> {
        self.learn(proof)
    }

    async fn ingest(&mut self, entry: Entry) -> Result<(Vec<Event>, Vec<Entry>), Error> {
        Ok((self.take(entry)?, Vec::new()))
    }
}

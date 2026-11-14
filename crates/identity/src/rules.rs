//! The rules of a team: who is a member, in which role, and which key is
//! current. No cryptography and no state: [`resolve`] is a function of a set
//! of operations and of the forks known, so two replicas that hold the same
//! agree, whatever order things arrived in.
//!
//! # Ranks
//!
//! owner > admin > member > nobody. An operation on a device counts only if
//! its author ranks strictly higher than the device does, and strictly
//! higher than the role it hands out. So admins add and remove members, and
//! only the owner makes, unmakes or removes an admin. Two admins cannot
//! remove each other; nobody can remove the owner.
//!
//! # Cuts
//!
//! Every device writes one log, numbered from 0. An operation that changes a
//! device's standing names a cut: how many entries of that device's log the
//! author held. The new standing applies to the device's entries from the cut
//! on. What a device writes at or after the cut of its removal is void on
//! every replica, however it claims to be ordered, and what it wrote before
//! stays.
//!
//! Every entry also names the operation its author acts under (`under`): the
//! one that admitted it, made it admin, or made it owner. An entry counts
//! only while that admission is still the one in force at its position, so
//! entries written while removed stay void when the device is admitted again.
//!
//! # The owner
//!
//! The author of the team's first operation is the owner until it hands the
//! team over. A handover takes effect at its own position in the owner's
//! log: what the old owner writes after it is an admin's. The new owner
//! accepts with an entry of its own that carries the handover, and acts as
//! owner in what it writes under that acceptance. An owner has one log, so
//! its operations and those of its successors are in one order, and there
//! is one owner at a time.
//!
//! The acceptance is what keeps a handover standing when the old owner
//! forks below it: the handover is gone from the old owner's log, but the
//! new owner holds it, signed. Should a forked owner have handed over to
//! two devices and both accepted, neither handover counts.
//!
//! # Leaving
//!
//! A device takes itself out with a leave; the owner cannot. A leave beats
//! every admission, promotion and handover of the leaver that it had not
//! seen: a device that left stays out until somebody who knows admits it
//! again, and a handover it had not seen is void, so the old owner still
//! owns the team.
//!
//! # Forks
//!
//! A device that signed two entries for one position has forked. From the
//! lowest such position on, nothing of its log counts, on any branch, and
//! the device is out of the team for good, whatever admits it later. What
//! it wrote before stays. A reference to one of its void operations reads
//! as a reference to its last one that counts.
//!
//! When the owner forks, the team has no owner from there on. Members and
//! admins go on; nobody makes or removes an admin.
//!
//! # Concurrent operations
//!
//! - The owner's operations always count, unless a leave beats them.
//! - An admin's operation on a device is overruled when the owner acted on
//!   the same device and neither of the two had seen the other.
//! - Between admins, a removal beats an admission it had not seen.
//!
//! What is left never disagrees: admins can only admit as member or remove.
//!
//! # Keys
//!
//! A key is introduced by an operation and sealed to devices by that and
//! later operations. Only an operation that counts can introduce a key. A
//! key is held by every device it was sealed to, by an operation that counts
//! or by a device that holds it: an admission that was overruled has given
//! the key away all the same. The current key is the newest one held by
//! exactly the current members; when no key is, the next writer makes one.
//! That is how the key changes after a leave or a fork: the device that is
//! out still holds the old one.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use teamlog::{DeviceId, EntryId, Role, TeamId};

/// The hash of a team key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub [u8; 32]);

/// An operation named by another: the newest of `device` its author held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Parent {
    pub device: DeviceId,
    pub seq: u64,
    pub id: EntryId,
}

/// A handover as its acceptance carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handed {
    pub id: EntryId,
    /// The owner that handed over, and where in its log.
    pub author: DeviceId,
    pub seq: u64,
    pub under: EntryId,
    pub member: DeviceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The first operation of a team. Its author is the first owner.
    Create,
    /// Admits `member` in `role`, or changes its role.
    Set {
        member: DeviceId,
        role: Role,
        cut: u64,
    },
    Remove {
        member: DeviceId,
        cut: u64,
    },
    /// Makes `member` the owner and the author an admin.
    Handover {
        member: DeviceId,
        cut: u64,
    },
    /// Takes the ownership a handover gave the author, and keeps the
    /// handover: it stands even if the old owner's log is cut back by a
    /// fork later.
    Accept {
        handover: Handed,
    },
    /// Takes the author out.
    Leave,
    /// Introduces a key and changes nothing else.
    Rotate,
}

/// One operation of a team, as far as the rules need it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub id: EntryId,
    pub author: DeviceId,
    /// Position in the author's log.
    pub seq: u64,
    /// The operation the author acts under. Not read for [`Action::Create`].
    pub under: EntryId,
    /// For every device the author held operations of, the newest one.
    pub parents: Vec<Parent>,
    pub action: Action,
    /// The key this operation introduces.
    pub new_key: Option<KeyId>,
    /// Which key is sealed to which device.
    pub sealed: Vec<(KeyId, DeviceId)>,
}

/// A device's standing from one operation on.
#[derive(Clone, Debug)]
struct Step {
    /// `None` for the end a fork puts to a line.
    op: Option<EntryId>,
    cut: u64,
    role: Option<Role>,
    /// Counts the admissions before this step: steps of one unbroken stay
    /// share it.
    stay: u32,
}

/// Appends a step, opening a new stay when the device had no role before.
fn push(line: &mut Vec<Step>, op: Option<EntryId>, cut: u64, role: Option<Role>) {
    let stay = match line.last() {
        None => 0,
        Some(last) if last.role.is_none() && role.is_some() => last.stay + 1,
        Some(last) => last.stay,
    };
    line.push(Step {
        op,
        cut,
        role,
        stay,
    });
}

/// The step in force for the entry at `seq` of the device's log.
fn in_force(line: &[Step], seq: u64) -> Option<&Step> {
    line.iter().rev().find(|step| step.cut <= seq)
}

/// Whether an entry at `seq` that names `under` is covered by the line.
fn covers(line: &[Step], seq: u64, under: EntryId) -> bool {
    in_force(line, seq).is_some_and(|now| {
        now.role.is_some()
            && line
                .iter()
                .any(|step| step.op == Some(under) && step.stay == now.stay && step.role.is_some())
    })
}

/// The first operation of the stay that is in force at the end of the line.
fn grant(line: &[Step]) -> Option<EntryId> {
    let last = line.last().filter(|step| step.role.is_some())?;
    line.iter()
        .find(|step| step.stay == last.stay && step.role.is_some())
        .and_then(|step| step.op)
}

/// A team as one set of operations resolves it.
#[derive(Clone, Debug)]
pub struct Team {
    id: TeamId,
    /// The owner now and the operation it acts under, once it has accepted.
    owner: Option<(DeviceId, Option<EntryId>)>,
    /// The handover that made it owner, unless it is the first owner.
    handed: Option<EntryId>,
    /// Every device that was ever acted on, with all its standings in order.
    lines: BTreeMap<DeviceId, Vec<Step>>,
    /// The same from the owners' operations and the leaves alone, admin and
    /// owner or nothing.
    admin_lines: BTreeMap<DeviceId, Vec<Step>>,
    members: BTreeMap<DeviceId, Role>,
    current_key: Option<KeyId>,
    parents: Vec<Parent>,
    counted: BTreeSet<EntryId>,
}

impl Team {
    pub fn id(&self) -> TeamId {
        self.id
    }

    /// The owner now. `None` after an owner forked.
    pub fn owner(&self) -> Option<DeviceId> {
        self.owner.map(|(device, _)| device)
    }

    /// The members now.
    pub fn members(&self) -> &BTreeMap<DeviceId, Role> {
        &self.members
    }

    pub fn role(&self, device: DeviceId) -> Option<Role> {
        self.members.get(&device).copied()
    }

    /// Whether the device was a member at any time.
    pub fn was_member(&self, device: DeviceId) -> bool {
        self.lines
            .get(&device)
            .is_some_and(|line| line.iter().any(|step| step.role.is_some()))
    }

    /// Whether an entry of `author` at `seq`, written under `under`, is a
    /// member's.
    pub fn writes(&self, author: DeviceId, seq: u64, under: EntryId) -> bool {
        self.lines
            .get(&author)
            .is_some_and(|line| covers(line, seq, under))
    }

    /// Whether an entry of `author` at `seq`, written under `under`, is an
    /// admin's or an owner's. Read from the owners' operations alone.
    fn administers(&self, author: DeviceId, seq: u64, under: EntryId) -> bool {
        self.admin_lines
            .get(&author)
            .is_some_and(|line| covers(line, seq, under))
    }

    /// The handover the owner has still to accept. Until it has, it cannot
    /// act as owner.
    pub fn unaccepted(&self) -> Option<EntryId> {
        match self.owner {
            Some((_, None)) => self.handed,
            _ => None,
        }
    }

    /// What a member names as `under` in what it writes next.
    pub fn member_grant(&self, device: DeviceId) -> Option<EntryId> {
        grant(self.lines.get(&device)?)
    }

    /// What an admin or the owner names as `under` in its next operation.
    pub fn admin_grant(&self, device: DeviceId) -> Option<EntryId> {
        match self.owner {
            Some((owner, under)) if owner == device => under,
            _ => grant(self.admin_lines.get(&device)?),
        }
    }

    /// Whether `author` may now give `member` the role `role`, or remove it
    /// (`None`).
    pub fn may(&self, author: DeviceId, member: DeviceId, role: Option<Role>) -> bool {
        let rank = |role: Option<Role>| role.map_or(0, |role| role as u8 + 1);
        let mine = rank(self.role(author));
        mine > rank(self.role(member)) && mine > rank(role) && role != Some(Role::Owner)
    }

    /// The key to write under, if one is held by exactly the members.
    pub fn current_key(&self) -> Option<KeyId> {
        self.current_key
    }

    /// What the next operation names: the newest operation of every device.
    pub fn parents(&self) -> &[Parent] {
        &self.parents
    }

    /// Whether the operation took effect: it changed standings or keys.
    pub fn counted(&self, op: EntryId) -> bool {
        self.counted.contains(&op)
    }
}

/// The operations that can be placed, in an order that puts each after all
/// it descends from and depends on nothing but the set; and for each, the
/// operations it descends from directly.
///
/// An operation descends from the one before it in its author's log and
/// from those it names. A name that points into the void part of a forked
/// log stands for the last operation of that log that counts. An operation
/// is placed when everything it descends from is, down to the first one.
fn placed<'a>(
    team: TeamId,
    live: &[&'a Op],
    forks: &BTreeMap<DeviceId, u64>,
) -> (Vec<&'a Op>, Vec<Vec<usize>>) {
    let fork = |device: &DeviceId| forks.get(device).copied().unwrap_or(u64::MAX);
    let by_id: BTreeMap<EntryId, &Op> = live
        .iter()
        .filter(|op| op.action != Action::Create || op.id.0 == team.0)
        .map(|op| (op.id, *op))
        .collect();
    let mut logs: BTreeMap<DeviceId, BTreeMap<u64, EntryId>> = BTreeMap::new();
    for op in by_id.values() {
        logs.entry(op.author).or_default().insert(op.seq, op.id);
    }
    let newest_before = |device: &DeviceId, seq: u64| {
        let log = logs.get(device)?;
        log.range(..seq).next_back().map(|(_, id)| *id)
    };

    // What each operation descends from, or `None` when something it names
    // is not held.
    let mut below: BTreeMap<EntryId, Option<BTreeSet<EntryId>>> = BTreeMap::new();
    for op in by_id.values() {
        let mut deps = BTreeSet::new();
        let mut held = true;
        deps.extend(newest_before(&op.author, op.seq));
        for parent in &op.parents {
            if fork(&parent.device) <= parent.seq {
                deps.extend(newest_before(&parent.device, fork(&parent.device)));
            } else if by_id
                .get(&parent.id)
                .is_some_and(|named| (named.author, named.seq) == (parent.device, parent.seq))
            {
                deps.insert(parent.id);
            } else {
                held = false;
            }
        }
        deps.remove(&op.id);
        below.insert(op.id, held.then_some(deps));
    }

    let mut children: BTreeMap<EntryId, Vec<EntryId>> = BTreeMap::new();
    let mut missing: BTreeMap<EntryId, usize> = BTreeMap::new();
    for (id, deps) in &below {
        let Some(deps) = deps else { continue };
        missing.insert(*id, deps.len());
        for dep in deps {
            children.entry(*dep).or_default().push(*id);
        }
    }
    let mut depth: BTreeMap<EntryId, u32> = BTreeMap::new();
    let mut queue = VecDeque::new();
    if let Some(create) = by_id.get(&EntryId(team.0))
        && create.action == Action::Create
        && missing.get(&create.id) == Some(&0)
    {
        depth.insert(create.id, 0);
        queue.push_back(create.id);
    }
    let mut done = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        done.insert(id);
        let level = depth[&id];
        for child in children.get(&id).into_iter().flatten() {
            let Some(left) = missing.get_mut(child) else {
                continue;
            };
            let deepest = depth.entry(*child).or_insert(0);
            *deepest = (*deepest).max(level + 1);
            *left -= 1;
            if *left == 0 {
                queue.push_back(*child);
            }
        }
    }

    let mut order: Vec<&Op> = done.iter().map(|id| by_id[id]).collect();
    order.sort_by_key(|op| (depth[&op.id], op.id));
    let index: BTreeMap<EntryId, usize> =
        order.iter().enumerate().map(|(i, op)| (op.id, i)).collect();
    let deps = order
        .iter()
        .map(|op| {
            let deps = below[&op.id].as_ref().expect("placed ops have deps");
            deps.iter().map(|dep| index[dep]).collect()
        })
        .collect();
    (order, deps)
}

/// For every placed operation, the set of those it descends from.
struct Ancestry {
    words: usize,
    bits: Vec<u64>,
}

impl Ancestry {
    fn new(deps: &[Vec<usize>]) -> Self {
        let words = deps.len().div_ceil(64);
        let mut bits = vec![0u64; words * deps.len()];
        for (i, deps) in deps.iter().enumerate() {
            for p in deps {
                let (before, after) = bits.split_at_mut(i * words);
                let parent_row = &before[p * words..(p + 1) * words];
                for (word, parent_word) in after[..words].iter_mut().zip(parent_row) {
                    *word |= parent_word;
                }
                after[p / 64] |= 1 << (p % 64);
            }
        }
        Self { words, bits }
    }

    /// Whether `a` is an ancestor of `b`.
    fn before(&self, a: usize, b: usize) -> bool {
        self.bits[b * self.words + a / 64] & (1 << (a % 64)) != 0
    }

    /// Whether one had seen the other.
    fn comparable(&self, a: usize, b: usize) -> bool {
        a == b || self.before(a, b) || self.before(b, a)
    }
}

/// A stretch of one device's log in which it is the owner.
struct Reign {
    device: DeviceId,
    /// The first operation that knows of it: the handover, or the
    /// acceptance when the handover is no longer in the old owner's log.
    since: Option<usize>,
    /// The operation the owner acts under, as index and id: the team's
    /// first for its author, the acceptance for every later owner. `None`
    /// until the new owner has accepted.
    grant: Option<(usize, EntryId)>,
    /// The first position that is no longer the owner's.
    until: u64,
}

/// What an operation does to devices' standings: (device, cut, role).
type Effects = Vec<(DeviceId, u64, Option<Role>)>;

/// Resolves the team `team` from whatever operations of it are held and the
/// forks known: for a forked device, the lowest position it signed twice.
/// `None` while the team's first operation is not among the operations.
pub fn resolve(team: TeamId, ops: &[Op], forks: &BTreeMap<DeviceId, u64>) -> Option<Team> {
    let fork = |device: &DeviceId| forks.get(device).copied().unwrap_or(u64::MAX);
    let live: Vec<&Op> = ops.iter().filter(|op| op.seq < fork(&op.author)).collect();
    let (order, deps) = placed(team, &live, forks);
    let create = *order.first()?;
    let ancestry = Ancestry::new(&deps);

    // A leave beats what it had not seen of admissions, promotions and
    // handovers of its author.
    let mut leaves: BTreeMap<DeviceId, Vec<usize>> = BTreeMap::new();
    for (i, op) in order.iter().enumerate() {
        if op.action == Action::Leave {
            leaves.entry(op.author).or_default().push(i);
        }
    }
    let left = |i: usize, member: &DeviceId| {
        leaves
            .get(member)
            .is_some_and(|leaves| leaves.iter().any(|l| !ancestry.comparable(*l, i)))
    };

    // 1. Who owns the team, from where to where.
    let mut reigns: Vec<Reign> = Vec::new();
    let mut handovers = BTreeSet::new();
    let mut accepts = BTreeSet::new();
    let mut handed = None;
    let mut next = Some((create.author, Some(0), Some((0, create.id))));
    while let Some((device, since, grant)) = next.take() {
        // The handover that ends this reign: the first one in the owner's
        // log, or, when the owner forked and none is left there, the one
        // that was accepted, if it is the only one.
        let in_log = order
            .iter()
            .enumerate()
            .filter(|(i, op)| {
                grant.is_some_and(|(g, id)| op.under == id && ancestry.before(g, *i))
                    && op.author == device
                    && matches!(op.action, Action::Handover { member, .. }
                        if member != device && !left(*i, &member))
            })
            .min_by_key(|(_, op)| op.seq)
            .map(|(i, op)| (Some(i), op.id, op.seq + 1));
        let accepted = || {
            let named: BTreeSet<EntryId> = order
                .iter()
                .filter_map(|op| match op.action {
                    Action::Accept { handover }
                        if handover.author == device
                            && handover.member == op.author
                            && handover.member != device
                            && grant.is_some_and(|(_, id)| handover.under == id)
                            && fork(&device) <= handover.seq =>
                    {
                        Some(handover.id)
                    }
                    _ => None,
                })
                .collect();
            let only = named.first().filter(|_| named.len() == 1)?;
            Some((None, *only, fork(&device)))
        };
        let Some((handover, id, until)) = in_log.or_else(accepted) else {
            reigns.push(Reign {
                device,
                since,
                grant,
                until: fork(&device),
            });
            break;
        };
        reigns.push(Reign {
            device,
            since,
            grant,
            until,
        });
        handovers.extend(handover);
        handed = Some(id);
        // The new owner reigns from its acceptance, the first in its log.
        let accept = order
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                matches!(op.action, Action::Accept { handover }
                    if handover.id == id && handover.member == op.author)
            })
            .min_by_key(|(_, op)| op.seq);
        let member = match (handover, accept) {
            (Some(i), _) => match order[i].action {
                Action::Handover { member, .. } => member,
                _ => unreachable!("filtered for handovers"),
            },
            (None, Some((_, op))) => op.author,
            (None, None) => unreachable!("an accepted handover has an acceptance"),
        };
        let accept = accept.filter(|(_, op)| op.author == member);
        accepts.extend(accept.map(|(i, _)| i));
        next = Some((
            member,
            handover.or(accept.map(|(i, _)| i)),
            accept.map(|(i, op)| (i, op.id)),
        ));
    }
    let owner = reigns
        .last()
        .filter(|reign| reign.until == u64::MAX)
        .map(|reign| (reign.device, reign.grant.map(|(_, id)| id)));
    // Whether the operation was written by the owner of its time, knowing.
    let reigning = |i: usize, op: &Op| {
        reigns.iter().any(|reign| {
            op.author == reign.device
                && op.seq < reign.until
                && reign.since.is_some_and(|since| ancestry.before(since, i))
        })
    };
    // Whether it was written as the owner.
    let by_owner = |i: usize, op: &Op| {
        reigns.iter().any(|reign| {
            op.author == reign.device
                && op.seq < reign.until
                && reign
                    .grant
                    .is_some_and(|(g, id)| op.under == id && ancestry.before(g, i))
        })
    };

    // 2. The owners' operations. They count unless a leave beats them, and
    //    with the leaves they alone decide who is admin, and from which cut.
    let mut effects: BTreeMap<usize, Effects> = BTreeMap::new();
    effects.insert(0, vec![(create.author, 0, Some(Role::Owner))]);
    for (i, op) in order.iter().enumerate() {
        let effect = match op.action {
            Action::Set { member, role, cut }
                if by_owner(i, op)
                    && member != op.author
                    && role != Role::Owner
                    && !left(i, &member) =>
            {
                vec![(member, cut, Some(role))]
            }
            Action::Remove { member, cut } if by_owner(i, op) && member != op.author => {
                vec![(member, cut, None)]
            }
            Action::Handover { member, cut } if handovers.contains(&i) => vec![
                (member, cut, Some(Role::Owner)),
                (op.author, op.seq + 1, Some(Role::Admin)),
            ],
            Action::Accept { .. } if accepts.contains(&i) => {
                vec![(op.author, op.seq, Some(Role::Owner))]
            }
            // The owner cannot leave.
            Action::Leave if !reigning(i, op) => vec![(op.author, op.seq + 1, None)],
            _ => continue,
        };
        effects.insert(i, effect);
    }
    let mut on: BTreeMap<DeviceId, Vec<(usize, Option<Role>)>> = BTreeMap::new();
    let mut admin_lines: BTreeMap<DeviceId, Vec<Step>> = BTreeMap::new();
    for (i, effect) in &effects {
        for (device, cut, role) in effect {
            if order[*i].action != Action::Leave {
                on.entry(*device).or_default().push((*i, *role));
            }
            push(
                admin_lines.entry(*device).or_default(),
                Some(order[*i].id),
                *cut,
                role.filter(|role| *role >= Role::Admin),
            );
        }
    }
    for (device, line) in &mut admin_lines {
        if let Some(at) = forks.get(device) {
            push(line, None, *at, None);
        }
    }
    let mut resolved = Team {
        id: team,
        owner,
        handed,
        lines: BTreeMap::new(),
        admin_lines,
        members: BTreeMap::new(),
        current_key: None,
        parents: Vec::new(),
        counted: effects.keys().map(|i| order[*i].id).collect(),
    };

    // 3. The admins' operations: written as admin, on a device that is
    //    neither admin nor owner, and not overruled by an owner.
    let mut candidates = Vec::new();
    for (i, op) in order.iter().enumerate() {
        let (member, cut, role) = match op.action {
            Action::Set { member, role, cut } if role == Role::Member => (member, cut, Some(role)),
            Action::Remove { member, cut } => (member, cut, None),
            _ => continue,
        };
        if effects.contains_key(&i)
            || member == op.author
            || !resolved.administers(op.author, op.seq, op.under)
            || (role.is_some() && left(i, &member))
        {
            continue;
        }
        let owners = on.get(&member).map_or(&[][..], Vec::as_slice);
        if !owners.iter().all(|(o, _)| ancestry.comparable(*o, i)) {
            continue;
        }
        let seen = owners.iter().rev().find(|(o, _)| ancestry.before(*o, i));
        if seen.is_some_and(|(_, role)| *role >= Some(Role::Admin)) {
            continue;
        }
        candidates.push((i, member, cut, role));
    }
    let removals: Vec<(usize, DeviceId)> = candidates
        .iter()
        .filter(|(.., role)| role.is_none())
        .map(|(i, member, ..)| (*i, *member))
        .collect();
    for (i, member, cut, role) in candidates {
        let beaten = role.is_some()
            && removals
                .iter()
                .any(|(r, removed)| *removed == member && !ancestry.comparable(*r, i));
        if !beaten {
            effects.insert(i, vec![(member, cut, role)]);
            resolved.counted.insert(order[i].id);
        }
    }

    // 4. Every device's standings, in order. A fork ends a line for good.
    for (i, effect) in &effects {
        for (device, cut, role) in effect {
            push(
                resolved.lines.entry(*device).or_default(),
                Some(order[*i].id),
                *cut,
                *role,
            );
        }
    }
    for (device, line) in &mut resolved.lines {
        if let Some(at) = forks.get(device) {
            push(line, None, *at, None);
        }
        if let Some(role) = line.last().and_then(|step| step.role) {
            resolved.members.insert(*device, role);
        }
    }

    // 5. Keys. A rotation counts when a member wrote it.
    let mut holders: BTreeMap<KeyId, BTreeSet<DeviceId>> = BTreeMap::new();
    let mut introduced = Vec::new();
    for op in &order {
        if op.action == Action::Rotate && resolved.writes(op.author, op.seq, op.under) {
            resolved.counted.insert(op.id);
        }
        if !resolved.counted.contains(&op.id) {
            continue;
        }
        if let Some(key) = op.new_key {
            holders.entry(key).or_default();
            introduced.push(key);
        }
        for (key, device) in &op.sealed {
            if let Some(holders) = holders.get_mut(key) {
                holders.insert(*device);
            }
        }
    }
    // A device that holds a key can hand it on in an operation that does not
    // count, an admission that was overruled for one. The device it sealed
    // the key to has it all the same.
    loop {
        let mut grew = false;
        for op in &live {
            for (key, device) in &op.sealed {
                if let Some(holders) = holders.get_mut(key)
                    && holders.contains(&op.author)
                {
                    grew |= holders.insert(*device);
                }
            }
        }
        if !grew {
            break;
        }
    }
    let members: BTreeSet<DeviceId> = resolved.members.keys().copied().collect();
    resolved.current_key = introduced
        .into_iter()
        .rev()
        .find(|key| holders[key] == members);

    let mut newest: BTreeMap<DeviceId, Parent> = BTreeMap::new();
    for op in &order {
        let parent = Parent {
            device: op.author,
            seq: op.seq,
            id: op.id,
        };
        let held = newest.entry(op.author).or_insert(parent);
        if held.seq < op.seq {
            *held = parent;
        }
    }
    resolved.parents = newest.into_values().collect();
    Some(resolved)
}

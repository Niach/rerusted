//! The rules of a team: who is a member, in which role, and which key is
//! current. No cryptography and no state: [`resolve`] is a function of a set
//! of operations, so two replicas that hold the same set agree, whatever
//! order the operations arrived in.
//!
//! # Ranks
//!
//! owner > admin > member > nobody. The owner is the author of the team's
//! first operation and stays the owner. An operation on a device counts only
//! if its author ranks strictly higher than the device does, and strictly
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
//! one that admitted it, or made it admin. An entry counts only while that
//! admission is still the one in force at its position, so entries written
//! while removed stay void when the device is admitted again.
//!
//! # Concurrent operations
//!
//! - The owner's operations always count.
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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use teamlog::{DeviceId, EntryId, Role, TeamId};

/// The hash of a team key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub [u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The first operation of a team. Its author is the owner.
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
    /// The newest operations of the team the author held.
    pub parents: Vec<EntryId>,
    pub action: Action,
    /// The key this operation introduces.
    pub new_key: Option<KeyId>,
    /// Which key is sealed to which device.
    pub sealed: Vec<(KeyId, DeviceId)>,
}

/// A device's standing from one operation on.
#[derive(Clone, Debug)]
struct Step {
    op: EntryId,
    cut: u64,
    role: Option<Role>,
    /// Counts the admissions before this step: steps of one unbroken stay
    /// share it.
    stay: u32,
}

/// Appends a step, opening a new stay when the device had no role before.
fn push(line: &mut Vec<Step>, op: EntryId, cut: u64, role: Option<Role>) {
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
                .any(|step| step.op == under && step.stay == now.stay && step.role.is_some())
    })
}

/// The first operation of the stay that is in force at the end of the line.
fn grant(line: &[Step]) -> Option<EntryId> {
    let last = line.last().filter(|step| step.role.is_some())?;
    line.iter()
        .find(|step| step.stay == last.stay && step.role.is_some())
        .map(|step| step.op)
}

/// A team as one set of operations resolves it.
#[derive(Clone, Debug)]
pub struct Team {
    id: TeamId,
    owner: DeviceId,
    /// Every device that was ever acted on, with all its standings in order.
    lines: BTreeMap<DeviceId, Vec<Step>>,
    /// The same from the owner's operations alone, admin or nothing.
    admin_lines: BTreeMap<DeviceId, Vec<Step>>,
    members: BTreeMap<DeviceId, Role>,
    current_key: Option<KeyId>,
    heads: Vec<EntryId>,
    counted: BTreeSet<EntryId>,
}

impl Team {
    pub fn id(&self) -> TeamId {
        self.id
    }

    pub fn owner(&self) -> DeviceId {
        self.owner
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
    /// admin's or the owner's. Read from the owner's operations alone.
    fn administers(&self, author: DeviceId, seq: u64, under: EntryId) -> bool {
        self.admin_lines
            .get(&author)
            .is_some_and(|line| covers(line, seq, under))
    }

    /// What a member names as `under` in what it writes next.
    pub fn member_grant(&self, device: DeviceId) -> Option<EntryId> {
        grant(self.lines.get(&device)?)
    }

    /// What an admin or the owner names as `under` in its next operation.
    pub fn admin_grant(&self, device: DeviceId) -> Option<EntryId> {
        grant(self.admin_lines.get(&device)?)
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

    /// The newest operations, the parents of the next one.
    pub fn heads(&self) -> &[EntryId] {
        &self.heads
    }

    /// Whether the operation took effect: it changed standings or keys.
    pub fn counted(&self, op: EntryId) -> bool {
        self.counted.contains(&op)
    }
}

/// The operations that can be placed: those whose parents, and theirs, down
/// to the first operation, are all in the set. In an order that puts every
/// operation after its parents and depends on nothing but the set.
fn placed(team: TeamId, ops: &[Op]) -> Vec<&Op> {
    let by_id: BTreeMap<EntryId, &Op> = ops.iter().map(|op| (op.id, op)).collect();
    let mut children: BTreeMap<EntryId, Vec<EntryId>> = BTreeMap::new();
    let mut missing: BTreeMap<EntryId, usize> = BTreeMap::new();
    for op in by_id.values() {
        let parents: BTreeSet<_> = op.parents.iter().collect();
        missing.insert(op.id, parents.len());
        for parent in parents {
            children.entry(*parent).or_default().push(op.id);
        }
    }

    let mut depth: BTreeMap<EntryId, u32> = BTreeMap::new();
    let mut queue = VecDeque::new();
    if let Some(create) = by_id.get(&EntryId(team.0))
        && create.action == Action::Create
        && create.parents.is_empty()
    {
        depth.insert(create.id, 0);
        queue.push_back(create.id);
    }
    while let Some(id) = queue.pop_front() {
        let below = depth[&id];
        for child in children.get(&id).into_iter().flatten() {
            let level = depth.entry(*child).or_insert(0);
            *level = (*level).max(below + 1);
            let left = missing.get_mut(child).expect("every op is counted");
            *left -= 1;
            if *left == 0 && by_id[child].action != Action::Create {
                queue.push_back(*child);
            }
        }
    }

    let mut order: Vec<&Op> = by_id
        .values()
        .filter(|op| missing[&op.id] == 0 && depth.contains_key(&op.id))
        .filter(|op| op.action != Action::Create || op.id.0 == team.0)
        .copied()
        .collect();
    order.sort_by_key(|op| (depth[&op.id], op.id));
    order
}

/// For every placed operation, the set of those it descends from.
struct Ancestry {
    words: usize,
    bits: Vec<u64>,
}

impl Ancestry {
    fn new(order: &[&Op], index: &BTreeMap<EntryId, usize>) -> Self {
        let words = order.len().div_ceil(64);
        let mut bits = vec![0u64; words * order.len()];
        for (i, op) in order.iter().enumerate() {
            for parent in &op.parents {
                let p = index[parent];
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

fn target(op: &Op) -> Option<(DeviceId, u64, Option<Role>)> {
    match op.action {
        Action::Set { member, role, cut } => Some((member, cut, Some(role))),
        Action::Remove { member, cut } => Some((member, cut, None)),
        Action::Create | Action::Rotate => None,
    }
}

/// Resolves the team `team` from whatever operations of it are held.
/// `None` while the team's first operation is not among them.
pub fn resolve(team: TeamId, ops: &[Op]) -> Option<Team> {
    let order = placed(team, ops);
    let create = *order.first()?;
    let owner = create.author;
    let index: BTreeMap<EntryId, usize> =
        order.iter().enumerate().map(|(i, op)| (op.id, i)).collect();
    let ancestry = Ancestry::new(&order, &index);

    // 1. The owner's operations. They always count, and they alone decide
    //    who is admin, and from which cut.
    let mut by_owner: BTreeMap<DeviceId, Vec<usize>> = BTreeMap::new();
    let mut admin_lines: BTreeMap<DeviceId, Vec<Step>> = BTreeMap::new();
    push(
        admin_lines.entry(owner).or_default(),
        create.id,
        0,
        Some(Role::Owner),
    );
    let mut counted = BTreeSet::from([create.id]);
    for (i, op) in order.iter().enumerate() {
        let Some((member, cut, role)) = target(op) else {
            continue;
        };
        if op.author != owner
            || op.under != create.id
            || member == owner
            || role == Some(Role::Owner)
        {
            continue;
        }
        counted.insert(op.id);
        by_owner.entry(member).or_default().push(i);
        push(
            admin_lines.entry(member).or_default(),
            op.id,
            cut,
            role.filter(|role| *role == Role::Admin),
        );
    }
    let mut resolved = Team {
        id: team,
        owner,
        lines: BTreeMap::new(),
        admin_lines,
        members: BTreeMap::new(),
        current_key: None,
        heads: Vec::new(),
        counted,
    };

    // 2. The admins' operations: written as admin, on a device that is not
    //    admin, and not overruled by the owner.
    let mut candidates = Vec::new();
    for (i, op) in order.iter().enumerate() {
        let Some((member, _, role)) = target(op) else {
            continue;
        };
        if op.author == owner
            || member == owner
            || member == op.author
            || role.is_some_and(|role| role != Role::Member)
            || !resolved.administers(op.author, op.seq, op.under)
        {
            continue;
        }
        let owners = by_owner.get(&member).map_or(&[][..], Vec::as_slice);
        if !owners.iter().all(|o| ancestry.comparable(*o, i)) {
            continue;
        }
        let seen = owners.iter().rev().find(|o| ancestry.before(**o, i));
        if seen.is_some_and(
            |o| matches!(order[*o].action, Action::Set { role, .. } if role == Role::Admin),
        ) {
            continue;
        }
        candidates.push(i);
    }
    let removals: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|i| matches!(order[*i].action, Action::Remove { .. }))
        .collect();
    for i in candidates {
        let (member, _, role) = target(order[i]).expect("candidates have a target");
        let beaten = role.is_some()
            && removals.iter().any(|r| {
                target(order[*r]).is_some_and(|(removed, ..)| removed == member)
                    && !ancestry.comparable(*r, i)
            });
        if !beaten {
            resolved.counted.insert(order[i].id);
        }
    }

    // 3. Every device's standings, in order.
    push(
        resolved.lines.entry(owner).or_default(),
        create.id,
        0,
        Some(Role::Owner),
    );
    for op in &order {
        if let Some((member, cut, role)) = target(op)
            && resolved.counted.contains(&op.id)
        {
            push(resolved.lines.entry(member).or_default(), op.id, cut, role);
        }
    }
    for (device, line) in &resolved.lines {
        if let Some(role) = line.last().and_then(|step| step.role) {
            resolved.members.insert(*device, role);
        }
    }

    // 4. Keys. A rotation counts when a member wrote it.
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
        for op in ops {
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

    let parents: BTreeSet<EntryId> = order.iter().flat_map(|op| op.parents.clone()).collect();
    resolved.heads = order
        .iter()
        .map(|op| op.id)
        .filter(|id| !parents.contains(id))
        .collect();
    resolved.heads.sort();
    Some(resolved)
}

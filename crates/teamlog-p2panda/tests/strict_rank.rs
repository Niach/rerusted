//! Feasibility comparison: the "strict rank" membership rule on `p2panda-auth`
//! 0.8.0 alone (no `p2panda-spaces`), as our own implementation of its public
//! `Resolver` trait.
//!
//! The rule: a group's creator is its owner, for ever. Owner > manager >
//! everyone else. An add, remove, promote or demote counts only if its author
//! ranks strictly above the target (and above the access being granted).
//!
//! What `p2panda-auth` lets an outside resolver do, and what it does not:
//!
//! - `Resolver`, `GroupCrdt`, `GroupCrdtInnerState` (all fields) and
//!   `StrongRemove` are public, so a resolver can be plugged in.
//! - Applying an action to a membership state is not: `apply_action` and the
//!   `group::crdt::state` functions are `pub(crate)`, and the fields of
//!   `GroupMembersState` / `MemberState` are too. A resolver outside the crate
//!   cannot compute a single state itself.
//!
//! So `StrictRank` never computes states. It hands `StrongRemove` a copy of the
//! operations in which every operation the rule voids is swapped for an action
//! that is certain to fail (the author adding themself), and takes the states
//! `StrongRemove` computes from that.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::marker::PhantomData;

use p2panda_auth::group::resolver::StrongRemove;
use p2panda_auth::group::{
    GroupAction, GroupCrdt, GroupCrdtError, GroupCrdtInnerError, GroupCrdtInnerState,
    GroupCrdtState, GroupMember, GroupMembersState, GroupMembershipError,
};
use p2panda_auth::traits::{Conditions, Resolver};
use p2panda_auth::{Access, AccessLevel, GroupsOperation};
use p2panda_core::{Hash, SigningKey, VerifyingKey};

// ---------------------------------------------------------------------------
// The resolver.
// ---------------------------------------------------------------------------

type Id = VerifyingKey;
type Inner<C> = GroupCrdtInnerState<Id, Hash, GroupsOperation<C>, C>;
type Strong<C> = StrongRemove<Id, Hash, GroupsOperation<C>, C>;
type States<C> = HashMap<Id, GroupMembersState<GroupMember<Id>, C>>;

/// Strict rank on top of `StrongRemove`.
pub struct StrictRank<C>(PhantomData<C>);

impl<C: Conditions> Resolver<Id, Hash, GroupsOperation<C>, C> for StrictRank<C> {
    type State = Inner<C>;
    type Error = GroupCrdtInnerError<Hash>;

    /// The CRDT applies an operation appended to the current heads without
    /// asking the resolver, so one that breaks the rule has to force a rebuild.
    fn rebuild_required(y: &Inner<C>, operation: &GroupsOperation<C>) -> Result<bool, Self::Error> {
        if Strong::<C>::rebuild_required(y, operation)? {
            return Ok(true);
        }
        let before = y.state_at(&operation.dependencies.iter().copied().collect())?;
        Ok(!allowed(&y.operations, &before, operation))
    }

    fn process(mut y: Inner<C>) -> Result<Inner<C>, Self::Error> {
        let order = causal_order(&y);

        // Operations aimed at an owner are void whatever the state. They go
        // first: left in, `StrongRemove` would filter the owner's concurrent
        // operations, and every other verdict rests on those.
        let mut void: HashSet<Hash> = order
            .iter()
            .copied()
            .filter(|id| targets_owner(&y.operations, &y.operations[id]))
            .collect();

        loop {
            let mut shadow = y.clone();
            for id in &void {
                let operation = shadow.operations.get_mut(id).expect("operation is known");
                operation.action = GroupAction::Add {
                    member: GroupMember::Individual(operation.author),
                    access: Access::pull(),
                };
            }
            let shadow = Strong::<C>::process(shadow)?;

            // One at a time, earliest first: a verdict depends on the ranks
            // left by the operations before it, so a later one is only judged
            // once everything before it is settled.
            let mut offender = None;
            for id in order.iter().filter(|id| !void.contains(id)) {
                let operation = &y.operations[id];
                let before = shadow.state_at(&operation.dependencies.iter().copied().collect())?;
                if !allowed(&y.operations, &before, operation) {
                    offender = Some(*id);
                    break;
                }
            }

            match offender {
                Some(id) => {
                    void.insert(id);
                }
                // What is still a removal cycle here is between members who
                // each ranked above the other at some point. `StrongRemove`
                // would apply all of them; we apply none.
                None if !shadow.mutual_removes.is_empty() => {
                    void.extend(shadow.mutual_removes.iter().copied());
                }
                None => {
                    y.states = shadow.states;
                    y.ignore = &shadow.ignore | &void;
                    y.mutual_removes.clear();
                    return Ok(y);
                }
            }
        }
    }
}

/// The operations in the graph, each after everything it depends on.
fn causal_order<C: Conditions>(y: &Inner<C>) -> Vec<Hash> {
    let mut waiting: Vec<Hash> = y.graph.nodes().collect();
    waiting.sort();
    let mut order = Vec::with_capacity(waiting.len());
    let mut placed = HashSet::new();
    while !waiting.is_empty() {
        let (ready, rest): (Vec<Hash>, Vec<Hash>) = waiting.into_iter().partition(|id| {
            y.operations[id].dependencies.iter().all(|dependency| {
                placed.contains(dependency) || !y.graph.contains_node(*dependency)
            })
        });
        assert!(!ready.is_empty(), "the operation graph has a cycle");
        placed.extend(ready.iter().copied());
        order.extend(ready);
        waiting = rest;
    }
    order
}

/// The owner of a group: the author of its create operation.
fn owner_of<C>(operations: &HashMap<Hash, GroupsOperation<C>>, group: Id) -> Option<Id> {
    operations
        .values()
        .find(|operation| operation.group_id == group && operation.action.is_create())
        .map(|operation| operation.author)
}

fn targets_owner<C>(
    operations: &HashMap<Hash, GroupsOperation<C>>,
    operation: &GroupsOperation<C>,
) -> bool {
    let owner = owner_of(operations, operation.group_id).map(GroupMember::Individual);
    match &operation.action {
        GroupAction::Remove { member } | GroupAction::Demote { member, .. } => {
            Some(*member) == owner
        }
        _ => false,
    }
}

/// Whether the author outranks what the operation touches, in the state the
/// author saw.
fn allowed<C: Conditions>(
    operations: &HashMap<Hash, GroupsOperation<C>>,
    before: &States<C>,
    operation: &GroupsOperation<C>,
) -> bool {
    let owner = owner_of(operations, operation.group_id).map(GroupMember::Individual);
    let members = before
        .get(&operation.group_id)
        .map(GroupMembersState::access_levels)
        .unwrap_or_default();
    let granted = |access: &Access<C>| if access.is_manage() { 2 } else { 1 };
    let rank = |who: GroupMember<Id>| {
        if Some(who) == owner {
            return 3;
        }
        members
            .iter()
            .find(|(member, _)| *member == who)
            .map_or(0, |(_, access)| granted(access))
    };

    let author = rank(GroupMember::Individual(operation.author));
    match &operation.action {
        GroupAction::Create { .. } => true,
        GroupAction::Remove { member } | GroupAction::Demote { member, .. } => {
            author > rank(*member)
        }
        GroupAction::Add { member, access } | GroupAction::Promote { member, access } => {
            author > rank(*member) && author > granted(access)
        }
    }
}

// ---------------------------------------------------------------------------
// The tests: the group CRDT driven directly, no orderer, no store, no spaces.
// ---------------------------------------------------------------------------

type Op = GroupsOperation<()>;
type Roles = BTreeMap<&'static str, AccessLevel>;

const NAMES: [&str; 6] = ["group", "owner", "ann", "bob", "pat", "kim"];

fn key(name: &str) -> Id {
    let seed = NAMES.iter().position(|known| *known == name).unwrap() as u8 + 1;
    SigningKey::from_bytes(&[seed; 32]).verifying_key()
}

fn person(name: &str) -> GroupMember<Id> {
    GroupMember::Individual(key(name))
}

fn op(author: &str, after: &[&Op], action: GroupAction<Id, ()>) -> Op {
    let dependencies: Vec<Hash> = after.iter().map(|operation| operation.id).collect();
    Op {
        id: Hash::digest(format!("{author} {dependencies:?} {action:?}")),
        author: key(author),
        dependencies,
        group_id: key("group"),
        action,
    }
}

/// The owner creates the group with two managers (ann, bob) and a writer (pat).
fn genesis() -> Op {
    let initial_members = vec![
        (person("owner"), Access::manage()),
        (person("ann"), Access::manage()),
        (person("bob"), Access::manage()),
        (person("pat"), Access::write()),
    ];
    op("owner", &[], GroupAction::Create { initial_members })
}

fn remove(author: &str, who: &str, after: &[&Op]) -> Op {
    let member = person(who);
    op(author, after, GroupAction::Remove { member })
}

fn add(author: &str, who: &str, access: Access<()>, after: &[&Op]) -> Op {
    let member = person(who);
    op(author, after, GroupAction::Add { member, access })
}

fn promote(author: &str, who: &str, access: Access<()>, after: &[&Op]) -> Op {
    let member = person(who);
    op(author, after, GroupAction::Promote { member, access })
}

fn demote(author: &str, who: &str, access: Access<()>, after: &[&Op]) -> Op {
    let member = person(who);
    op(author, after, GroupAction::Demote { member, access })
}

fn roles(members: &[(&'static str, AccessLevel)]) -> Roles {
    members.iter().copied().collect()
}

const EVERYONE: [(&str, AccessLevel); 4] = [
    ("owner", AccessLevel::Manage),
    ("ann", AccessLevel::Manage),
    ("bob", AccessLevel::Manage),
    ("pat", AccessLevel::Write),
];

/// Every order the operations can arrive in (dependencies first).
fn arrival_orders<'a>(operations: &[&'a Op]) -> Vec<Vec<&'a Op>> {
    fn extend<'a>(operations: &[&'a Op], placed: &mut Vec<&'a Op>, orders: &mut Vec<Vec<&'a Op>>) {
        if placed.len() == operations.len() {
            orders.push(placed.clone());
            return;
        }
        for operation in operations {
            let seen = |id: &Hash| placed.iter().any(|earlier| earlier.id == *id);
            if !seen(&operation.id) && operation.dependencies.iter().all(seen) {
                placed.push(operation);
                extend(operations, placed, orders);
                placed.pop();
            }
        }
    }
    let mut orders = Vec::new();
    extend(operations, &mut Vec::new(), &mut orders);
    orders
}

/// The membership each arrival order ends in. One entry means every replica
/// agrees.
fn outcomes<RS>(operations: &[&Op]) -> BTreeSet<Roles>
where
    RS: Resolver<Id, Hash, Op, (), State = GroupCrdtInnerState<Id, Hash, Op, ()>>,
{
    let orders = arrival_orders(operations);
    assert!(!orders.is_empty());
    orders
        .into_iter()
        .map(|order| {
            let mut y = GroupCrdtState::new();
            for operation in order {
                y = GroupCrdt::<Id, Hash, Op, (), RS>::process(y, operation).unwrap();
            }
            y.members(key("group"))
                .into_iter()
                .map(|(id, access)| {
                    let name = NAMES.iter().find(|name| key(name) == id).unwrap();
                    (*name, access.level())
                })
                .collect()
        })
        .collect()
}

fn strict(operations: &[&Op]) -> BTreeSet<Roles> {
    let found = outcomes::<StrictRank<()>>(operations);
    println!("strict rank, {} operations: {found:?}", operations.len());
    found
}

fn agreed(members: &[(&'static str, AccessLevel)]) -> BTreeSet<Roles> {
    BTreeSet::from([roles(members)])
}

/// Case 1. Two managers remove each other concurrently: neither removal counts.
#[test]
fn managers_removing_each_other_both_stay() {
    let g = genesis();
    let ann_removes_bob = remove("ann", "bob", &[&g]);
    let bob_removes_ann = remove("bob", "ann", &[&g]);
    let operations = [&g, &ann_removes_bob, &bob_removes_ann];

    assert_eq!(strict(&operations), agreed(&EVERYONE));

    // The same operations under p2panda's own rule: both managers are gone.
    let strong = outcomes::<StrongRemove<Id, Hash, Op, ()>>(&operations);
    println!("strong remove: {strong:?}");
    assert_eq!(
        strong,
        agreed(&[("owner", AccessLevel::Manage), ("pat", AccessLevel::Write)])
    );
}

/// Case 2. Owner and manager remove each other concurrently: the owner's stands.
#[test]
fn owner_and_manager_removing_each_other_owner_wins() {
    let g = genesis();
    let owner_removes_ann = remove("owner", "ann", &[&g]);
    let ann_removes_owner = remove("ann", "owner", &[&g]);

    assert_eq!(
        strict(&[&g, &owner_removes_ann, &ann_removes_owner]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
            ("pat", AccessLevel::Write),
        ])
    );
}

/// Case 3. A manager removes (or demotes) the owner with nothing concurrent:
/// ignored, and the owner goes on to remove that manager.
#[test]
fn manager_cannot_remove_or_demote_the_owner() {
    let g = genesis();
    let ann_removes_owner = remove("ann", "owner", &[&g]);
    assert_eq!(strict(&[&g, &ann_removes_owner]), agreed(&EVERYONE));

    let bob_demotes_owner = demote("bob", "owner", Access::read(), &[&ann_removes_owner]);
    assert_eq!(
        strict(&[&g, &ann_removes_owner, &bob_demotes_owner]),
        agreed(&EVERYONE)
    );

    let owner_removes_ann = remove("owner", "ann", &[&bob_demotes_owner]);
    assert_eq!(
        strict(&[
            &g,
            &ann_removes_owner,
            &bob_demotes_owner,
            &owner_removes_ann
        ]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
            ("pat", AccessLevel::Write),
        ])
    );
}

/// Case 4. A manager the owner removed, who had not seen it and removed a
/// member: the manager's removal is ignored (`StrongRemove`'s behaviour, kept).
#[test]
fn removed_managers_concurrent_removal_is_ignored() {
    let g = genesis();
    let owner_removes_ann = remove("owner", "ann", &[&g]);
    let ann_removes_pat = remove("ann", "pat", &[&g]);

    assert_eq!(
        strict(&[&g, &owner_removes_ann, &ann_removes_pat]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
            ("pat", AccessLevel::Write),
        ])
    );
}

/// Case 5. One manager re-adds a member while another removes them: removed.
#[test]
fn concurrent_add_and_remove_removal_wins() {
    let g = genesis();
    let ann_removes_pat = remove("ann", "pat", &[&g]);
    let ann_adds_pat = add("ann", "pat", Access::write(), &[&ann_removes_pat]);
    let bob_removes_pat = remove("bob", "pat", &[&g]);

    assert_eq!(
        strict(&[&g, &ann_removes_pat, &ann_adds_pat, &bob_removes_pat]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("ann", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
        ])
    );
}

/// Only the owner makes and unmakes managers; managers manage the rest.
#[test]
fn only_the_owner_grants_and_revokes_manage() {
    let g = genesis();

    // A manager cannot promote to manager, nor add a manager.
    let ann_promotes_pat = promote("ann", "pat", Access::manage(), &[&g]);
    let ann_adds_kim_manager = add("ann", "kim", Access::manage(), &[&ann_promotes_pat]);
    assert_eq!(
        strict(&[&g, &ann_promotes_pat, &ann_adds_kim_manager]),
        agreed(&EVERYONE)
    );

    // Nor does a promotion that did not count give pat a manager's powers.
    // The CRDT validates a new operation against the resolved state at its
    // dependencies, so this one is refused outright, never entering the graph.
    let pat_removes_bob = remove("pat", "bob", &[&ann_promotes_pat]);
    let mut y = GroupCrdtState::new();
    for operation in [&g, &ann_promotes_pat] {
        y = GroupCrdt::<Id, Hash, Op, (), StrictRank<()>>::process(y, operation).unwrap();
    }
    let refused = GroupCrdt::<Id, Hash, Op, (), StrictRank<()>>::process(y, &pat_removes_bob);
    assert!(matches!(
        refused,
        Err(GroupCrdtError::StateChangeError(
            _,
            GroupMembershipError::InsufficientAccess(_)
        ))
    ));

    // A manager can add and remove below their own rank.
    let ann_adds_kim = add("ann", "kim", Access::read(), &[&g]);
    let bob_removes_pat = remove("bob", "pat", &[&ann_adds_kim]);
    assert_eq!(
        strict(&[&g, &ann_adds_kim, &bob_removes_pat]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("ann", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
            ("kim", AccessLevel::Read),
        ])
    );

    // The owner can do both; a manager the owner made cannot remove a peer.
    let owner_promotes_pat = promote("owner", "pat", Access::manage(), &[&g]);
    let owner_demotes_bob = demote("owner", "bob", Access::read(), &[&owner_promotes_pat]);
    let pat_removes_ann = remove("pat", "ann", &[&owner_demotes_bob]);
    assert_eq!(
        strict(&[
            &g,
            &owner_promotes_pat,
            &owner_demotes_bob,
            &pat_removes_ann
        ]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("ann", AccessLevel::Manage),
            ("bob", AccessLevel::Read),
            ("pat", AccessLevel::Manage),
        ])
    );
}

/// A demotion that did not count takes nothing from its target: bob, "demoted"
/// by a peer, still removes a member.
#[test]
fn a_void_demotion_leaves_its_target_in_power() {
    let g = genesis();
    let ann_demotes_bob = demote("ann", "bob", Access::read(), &[&g]);
    let bob_removes_pat = remove("bob", "pat", &[&ann_demotes_bob]);

    assert_eq!(
        strict(&[&g, &ann_demotes_bob, &bob_removes_pat]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("ann", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
        ])
    );
}

/// The owner's concurrent work survives an attempt on the owner: without the
/// owner-first pass `StrongRemove` would filter the promotion, and pat's
/// removal of kim with it.
#[test]
fn an_attempt_on_the_owner_does_not_undo_the_owners_concurrent_work() {
    let g = genesis();
    let ann_adds_kim = add("ann", "kim", Access::read(), &[&g]);
    let owner_promotes_pat = promote("owner", "pat", Access::manage(), &[&ann_adds_kim]);
    let pat_removes_kim = remove("pat", "kim", &[&owner_promotes_pat]);
    let bob_removes_owner = remove("bob", "owner", &[&ann_adds_kim]);

    assert_eq!(
        strict(&[
            &g,
            &ann_adds_kim,
            &owner_promotes_pat,
            &pat_removes_kim,
            &bob_removes_owner,
        ]),
        agreed(&[
            ("owner", AccessLevel::Manage),
            ("ann", AccessLevel::Manage),
            ("bob", AccessLevel::Manage),
            ("pat", AccessLevel::Manage),
        ])
    );
}

//! Taking a team over after its owner forked (decided 2026-11-14): of the
//! admins, the one that has been admin for the longest may, with a claim.

mod common;

use common::{Node, now, settle, sync};
use identity::Error;
use identity::format::Kind;
use teamlog::Role::{Admin, Member, Owner};
use teamlog::{DeviceId, TeamId, TeamLog};

/// a owns the team, b and c are admins, b for longer, d and e members.
/// Everybody has everything.
fn team() -> (TeamId, [Node; 5]) {
    let mut nodes = ["a", "b", "c", "d", "e"].map(Node::new);
    {
        let [a, b, c, d, e] = &mut nodes;
        settle(&mut [a, b, c, d, e]);
    }
    let [a, b, c, d, e] = &mut nodes;
    let (team, _) = now(a.log.create_team()).unwrap();
    now(a.log.add_member(team, b.id(), Admin)).unwrap();
    now(a.log.add_member(team, c.id(), Admin)).unwrap();
    now(a.log.add_member(team, d.id(), Member)).unwrap();
    now(a.log.add_member(team, e.id(), Member)).unwrap();
    settle(&mut [a, b, c, d, e]);
    (team, nodes)
}

/// Puts `node` back to a backup taken now and lets it write on both sides
/// of it, so that its log forks. `witness` holds one branch.
fn fork(team: TeamId, node: &mut Node, witness: &mut Node) {
    let backup = node.backup();
    node.write(team, "x");
    sync(node, witness);
    node.restore(&backup);
    node.forge(Kind::Announce { sealing: [7; 32] });
}

fn owner(node: &Node, team: TeamId) -> Option<DeviceId> {
    node.log.team(team).unwrap().owner()
}

fn heir(node: &Node, team: TeamId) -> Option<(DeviceId, DeviceId)> {
    node.log.team(team).unwrap().heir()
}

/// Everybody sees the same owner, heir and members.
fn agree(team: TeamId, nodes: &[&Node]) {
    for node in nodes {
        assert_eq!(owner(node, team), owner(nodes[0], team), "{}", node.name);
        assert_eq!(heir(node, team), heir(nodes[0], team), "{}", node.name);
        assert_eq!(
            now(node.log.members(team)).unwrap(),
            now(nodes[0].log.members(team)).unwrap(),
            "{}",
            node.name
        );
    }
}

#[test]
fn the_longest_standing_admin_takes_over() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    assert_eq!(heir(b, team), None);
    assert_eq!(b.log.take_over(team), Err(Error::NotAllowed));

    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    for node in [&*a, &*b, &*c, &*d, &*e] {
        assert_eq!(owner(node, team), None, "{}", node.name);
        assert_eq!(heir(node, team), Some((a.id(), b.id())), "{}", node.name);
    }

    // Only the heir may, and nobody else.
    assert_eq!(c.log.take_over(team), Err(Error::NotAllowed));
    assert_eq!(d.log.take_over(team), Err(Error::NotAllowed));
    b.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    for node in all {
        assert_eq!(owner(node, team), Some(b.id()), "{}", node.name);
        assert_eq!(heir(node, team), None, "{}", node.name);
        assert_eq!(
            node.roles(team, &all),
            [("b", Owner), ("c", Admin), ("d", Member), ("e", Member)],
            "{}",
            node.name
        );
    }

    // It is the owner in full: it makes and removes admins and hands over.
    now(b.log.set_role(team, e.id(), Admin)).unwrap();
    now(b.log.remove_member(team, c.id())).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(
        d.roles(team, &all),
        [("b", Owner), ("d", Member), ("e", Admin)]
    );
    now(b.log.hand_over(team, e.id())).unwrap();
    settle(&mut [a, b, c, d, e]);
    now(e.log.set_role(team, d.id(), Admin)).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(
        d.roles(team, &all),
        [("b", Admin), ("d", Admin), ("e", Owner)]
    );
    b.write(team, "on");
    settle(&mut [a, b, c, d, e]);
    assert!(d.texts(team).contains(&"on".to_string()));
}

/// An admin that is not the heir writes a claim all the same.
#[test]
fn a_claim_by_another_admin_does_not_count() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);

    let resolved = c.log.team(team).unwrap();
    let claim = Kind::Claim {
        team,
        under: resolved.admin_grant(c.id()).unwrap(),
        parents: resolved.parents().to_vec(),
        owner: a.id(),
    };
    c.forge(claim);
    // A member's, too.
    let resolved = d.log.team(team).unwrap();
    let claim = Kind::Claim {
        team,
        under: resolved.member_grant(d.id()).unwrap(),
        parents: resolved.parents().to_vec(),
        owner: a.id(),
    };
    d.forge(claim);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(e, team), None);
    assert_eq!(heir(e, team), Some((a.id(), b.id())));

    // The heir's claim counts whenever it comes.
    b.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(e, team), Some(b.id()));
    assert_eq!(
        e.roles(team, &all),
        [("b", Owner), ("c", Admin), ("d", Member), ("e", Member)]
    );
}

/// A claim names the owner that forked. One written for an owner that did
/// not fork is nothing.
#[test]
fn a_claim_against_an_owner_that_did_not_fork_does_not_count() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    let resolved = b.log.team(team).unwrap();
    let claim = Kind::Claim {
        team,
        under: resolved.admin_grant(b.id()).unwrap(),
        parents: resolved.parents().to_vec(),
        owner: a.id(),
    };
    b.forge(claim);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(e, team), Some(a.id()));
    assert_eq!(e.roles(team, &all)[..2], [("a", Owner), ("b", Admin)]);
}

/// The heir is the admin whose present stay as admin began first: an admin
/// that was demoted and promoted again starts anew.
#[test]
fn a_break_as_admin_starts_the_count_again() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    now(a.log.set_role(team, b.id(), Member)).unwrap();
    now(a.log.set_role(team, b.id(), Admin)).unwrap();
    settle(&mut [a, b, c, d, e]);
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    for node in [&*a, &*b, &*c, &*d, &*e] {
        assert_eq!(heir(node, team), Some((a.id(), c.id())), "{}", node.name);
    }
    assert_eq!(b.log.take_over(team), Err(Error::NotAllowed));
    c.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(d, team), Some(c.id()));
}

/// The heir leaves instead: the next admin is the heir.
#[test]
fn when_the_heir_leaves_the_next_admin_is_the_heir() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    now(b.log.leave(team)).unwrap();
    settle(&mut [a, b, c, d, e]);
    for node in [&*a, &*b, &*c, &*d, &*e] {
        assert_eq!(heir(node, team), Some((a.id(), c.id())), "{}", node.name);
    }
    c.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(
        d.roles(team, &all),
        [("c", Owner), ("d", Member), ("e", Member)]
    );
}

/// No admin when the owner forks: no heir, no owner, as before.
#[test]
fn without_an_admin_there_is_no_heir() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    now(a.log.set_role(team, b.id(), Member)).unwrap();
    now(a.log.set_role(team, c.id(), Member)).unwrap();
    settle(&mut [a, b, c, d, e]);
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    for node in all {
        assert_eq!(owner(node, team), None, "{}", node.name);
        assert_eq!(heir(node, team), None, "{}", node.name);
    }
    assert_eq!(b.log.take_over(team), Err(Error::NotAllowed));
    b.write(team, "on");
    settle(&mut [a, b, c, d, e]);
    assert!(e.texts(team).contains(&"on".to_string()));
}

/// The admin that took over forks in its turn: the next one may.
#[test]
fn the_new_owner_forks_too() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    b.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);
    assert_eq!(owner(d, team), Some(b.id()));

    fork(team, b, c);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(d, team), None);
    assert_eq!(heir(d, team), Some((b.id(), c.id())));
    c.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(
        d.roles(team, &all),
        [("c", Owner), ("d", Member), ("e", Member)]
    );
}

/// The owner had handed the team to d and d had accepted, but b knew only
/// of the fork below the handover and claimed. The accepted handover
/// stands; the claim is nothing.
#[test]
fn an_accepted_handover_stands_against_a_claim() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    let backup = a.backup();

    // a hands over to d, d acts as owner; only e hears of it.
    now(a.log.hand_over(team, d.id())).unwrap();
    sync(a, d);
    now(d.log.set_role(team, e.id(), Admin)).unwrap();
    sync(d, e);

    // a is put back and forks below the handover; b and c see the fork
    // and a's branch without the handover.
    a.restore(&backup);
    a.write(team, "from the backup");
    sync(a, b);
    sync(e, b);
    sync(b, c);
    // What b holds depends on what e gave it; if it sees no owner, it
    // claims. Either way everybody ends with d.
    if heir(b, team) == Some((a.id(), b.id())) {
        b.log.take_over(team).unwrap();
    }
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    for node in all {
        assert_eq!(owner(node, team), Some(d.id()), "{}", node.name);
        assert_eq!(
            node.roles(team, &all),
            [("b", Admin), ("c", Admin), ("d", Owner), ("e", Admin)],
            "{}",
            node.name
        );
    }
}

/// b claims before the others have heard of the fork, and c, who does not
/// know of the claim, goes on as admin. Everybody ends the same.
#[test]
fn a_claim_and_an_admins_operation_at_once_agree() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    fork(team, a, b);
    // b holds both branches' evidence only after it meets a again.
    sync(a, b);
    assert_eq!(heir(b, team), Some((a.id(), b.id())));
    b.log.take_over(team).unwrap();
    now(b.log.set_role(team, d.id(), Admin)).unwrap();
    now(c.log.remove_member(team, d.id())).unwrap();
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    // The owner's promotion counts against the admin's removal.
    assert_eq!(
        e.roles(team, &all),
        [("b", Owner), ("c", Admin), ("d", Admin), ("e", Member)]
    );
}

/// A forked owner writes a claim on its own team below its fork. It is not
/// its own heir.
#[test]
fn a_forked_owner_does_not_succeed_itself() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    let resolved = a.log.team(team).unwrap();
    let claim = Kind::Claim {
        team,
        under: resolved.admin_grant(a.id()).unwrap(),
        parents: resolved.parents().to_vec(),
        owner: a.id(),
    };
    a.forge(claim);
    settle(&mut [a, b, c, d, e]);
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(e, team), None);
    assert_eq!(heir(e, team), Some((a.id(), b.id())));
}

/// The heir had not claimed when it forked: it is passed over.
#[test]
fn an_heir_that_forks_before_claiming_is_passed_over() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    fork(team, a, b);
    settle(&mut [a, b, c, d, e]);
    fork(team, b, c);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(heir(e, team), Some((a.id(), c.id())));
    c.log.take_over(team).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    assert_eq!(owner(e, team), Some(c.id()));
}

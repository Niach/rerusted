//! A user's several devices in a team (#RUST-42): each device says whose it
//! is, a user's devices let each other in and take each other out.
//!
//! The team: `a` owns it and is one device of the first user, whose other
//! devices `a2` and `a3` are not in yet. `b` is an admin and `r` a member,
//! each a device of a user of its own with a second device, `b2` and `r2`,
//! not in yet. `c` is a member that never said whose it is.

mod common;

use common::{Node, now, one_user, settle, sync};
use identity::format::Kind;
use identity::{Error, certificate};
use teamlog::Role::{Admin, Member, Owner};
use teamlog::{DeviceId, Event, Role, TeamId, TeamLog};

const NAMES: [&str; 8] = ["a", "a2", "a3", "b", "b2", "r", "r2", "c"];

fn team() -> (TeamId, [Node; 8]) {
    let mut nodes = NAMES.map(Node::new);
    let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
    one_user(&mut [a, a2, a3]);
    one_user(&mut [b, b2]);
    one_user(&mut [r, r2]);
    settle(&mut [a, a2, a3, b, b2, r, r2, c]);
    let (team, _) = now(a.log.create_team()).unwrap();
    now(a.log.add_member(team, b.id(), Admin)).unwrap();
    now(a.log.add_member(team, r.id(), Member)).unwrap();
    now(a.log.add_member(team, c.id(), Member)).unwrap();
    settle(&mut [a, a2, a3, b, b2, r, r2, c]);
    (team, nodes)
}

fn all(nodes: &mut [Node; 8]) {
    let [a, a2, a3, b, b2, r, r2, c] = nodes;
    settle(&mut [a, a2, a3, b, b2, r, r2, c]);
}

/// The roles everybody agrees on, by name.
fn roles(team: TeamId, nodes: &[Node; 8]) -> Vec<(&'static str, Role)> {
    let refs: Vec<&Node> = nodes.iter().collect();
    let first = nodes[0].roles(team, &refs);
    for node in nodes {
        assert_eq!(node.roles(team, &refs), first, "{}", node.name);
        assert_eq!(
            node.log.team(team).unwrap().owner(),
            nodes[0].log.team(team).unwrap().owner(),
            "{}",
            node.name
        );
        assert_eq!(
            node.log.team(team).unwrap().current_key(),
            nodes[0].log.team(team).unwrap().current_key(),
            "{}",
            node.name
        );
        assert_eq!(
            node.log.void(team),
            nodes[0].log.void(team),
            "{}",
            node.name
        );
    }
    first
}

fn owner(team: TeamId, nodes: &[Node; 8]) -> Option<DeviceId> {
    nodes[7].log.team(team).unwrap().owner()
}

fn grant(node: &Node, team: TeamId) -> teamlog::EntryId {
    let resolved = node.log.team(team).unwrap();
    resolved
        .member_grant(node.id())
        .unwrap_or(teamlog::EntryId(team.0))
}

const START: [(&str, Role); 4] = [("a", Owner), ("b", Admin), ("c", Member), ("r", Member)];

#[test]
fn a_device_says_whose_it_is() {
    let (team, nodes) = team();
    let [a, a2, a3, b, b2, _, _, c] = &nodes;
    assert_eq!(roles(team, &nodes), START);
    // Everybody knows the users, also of devices that are in no team.
    for node in &nodes {
        let user = node.log.user(a.id());
        assert!(user.is_some());
        assert_eq!(node.log.user(a2.id()), user, "{}", node.name);
        assert_eq!(node.log.user(a3.id()), user, "{}", node.name);
        assert_ne!(node.log.user(b.id()), user, "{}", node.name);
        assert_eq!(node.log.user(b2.id()), node.log.user(b.id()));
        assert_eq!(node.log.user(c.id()), None);
    }
}

#[test]
fn a_chain_that_is_not_the_devices_is_not_published_or_taken() {
    let (_, mut nodes) = team();
    let [a, a2, _, b, _, _, _, c] = &mut nodes;
    // a2's chain, published by c.
    let stolen =
        match identity::format::read(&now(a2.log.entries_after(a2.id(), 1)).unwrap()[0].bytes)
            .unwrap()
            .kind
        {
            Kind::Certify { chain } => chain,
            other => panic!("{other:?}"),
        };
    assert_eq!(c.log.certify(&stolen), Err(Error::Mismatch));
    assert!(c.log.certify(&[1, 2, 3]).is_err());
    // Written all the same: nobody takes the entry.
    let forged = c.sign(Kind::Certify { chain: stolen });
    assert_eq!(now(b.log.ingest(forged)), Err(Error::Mismatch));
    assert_eq!(b.log.user(c.id()), None);
    // A second chain changes nothing: the first one counts.
    assert!(
        a.log
            .certify(&certificate::chain(&[vec![0; 161]]).unwrap())
            .is_err()
    );
}

#[test]
fn a_users_devices_let_a_new_device_in() {
    let (team, mut nodes) = team();
    {
        let [a, a2, _, b, b2, r, r2, c] = &mut nodes;
        // The owner's device admits as admin at most.
        assert_eq!(a.log.admit(team, a2.id(), Owner), Err(Error::NotAllowed));
        a.log.admit(team, a2.id(), Admin).unwrap();
        // An admin's as admin, a member's as member only.
        b.log.admit(team, b2.id(), Admin).unwrap();
        assert_eq!(r.log.admit(team, r2.id(), Admin), Err(Error::NotAllowed));
        r.log.admit(team, r2.id(), Member).unwrap();
        // Not a device of another user, nor one that never said whose it is.
        assert_eq!(b.log.admit(team, r2.id(), Member), Err(Error::NotAllowed));
        assert_eq!(c.log.admit(team, a2.id(), Member), Err(Error::NotAllowed));
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [
            ("a", Owner),
            ("a2", Admin),
            ("b", Admin),
            ("b2", Admin),
            ("c", Member),
            ("r", Member),
            ("r2", Member)
        ]
    );
    // The new devices read what was written before they came, and write.
    {
        let [a, a2, _, _, _, _, r2, _] = &mut nodes;
        a.write(team, "one");
        sync(a, r2);
        assert_eq!(r2.texts(team), ["one"]);
        // A device let in by its own user acts in its role.
        now(a2.log.remove_member(team, r2.id())).unwrap();
    }
    all(&mut nodes);
    assert!(!roles(team, &nodes).contains(&("r2", Member)));
}

/// Admissions that the rules do not back, written all the same.
#[test]
fn an_admission_nobody_may_write_does_not_count() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, _, r, r2, c] = &mut nodes;
        // A member admits its own device as admin.
        r.forge(r.admission(team, r2.id(), Admin, grant(r, team)));
        // An admin admits a device of another user, and one of nobody.
        b.forge(b.admission(team, a2.id(), Member, grant(b, team)));
        c.forge(c.admission(team, a3.id(), Member, grant(c, team)));
        // A device that is not in the team admits its sibling.
        a2.forge(a2.admission(team, a3.id(), Member, grant(a, team)));
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), START);
}

#[test]
fn a_device_takes_its_users_other_device_out() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        r2.write(team, "before");
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        let key = c.log.team(team).unwrap().current_key();

        // Others' devices, and a device that is not in, cannot be revoked.
        assert_eq!(r.log.revoke(team, c.id()), Err(Error::NotAllowed));
        assert_eq!(r.log.revoke(team, b.id()), Err(Error::NotAllowed));
        assert_eq!(b.log.revoke(team, b2.id()), Err(Error::NotMember));

        r.log.revoke(team, r2.id()).unwrap();
        // r2 has not heard of it and writes on.
        r2.write(team, "after");
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        assert_ne!(c.log.team(team).unwrap().current_key(), key);
        assert!(r2.events.contains(&Event::Ejected { team }));

        c.write(team, "without r2");
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        assert_eq!(c.texts(team), ["before", "without r2"]);
        assert_eq!(r2.texts(team), ["before"]);
    }
    assert_eq!(roles(team, &nodes), START);
}

#[test]
fn revoking_the_owners_device_makes_the_revoker_the_owner() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        a.log.admit(team, a2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        // The stolen owner device goes on, unseen.
        a2.log.revoke(team, a.id()).unwrap();
        now(a.log.set_role(team, c.id(), Admin)).unwrap();
        now(a.log.remove_member(team, b.id())).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a2", Owner), ("b", Admin), ("c", Member), ("r", Member)]
    );
    assert_eq!(owner(team, &nodes), Some(nodes[1].id()));
    // It is the owner in full, and lets the user's third device in.
    {
        let [_, a2, a3, b, _, r, _, _] = &mut nodes;
        now(a2.log.set_role(team, r.id(), Admin)).unwrap();
        now(a2.log.remove_member(team, b.id())).unwrap();
        a2.log.admit(team, a3.id(), Admin).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a2", Owner), ("a3", Admin), ("c", Member), ("r", Admin)]
    );
    let [a, ..] = &mut nodes;
    assert_eq!(now(a.log.write(team, b"x")), Err(Error::NotMember));
}

/// The revoked device has heard of it and revokes back.
#[test]
fn a_revoked_device_cannot_revoke_back() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        a.log.admit(team, a2.id(), Admin).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        let under = grant(a2, team);
        a.log.revoke(team, a2.id()).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        assert_eq!(a2.log.revoke(team, a.id()), Err(Error::NotMember));
        a2.forge(a2.revocation(team, a.id(), under));
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), START);
}

/// Two devices of one user revoke each other and neither has seen it:
/// nothing says which was first, and both are out.
#[test]
fn two_devices_that_revoke_each_other_at_once_are_both_out() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        r.log.revoke(team, r2.id()).unwrap();
        r2.log.revoke(team, r.id()).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a", Owner), ("b", Admin), ("c", Member)]
    );

    // The same with the owner's device: the team is left without an owner.
    {
        let [a, a2, ..] = &mut nodes;
        a.log.admit(team, a2.id(), Admin).unwrap();
        sync(a, a2);
        a.log.revoke(team, a2.id()).unwrap();
        a2.log.revoke(team, a.id()).unwrap();
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), [("b", Admin), ("c", Member)]);
    assert_eq!(owner(team, &nodes), None);
}

#[test]
fn a_revocation_beats_a_promotion_it_had_not_seen() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        r.log.revoke(team, r2.id()).unwrap();
        now(a.log.set_role(team, r2.id(), Admin)).unwrap();
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), START);

    // And a handover of the team to it.
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        now(a.log.add_member(team, r2.id(), Member)).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        r.log.revoke(team, r2.id()).unwrap();
        now(a.log.hand_over(team, r2.id())).unwrap();
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), START);
    assert_eq!(owner(team, &nodes), Some(nodes[0].id()));
}

/// An admin removed a device. What that device does to its user's other
/// device afterwards is nothing.
#[test]
fn a_removed_device_cannot_revoke_its_sibling() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        now(b.log.remove_member(team, r2.id())).unwrap();
        // Unseen by r2, which revokes r.
        r2.log.revoke(team, r.id()).unwrap();
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), START);
}

/// a vouched for a2 and a2 for a3. Taking a2 out leaves a3 in, and a3 is
/// still known as the user's.
#[test]
fn devices_a_revoked_device_vouched_for_stay() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        a.log.admit(team, a2.id(), Admin).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        a2.log.admit(team, a3.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        a.log.revoke(team, a2.id()).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [
            ("a", Owner),
            ("a3", Member),
            ("b", Admin),
            ("c", Member),
            ("r", Member)
        ]
    );
    let [a, _, a3, ..] = &mut nodes;
    assert_eq!(a.log.user(a3.id()), a.log.user(a.id()));
    // a3 can be taken out by a, and a by a3.
    a3.log.revoke(team, a.id()).unwrap();
    all(&mut nodes);
    assert_eq!(owner(team, &nodes), Some(nodes[2].id()));
}

/// A removal between two devices of one user is a revocation, whoever
/// wrote it as what.
#[test]
fn an_admins_removal_of_its_own_device_is_a_revocation() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        b.log.admit(team, b2.id(), Admin).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        // An admin cannot remove an admin, but it can its own device.
        assert_eq!(
            now(b.log.remove_member(team, b2.id())),
            Err(Error::NotAllowed)
        );
        let under = b.log.team(team).unwrap().admin_grant(b.id()).unwrap();
        b.forge(b.removal(team, b2.id(), under));
    }
    all(&mut nodes);
    assert_eq!(roles(team, &nodes), START);
}

/// The team was handed to r. r2, which knows of the handover but not that
/// r has accepted and acted, revokes r: the acceptance is cut off, what r
/// did as owner is void, and r2 is the owner.
#[test]
fn a_revocation_cuts_off_an_acceptance_it_had_not_seen() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        now(a.log.hand_over(team, r.id())).unwrap();
        sync(a, r);
        sync(a, r2);
        now(r.log.set_role(team, c.id(), Admin)).unwrap();
        r2.log.revoke(team, r.id()).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a", Admin), ("b", Admin), ("c", Member), ("r2", Owner)]
    );
    assert_eq!(owner(team, &nodes), Some(nodes[6].id()));
}

/// r was revoked, let in again and made the owner. The old revocation says
/// nothing about that.
#[test]
fn an_old_revocation_does_not_end_a_later_reign() {
    let (team, mut nodes) = team();
    {
        let [a, a2, a3, b, b2, r, r2, c] = &mut nodes;
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        r.log.revoke(team, r2.id()).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        r.log.admit(team, r2.id(), Member).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        now(a.log.hand_over(team, r2.id())).unwrap();
        settle(&mut [a, a2, a3, b, b2, r, r2, c]);
        now(r2.log.set_role(team, c.id(), Admin)).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [
            ("a", Admin),
            ("b", Admin),
            ("c", Admin),
            ("r", Member),
            ("r2", Owner)
        ]
    );
}

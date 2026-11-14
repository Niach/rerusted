//! The cases the rules were chosen for, one test each. Where a case needs a
//! device that does not follow the rules, its entries are forged: signed
//! with the device's own key, but written without asking the rules.

mod common;

use common::{Node, now, settle, sync};
use identity::Error;
use identity::format::Kind;
use identity::rules::KeyId;
use teamlog::Role::{Admin, Member, Owner};
use teamlog::{EntryId, Event, TeamId, TeamLog};

/// a owns the team, b and c are admins, d and e members. Everybody has
/// everything.
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

const START: [(&str, teamlog::Role); 5] = [
    ("a", Owner),
    ("b", Admin),
    ("c", Admin),
    ("d", Member),
    ("e", Member),
];

/// The operation `node` acts under as an admin.
fn admin_grant(node: &Node, team: TeamId) -> EntryId {
    node.log.team(team).unwrap().admin_grant(node.id()).unwrap()
}

/// Every node sees the same members, the same current key, and the same
/// payloads as void.
fn agree(team: TeamId, nodes: &[&Node]) -> Option<KeyId> {
    let first = nodes[0];
    for node in nodes {
        assert_eq!(
            now(node.log.members(team)).unwrap(),
            now(first.log.members(team)).unwrap(),
            "{}",
            node.name
        );
        assert_eq!(
            node.log.team(team).unwrap().current_key(),
            first.log.team(team).unwrap().current_key(),
            "{}",
            node.name
        );
        assert_eq!(node.log.void(team), first.log.void(team), "{}", node.name);
    }
    first.log.team(team).unwrap().current_key()
}

#[test]
fn two_admins_cannot_remove_each_other() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    // The rules refuse it where it is asked for.
    assert_eq!(
        now(b.log.remove_member(team, c.id())),
        Err(Error::NotAllowed)
    );
    assert_eq!(
        now(b.log.set_role(team, c.id(), Member)),
        Err(Error::NotAllowed)
    );

    // And where both write it anyway, at the same time.
    let by_b = b.forge(b.removal(team, c.id(), admin_grant(b, team)));
    let by_c = c.forge(c.removal(team, b.id(), admin_grant(c, team)));
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(node.roles(team, &all), START, "{}", node.name);
        assert!(!node.log.team(team).unwrap().counted(by_b.id));
        assert!(!node.log.team(team).unwrap().counted(by_c.id));
    }
    // The team works on: both can still act as admins, and the key is the
    // one everybody holds.
    assert!(agree(team, &all).is_some());
    now(b.log.remove_member(team, d.id())).unwrap();
    now(c.log.remove_member(team, e.id())).unwrap();
}

#[test]
fn the_owner_wins_against_an_admin() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    now(a.log.remove_member(team, b.id())).unwrap();
    let removal = b.forge(b.removal(team, a.id(), admin_grant(b, team)));
    let demotion = b.forge(b.setting(team, a.id(), Member, admin_grant(b, team)));
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(
            node.roles(team, &all),
            [("a", Owner), ("c", Admin), ("d", Member), ("e", Member)],
            "{}",
            node.name
        );
        assert!(!node.log.team(team).unwrap().counted(removal.id));
        assert!(!node.log.team(team).unwrap().counted(demotion.id));
    }
    assert!(b.events.contains(&Event::Ejected { team }));
    // The owner cannot be touched without a concurrent removal either.
    assert_eq!(
        now(c.log.remove_member(team, a.id())),
        Err(Error::NotAllowed)
    );
    let direct = c.forge(c.removal(team, a.id(), admin_grant(c, team)));
    settle(&mut [a, b, c, d, e]);
    assert_eq!(a.log.team(team).unwrap().role(a.id()), Some(Owner));
    assert!(!a.log.team(team).unwrap().counted(direct.id));
}

#[test]
fn only_the_owner_makes_admins() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    assert_eq!(
        now(b.log.set_role(team, d.id(), Admin)),
        Err(Error::NotAllowed)
    );
    let promotion = b.forge(b.setting(team, d.id(), Admin, admin_grant(b, team)));
    // A member writes operations as if it were an admin, under the
    // operation that admitted it and under the team's first.
    let grant = d.log.team(team).unwrap().member_grant(d.id()).unwrap();
    let as_member = d.forge(d.removal(team, e.id(), grant));
    let as_owner = d.forge(d.removal(team, e.id(), EntryId(team.0)));
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(node.roles(team, &all), START, "{}", node.name);
        for forged in [&promotion, &as_member, &as_owner] {
            assert!(!node.log.team(team).unwrap().counted(forged.id));
        }
    }
}

/// The owner demotes an admin who, not knowing, removes a member.
#[test]
fn a_demoted_admins_concurrent_removal_is_void() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    now(a.log.set_role(team, b.id(), Member)).unwrap();
    let removal = now(b.log.remove_member(team, d.id())).unwrap();
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(
            node.roles(team, &all),
            [
                ("a", Owner),
                ("b", Member),
                ("c", Admin),
                ("d", Member),
                ("e", Member)
            ],
            "{}",
            node.name
        );
        assert!(!node.log.team(team).unwrap().counted(removal[0].id));
    }
    // The key b made for the removal is nobody's current key: d, a member
    // all along, reads what is written next. (p2panda locked d out here in
    // 4 of 12 runs.)
    let key = agree(team, &all);
    c.write(team, "after");
    assert_eq!(c.log.team(team).unwrap().current_key(), key);
    settle(&mut [a, b, c, d, e]);
    for node in [&*a, &*b, &*c, &*d, &*e] {
        assert_eq!(node.texts(team), ["after"], "{}", node.name);
    }
}

/// An admin admits a new device and hands it the team's keys; the owner,
/// not knowing, demotes that admin. The admission is void, but the device
/// has the keys.
#[test]
fn a_void_admission_still_spends_the_key() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    let mut f = Node::new("f");
    sync(&mut f, b);

    now(a.log.set_role(team, b.id(), Member)).unwrap();
    let admission = now(b.log.add_member(team, f.id(), Member)).unwrap();
    settle(&mut [a, b, c, d, e, &mut f]);

    let all = [&*a, &*b, &*c, &*d, &*e, &f];
    for node in all {
        assert!(!node.log.team(team).unwrap().counted(admission[0].id));
        assert_eq!(node.log.team(team).unwrap().role(f.id()), None);
    }
    assert_eq!(agree(team, &all), None);
    assert_eq!(c.write(team, "not for f").len(), 2);
    settle(&mut [a, b, c, d, e, &mut f]);
    for node in [&*a, &*b, &*c, &*d, &*e] {
        assert_eq!(node.texts(team), ["not for f"], "{}", node.name);
    }
    assert!(f.texts(team).is_empty());
}

/// Two admins remove different members at the same time. Both removals
/// stand, and neither's key is held by exactly the members that are left.
#[test]
fn concurrent_removals_end_in_one_new_key() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    now(b.log.remove_member(team, d.id())).unwrap();
    now(c.log.remove_member(team, e.id())).unwrap();
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(
            node.roles(team, &all),
            [("a", Owner), ("b", Admin), ("c", Admin)]
        );
    }
    assert_eq!(agree(team, &all), None);

    // Two members write before they hear of each other: each makes a key.
    assert_eq!(b.write(team, "from b").len(), 2);
    assert_eq!(c.write(team, "from c").len(), 2);
    settle(&mut [a, b, c, d, e]);
    let all = [&*a, &*b, &*c, &*d, &*e];
    assert!(agree(team, &all).is_some());
    // With one current key again, a write is one entry.
    assert_eq!(a.write(team, "from a").len(), 1);
    settle(&mut [a, b, c, d, e]);
    for node in [&*a, &*b, &*c] {
        assert_eq!(node.texts(team), ["from a", "from b", "from c"]);
    }
    // The removed hold the entries and read none of them.
    for node in [&*d, &*e] {
        assert!(node.texts(team).is_empty(), "{}", node.name);
    }
}

/// c removes d, b admits d again, and a third admin who has seen neither
/// removes d as well.
#[test]
fn a_removal_beats_an_admission_it_had_not_seen() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    now(a.log.set_role(team, e.id(), Admin)).unwrap();
    settle(&mut [a, b, c, d, e]);

    now(c.log.remove_member(team, d.id())).unwrap();
    sync(c, b);
    let admission = now(b.log.add_member(team, d.id(), Member)).unwrap();
    now(e.log.remove_member(team, d.id())).unwrap();
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(
            node.roles(team, &all),
            [("a", Owner), ("b", Admin), ("c", Admin), ("e", Admin)],
            "{}",
            node.name
        );
        assert!(!node.log.team(team).unwrap().counted(admission[0].id));
    }
    agree(team, &all);
}

/// The owner promotes a member while an admin removes it.
#[test]
fn the_owner_overrules_an_admins_concurrent_operation() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    let removal = now(b.log.remove_member(team, d.id())).unwrap();
    now(a.log.set_role(team, d.id(), Admin)).unwrap();
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(
            node.roles(team, &all),
            [
                ("a", Owner),
                ("b", Admin),
                ("c", Admin),
                ("d", Admin),
                ("e", Member)
            ],
            "{}",
            node.name
        );
        assert!(!node.log.team(team).unwrap().counted(removal[0].id));
    }
    assert!(agree(team, &all).is_some());
}

/// A removed admin signs operations afterwards and claims they are old.
#[test]
fn a_removed_admin_cannot_backdate() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    let old_grant = admin_grant(b, team);
    let old_heads = b.log.team(team).unwrap().heads().to_vec();
    let old_member_grant = b.log.team(team).unwrap().member_grant(b.id()).unwrap();

    now(a.log.remove_member(team, b.id())).unwrap();
    settle(&mut [a, b, c, d, e]);

    // b knows it is out, and writes as if it had acted before: parents are
    // the heads from before its removal.
    let removal = b.forge(Kind::Remove {
        team,
        under: old_grant,
        parents: old_heads.clone(),
        member: d.id(),
        cut: 0,
        key: None,
        boxes: Vec::new(),
    });
    let rotation = b.forge(Kind::Rotate {
        team,
        under: old_member_grant,
        parents: old_heads,
        key: KeyId([7; 32]),
        boxes: Vec::new(),
    });
    let payload = b.forge(Kind::Payload {
        team,
        under: old_member_grant,
        key: KeyId([7; 32]),
        nonce: [0; 24],
        text: vec![0; 32],
    });
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    for node in all {
        assert_eq!(
            node.roles(team, &all),
            [("a", Owner), ("c", Admin), ("d", Member), ("e", Member)],
            "{}",
            node.name
        );
        let resolved = node.log.team(team).unwrap();
        assert!(!resolved.counted(removal.id));
        assert!(!resolved.counted(rotation.id));
        assert_eq!(node.log.void(team), [(b.id(), payload.seq)]);
    }
    assert!(agree(team, &all).is_some());
}

/// A member writes while a removal is on its way: what the remover had not
/// seen is void for everybody, and stays void when the member is admitted
/// again before it ever synced.
#[test]
fn writes_the_removal_had_not_seen_are_dropped() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;

    d.write(team, "seen");
    settle(&mut [a, b, c, d, e]);
    d.write(team, "unseen");
    sync(d, e);
    assert_eq!(e.texts(team), ["seen", "unseen"]);

    now(a.log.remove_member(team, d.id())).unwrap();
    now(a.log.add_member(team, d.id(), Member)).unwrap();
    settle(&mut [a, b, c, d, e]);
    d.write(team, "again");
    settle(&mut [a, b, c, d, e]);

    let all = [&*a, &*b, &*c, &*d, &*e];
    agree(team, &all);
    for node in all {
        assert_eq!(node.roles(team, &all), START, "{}", node.name);
        assert_eq!(node.texts(team), ["again", "seen"], "{}", node.name);
        assert_eq!(node.log.void(team).len(), 1);
    }
    // e had shown the payload and is told to take it back; so is d.
    for node in [&*d, &*e] {
        assert!(node.events.iter().any(|event| matches!(
            event,
            Event::Dropped { payload, .. } if payload.data == b"unseen"
        )));
    }
    assert!(d.events.contains(&Event::Ejected { team }));
}

/// A member that is admitted again reads what it could before and what is
/// written from then on, not what was written while it was out. A device
/// that is new to the team reads everything.
#[test]
fn a_readmitted_member_does_not_read_the_gap() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    let mut f = Node::new("f");
    sync(&mut f, b);

    c.write(team, "before");
    settle(&mut [a, b, c, d, e]);
    now(a.log.remove_member(team, d.id())).unwrap();
    settle(&mut [a, b, c, d, e]);
    c.write(team, "gap");
    settle(&mut [a, b, c, d, e]);
    assert_eq!(d.texts(team), ["before"]);
    assert_eq!(now(d.log.write(team, b"out")), Err(Error::NotMember));

    now(a.log.add_member(team, d.id(), Member)).unwrap();
    settle(&mut [a, b, c, d, e]);
    c.write(team, "after");
    d.write(team, "back");
    now(b.log.add_member(team, f.id(), Member)).unwrap();
    settle(&mut [a, b, c, d, e, &mut f]);

    assert_eq!(d.texts(team), ["after", "back", "before"]);
    for node in [&*a, &*b, &*c, &*e, &f] {
        assert_eq!(
            node.texts(team),
            ["after", "back", "before", "gap"],
            "{}",
            node.name
        );
    }
}

/// Everything but the seed comes back from the entries.
#[test]
fn a_replica_is_restored_from_its_seed_and_entries() {
    let (team, mut nodes) = team();
    let [a, b, c, d, e] = &mut nodes;
    c.write(team, "one");
    now(b.log.remove_member(team, d.id())).unwrap();
    now(c.log.remove_member(team, e.id())).unwrap();
    settle(&mut [a, b, c, d, e]);
    a.write(team, "two");
    settle(&mut [a, b, c, d, e]);

    for node in [&*a, &*b, &*c, &*d, &*e] {
        let restored = identity::Replica::restore(node.log.seed(), node.log.entries()).unwrap();
        assert_eq!(
            now(restored.members(team)).unwrap(),
            now(node.log.members(team)).unwrap()
        );
        assert_eq!(
            now(restored.history(team)).unwrap(),
            now(node.log.history(team)).unwrap()
        );
        assert_eq!(
            restored.team(team).unwrap().current_key(),
            node.log.team(team).unwrap().current_key()
        );
    }
}

//! Invites (#RUST-44): a one-time secret, an expiry, the team and the
//! inviter; nobody is in before an admin's entry says so.

mod common;

use common::{Node, now, settle, sync};
use identity::format::{self, Body, Kind, Lockbox};
use identity::invite::{INVITE_LEN, Invite, REQUEST_LEN, Request};
use identity::rules::{KeyId, Parent};
use identity::{Error, Replica, keys};
use teamlog::Role::{Admin, Member, Owner};
use teamlog::{DeviceId, EntryId, TeamId, TeamLog};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
fn the_invite_the_request_and_the_entries_are_those_of_the_vectors() {
    let vectors: Vec<Vec<&str>> = include_str!("vectors/invite-v1.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.split('|').map(str::trim).collect())
        .collect();
    assert_eq!(vectors.len(), 4);
    let (team, device) = (TeamId([1; 32]), DeviceId([6; 32]));
    let secret: [u8; 32] = std::array::from_fn(|i| 64 + i as u8);
    let invite = Invite::from_secret(team, DeviceId([9; 32]), 1_795_478_400, secret);

    assert_eq!(vectors[0][0], "invite");
    assert_eq!(hex(&invite.encode()), vectors[0][1]);
    assert_eq!(invite.encode().len(), INVITE_LEN);
    assert_eq!(hex(&invite.key()), vectors[0][2]);
    assert_eq!(Invite::decode(&unhex(vectors[0][1])), Ok(invite.clone()));

    assert_eq!(vectors[1][0], "request");
    let request = invite.request(device);
    assert_eq!(hex(&request), vectors[1][1]);
    assert_eq!(request.len(), REQUEST_LEN);
    let opened = Request::open(&request).unwrap();
    assert_eq!(
        (opened.team, opened.invite, opened.device),
        (team, invite.key(), device)
    );

    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let mut replica = Replica::from_seed(seed);
    let announce = now(replica.announce()).unwrap().remove(0);
    let under = EntryId([2; 32]);
    let parents = vec![Parent {
        device: DeviceId([3; 32]),
        seq: 4,
        id: EntryId([5; 32]),
    }];
    let kinds = [
        Kind::Invite {
            team,
            under,
            parents: parents.clone(),
            invite: invite.key(),
            expires: invite.expires,
        },
        Kind::Join {
            team,
            under,
            parents,
            member: device,
            cut: 7,
            invite: invite.key(),
            asked: opened.signature,
            key: Some(KeyId([8; 32])),
            boxes: vec![Lockbox {
                key: KeyId([8; 32]),
                device,
                sealed: vec![0xaa, 0xbb, 0xcc],
            }],
        },
    ];
    for (kind, vector) in kinds.into_iter().zip(&vectors[2..]) {
        let body = Body {
            author: announce.author,
            seq: 1,
            prev: announce.id,
            kind,
        };
        let bytes = format::seal(&body, &keys::signing_key(&seed));
        assert_eq!(hex(&bytes), vector[1], "{}", vector[0]);
        assert_eq!(hex(&format::id(&bytes).0), vector[2], "{}", vector[0]);
        assert_eq!(format::open(&bytes), Ok(body), "{}", vector[0]);
    }
}

#[test]
fn a_changed_invite_or_request_is_refused() {
    let invite = Invite::new(TeamId([1; 32]), DeviceId([2; 32]), 10);
    let bytes = invite.encode();
    let mut newer = bytes.clone();
    newer[0] += 1;
    assert_eq!(Invite::decode(&newer), Err(Error::UnsupportedVersion(2)));
    assert!(Invite::decode(&bytes[1..]).is_err());
    assert!(Invite::decode(&[bytes.clone(), vec![0]].concat()).is_err());
    assert!(Invite::decode(&[]).is_err());
    // The secret does not show.
    assert!(!format!("{invite:?}").contains("secret"));

    let request = invite.request(DeviceId([3; 32]));
    let mut newer = request.clone();
    newer[0] += 1;
    assert_eq!(Request::open(&newer), Err(Error::UnsupportedVersion(2)));
    for position in 1..request.len() {
        let mut tampered = request.clone();
        tampered[position] ^= 1;
        assert!(Request::open(&tampered).is_err(), "{position}");
    }
    assert!(Request::open(&request[1..]).is_err());
    assert!(Request::open(&[]).is_err());
}

/// a owns the team, b is an admin, c a member; x and y are in no team and
/// have announced themselves to everybody.
fn team() -> (TeamId, [Node; 5]) {
    let mut nodes = ["a", "b", "c", "x", "y"].map(Node::new);
    {
        let [a, b, c, x, y] = &mut nodes;
        settle(&mut [a, b, c, x, y]);
    }
    let [a, b, c, x, y] = &mut nodes;
    let (team, _) = now(a.log.create_team()).unwrap();
    now(a.log.add_member(team, b.id(), Admin)).unwrap();
    now(a.log.add_member(team, c.id(), Member)).unwrap();
    settle(&mut [a, b, c, x, y]);
    (team, nodes)
}

fn all(nodes: &mut [Node; 5]) {
    let [a, b, c, x, y] = nodes;
    settle(&mut [a, b, c, x, y]);
}

fn roles(team: TeamId, nodes: &[Node; 5]) -> Vec<(&'static str, teamlog::Role)> {
    let refs: Vec<&Node> = nodes.iter().collect();
    let first = nodes[0].roles(team, &refs);
    for node in nodes {
        assert_eq!(node.roles(team, &refs), first, "{}", node.name);
    }
    first
}

const START: [(&str, teamlog::Role); 3] = [("a", Owner), ("b", Admin), ("c", Member)];
const NOW: u64 = 1_795_000_000;

#[test]
fn an_invited_device_is_in_once_the_inviter_has_written_it() {
    let (team, mut nodes) = team();
    {
        let [a, b, c, x, y] = &mut nodes;
        a.write(team, "before");
        let (invite, _) = b.log.invite(team, NOW + 60).unwrap();
        assert_eq!((invite.team, invite.inviter), (team, b.id()));
        // The invite travels outside the log, as bytes.
        let invite = Invite::decode(&invite.encode()).unwrap();
        let request = invite.request(x.id());

        // Asking is not being in.
        settle(&mut [a, b, c, x, y]);
        assert_eq!(c.roles(team, &[&*a, &*b, &*c, &*x, &*y]), START);
        assert_eq!(now(x.log.write(team, b"x")), Err(Error::NotMember));

        // The request goes to the inviter, not to any admin.
        assert_eq!(a.log.join(&request, NOW), Err(Error::UnknownInvite));
        b.log.join(&request, NOW).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a", Owner), ("b", Admin), ("c", Member), ("x", Member)]
    );
    // It reads what was written before and writes.
    let [_, _, c, x, _] = &mut nodes;
    assert_eq!(x.texts(team), ["before"]);
    x.write(team, "hello");
    sync(x, c);
    assert_eq!(c.texts(team), ["before", "hello"]);
}

#[test]
fn an_invite_works_once_and_not_after_its_time() {
    let (team, mut nodes) = team();
    {
        let [a, b, c, x, y] = &mut nodes;
        let (invite, _) = b.log.invite(team, NOW + 60).unwrap();
        let (late, _) = b.log.invite(team, NOW).unwrap();

        // Out of time: the first second it no longer works in is `expires`.
        assert_eq!(b.log.join(&late.request(x.id()), NOW), Err(Error::Expired));
        assert!(b.log.join(&late.request(x.id()), NOW - 1).is_ok());
        settle(&mut [a, b, c, x, y]);

        // Once: not for a second device, not for the same one again.
        assert_eq!(b.log.join(&late.request(y.id()), NOW - 1), Err(Error::Used));
        b.log.join(&invite.request(y.id()), NOW).unwrap();
        assert_eq!(b.log.join(&invite.request(y.id()), NOW), Err(Error::Used));
        assert_eq!(b.log.join(&invite.request(x.id()), NOW), Err(Error::Used));

        // An invite nobody wrote into a log.
        let made_up = Invite::new(team, b.id(), NOW + 60);
        assert_eq!(
            b.log.join(&made_up.request(y.id()), NOW),
            Err(Error::UnknownInvite)
        );
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [
            ("a", Owner),
            ("b", Admin),
            ("c", Member),
            ("x", Member),
            ("y", Member)
        ]
    );
}

#[test]
fn only_an_admin_invites() {
    let (team, mut nodes) = team();
    let [_, _, c, x, _] = &mut nodes;
    assert_eq!(c.log.invite(team, NOW + 60).err(), Some(Error::NotAllowed));
    assert_eq!(x.log.invite(team, NOW + 60).err(), Some(Error::NotAllowed));
}

/// Entries written without asking the rules: none of them lets anybody in.
#[test]
fn a_join_that_does_not_hold_lets_nobody_in() {
    let (team, mut nodes) = team();
    {
        let [a, b, c, x, y] = &mut nodes;
        let join = |node: &Node, invite: &Invite, member: DeviceId, asked: DeviceId| {
            let resolved = node.log.team(team).unwrap();
            let request = Request::open(&invite.request(asked)).unwrap();
            Kind::Join {
                team,
                under: resolved
                    .admin_grant(node.id())
                    .or(resolved.member_grant(node.id()))
                    .unwrap(),
                parents: resolved.parents().to_vec(),
                member,
                cut: node.height_of(member),
                invite: invite.key(),
                asked: request.signature,
                key: None,
                boxes: Vec::new(),
            }
        };
        let note = |node: &Node, invite: &Invite| {
            let resolved = node.log.team(team).unwrap();
            Kind::Invite {
                team,
                under: resolved.member_grant(node.id()).unwrap(),
                parents: resolved.parents().to_vec(),
                invite: invite.key(),
                expires: invite.expires,
            }
        };

        // A member writes an invite and the join for it.
        let own = Invite::new(team, c.id(), NOW + 60);
        c.forge(note(c, &own));
        c.forge(join(c, &own, x.id(), x.id()));

        // An admin admits with an invite it never wrote into its log,
        let made_up = Invite::new(team, b.id(), NOW + 60);
        b.forge(join(b, &made_up, x.id(), x.id()));
        // with another admin's invite,
        let (others, _) = a.log.invite(team, NOW + 60).unwrap();
        sync(a, b);
        b.forge(join(b, &others, x.id(), x.id()));
        // and another device than the one that asked.
        let (invite, _) = b.log.invite(team, NOW + 60).unwrap();
        b.forge(join(b, &invite, y.id(), x.id()));
        settle(&mut [a, b, c, x, y]);
        assert_eq!(c.roles(team, &[&*a, &*b, &*c, &*x, &*y]), START);

        // None of that used the invite up: the device that asked gets in,
        // and a second join with the same invite, written anyway, does not.
        b.log.join(&invite.request(x.id()), NOW).unwrap();
        b.forge(join(b, &invite, y.id(), y.id()));
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a", Owner), ("b", Admin), ("c", Member), ("x", Member)]
    );
}

/// The inviter was demoted before it wrote the join: the join is an
/// admission by a member, and counts for nothing.
#[test]
fn a_join_counts_only_as_an_admins_entry() {
    let (team, mut nodes) = team();
    {
        let [a, b, c, x, y] = &mut nodes;
        let (invite, _) = b.log.invite(team, NOW + 60).unwrap();
        settle(&mut [a, b, c, x, y]);
        now(a.log.set_role(team, b.id(), Member)).unwrap();
        // b has not heard of it and lets x in.
        b.log.join(&invite.request(x.id()), NOW).unwrap();
    }
    all(&mut nodes);
    assert_eq!(
        roles(team, &nodes),
        [("a", Owner), ("b", Member), ("c", Member)]
    );
    let [_, b, _, x, _] = &mut nodes;
    let invite = Invite::new(team, b.id(), NOW + 60);
    assert_eq!(
        b.log.join(&invite.request(x.id()), NOW),
        Err(Error::UnknownInvite)
    );
    assert_eq!(b.log.invite(team, NOW + 60).err(), Some(Error::NotAllowed));
}

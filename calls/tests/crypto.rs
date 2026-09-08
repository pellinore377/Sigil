use sigil_calls::{Context, Error, MediaKind, Member, Receiver, Roster, Sender, SignedRoster};
use sigil_crypto::IdentityKey;
fn context() -> Context {
    Context {
        call: [1; 32],
        roster: [2; 32],
        sender: [3; 32],
        incarnation: [4; 32],
    }
}
#[test]
fn media_authenticates_context_metadata_and_every_wire_byte_before_replay_commit() {
    let (mut sender, share) = Sender::generate(context()).unwrap();
    let a = sender
        .seal(MediaKind::Audio, 100, false, b"synthetic audio")
        .unwrap();
    let b = sender
        .seal(MediaKind::Camera, 200, true, b"synthetic camera")
        .unwrap();
    let c = sender
        .seal(MediaKind::Screen, 300, true, b"synthetic screen")
        .unwrap();
    for index in 0..b.len() {
        let mut bad = b.clone();
        bad[index] ^= 1;
        let mut receiver = Receiver::new(&share).unwrap();
        assert!(receiver.open(&bad).is_err(), "wire byte {index}");
        assert_eq!(&*receiver.open(&b).unwrap().data, b"synthetic camera");
    }
    let mut receiver = Receiver::new(&share).unwrap();
    for (bytes, kind, timestamp) in [
        (&c, MediaKind::Screen, 300),
        (&a, MediaKind::Audio, 100),
        (&b, MediaKind::Camera, 200),
    ] {
        let frame = receiver.open(bytes).unwrap();
        assert_eq!(frame.kind, kind);
        assert_eq!(frame.timestamp, timestamp);
        assert!(matches!(receiver.open(bytes), Err(Error::Replay)));
    }
    let mut changed = share.clone();
    changed.context.roster = [8; 32];
    assert!(Receiver::new(&changed).unwrap().open(&a).is_err());
    let (_, wrong) = Sender::generate(context()).unwrap();
    assert!(Receiver::new(&wrong).unwrap().open(&a).is_err());
    assert!(sender
        .seal(MediaKind::Audio, 1, true, b"invalid flags")
        .is_err());
}
#[test]
fn roster_signatures_bind_membership_and_cannot_change_call_identity_or_move_backwards() {
    let owner = IdentityKey::generate().unwrap();
    let member = Member::new(owner.public_key());
    let first = Roster {
        version: 1,
        call: [1; 32],
        server: "chat.example".into(),
        owner: owner.public_key(),
        created: 1000,
        expires: 2000,
        revision: 0,
        previous: None,
        members: vec![member],
        closed: false,
    }
    .sign(&owner)
    .unwrap();
    assert!(SignedRoster::from_bytes(&first.to_bytes().unwrap()).unwrap() == first);
    let added = IdentityKey::generate().unwrap();
    let mut next = first.roster.clone();
    next.revision = 1;
    next.previous = Some(first.roster.digest().unwrap());
    next.members.push(Member::new(added.public_key()));
    next.members.sort_by_key(|m| m.id);
    let next = next.sign(&owner).unwrap();
    first.roster.successor(&next.roster, true).unwrap();
    let mut forged = next.clone();
    forged.roster.members.pop();
    assert!(forged.verify().is_err());
    let mut changed = next.roster.clone();
    changed.server = "other.example".into();
    assert!(first.roster.successor(&changed, true).is_err());
    assert!(next.roster.successor(&first.roster, true).is_err());
    assert!(first.roster.active(2000).is_err());
    let mut closed = next.roster.clone();
    closed.closed = true;
    closed.revision += 1;
    closed.previous = Some(next.roster.digest().unwrap());
    next.roster.successor(&closed, true).unwrap();
    assert!(closed.active(1500).is_err());
    assert!(closed.successor(&next.roster, false).is_err());
}

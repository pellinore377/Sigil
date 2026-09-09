use sigil_calls::*;
use sigil_crypto::IdentityKey;
use sigil_protocol::device::{Binding, SignedBinding};

fn state(version: u8) -> (State, Vec<IdentityKey>) {
    let keys: Vec<_> = (0..8).map(|_| IdentityKey::generate().unwrap()).collect();
    let server = [
        "z".repeat(63),
        "z".repeat(63),
        "z".repeat(63),
        "z".repeat(61),
    ]
    .join(".");
    let mut roster = Roster {
        controller: (version == 2).then(|| keys[0].public_key()),
        version,
        call: [1; 32],
        server: server.clone(),
        owner: keys[0].public_key(),
        created: 1000,
        expires: 2000,
        revision: 0,
        previous: None,
        members: keys.iter().map(|k| Member::new(k.public_key())).collect(),
        closed: false,
    };
    roster.members.sort_by_key(|m| m.id);
    let mut participants = Vec::new();
    let mut ready = Vec::new();
    for (n, key) in keys.iter().enumerate() {
        let identity = IdentityKey::generate().unwrap();
        let binding = Binding {
            server: server.clone(),
            username: "z".repeat(32),
            account: [255; 32],
            device: [255 - n as u8; 32],
            identity: identity.public_key(),
        };
        let device = SignedBinding {
            signature: identity.sign(&binding.signing_bytes().unwrap()).unwrap(),
            binding,
        }
        .to_bytes()
        .unwrap();
        participants.push(
            Attestation::sign(&roster, Member::new(key.public_key()), device, &identity).unwrap(),
        );
        ready.push(
            Ready::sign(
                &roster,
                1,
                [n as u8 + 1; 32],
                Tracks {
                    audio: true,
                    camera: true,
                    screen: true,
                },
                key,
            )
            .unwrap(),
        );
    }
    participants.sort_by_key(|p| p.member.id);
    ready.sort_by_key(|r| r.member);
    let state = State {
        roster: roster.sign(&keys[0]).unwrap(),
        epoch: 0,
        participants,
        ready,
        signature: Vec::new(),
    }
    .sign(&keys[0])
    .unwrap();
    (state, keys)
}
#[test]
fn eight_maximum_addresses_fit_and_device_ready_share_authority_is_separate() {
    let (state, keys) = state(1);
    assert!(serde_json::to_vec(&state).unwrap().len() < 49152);
    let mut forged = state.clone();
    forged.participants[0].device[100] ^= 1;
    assert!(forged.sign(&keys[0]).is_err());
    let target = Member::new(keys[1].public_key()).id;
    let mut forged = state.clone();
    forged
        .ready
        .iter_mut()
        .find(|r| r.member == target)
        .unwrap()
        .tracks
        .camera = false;
    assert!(forged.sign(&keys[0]).is_err());
    let (_, media) = Sender::generate(Context {
        call: state.roster.roster.call,
        roster: state.roster.roster.digest().unwrap(),
        sender: target,
        incarnation: [3; 32],
    })
    .unwrap();
    assert!(Share::sign(&state, 1, media.clone(), &keys[0]).is_err());
    let share = Share::sign(&state, 1, media, &keys[1]).unwrap();
    share.verify(&state).unwrap();
    let mut next = state.clone();
    next.epoch += 1;
    let ready = next.ready.iter_mut().find(|r| r.member == target).unwrap();
    *ready = Ready::sign(&state.roster.roster, 2, [9; 32], ready.tracks, &keys[1]).unwrap();
    let next = next.sign(&keys[0]).unwrap();
    state.successor(&next).unwrap();
    assert!(share.verify(&next).is_err());
    let mut rollback = state.clone();
    rollback.epoch = next.epoch + 1;
    assert!(next.successor(&rollback.sign(&keys[0]).unwrap()).is_err());
}
#[test]
fn controller_handoff_preserves_device_attestations_and_excludes_departed_media() {
    let (state, keys) = state(2);
    let sender = Member::new(keys[1].public_key()).id;
    let (_, old_key) = Sender::generate(Context {
        call: state.roster.roster.call,
        roster: state.roster.roster.digest().unwrap(),
        sender,
        incarnation: [1; 32],
    })
    .unwrap();
    let old_share = Share::sign(&state, 1, old_key, &keys[1]).unwrap();
    let mut current = state;
    for index in 1..8 {
        let delegation = current
            .roster
            .delegate(keys[index].public_key(), &keys[index - 1])
            .unwrap();
        let roster = current.roster.transfer(delegation, &keys[index]).unwrap();
        let mut next = current.clone();
        next.roster = roster;
        next.epoch += 1;
        next.participants
            .retain(|p| next.roster.roster.member(p.member.id).is_ok());
        next.ready
            .retain(|p| next.roster.roster.member(p.member).is_ok());
        assert!(next.clone().sign(&keys[index - 1]).is_err());
        let next = next.sign(&keys[index]).unwrap();
        current.successor(&next).unwrap();
        assert_eq!(next.participants.len(), 8 - index);
        assert!(next
            .roster
            .roster
            .member(Member::new(keys[index - 1].public_key()).id)
            .is_err());
        assert!(old_share.verify(&next).is_err());
        assert!(serde_json::to_vec(&next).unwrap().len() < 49152);
        current = next;
    }
    let mut closed = current.roster.roster.clone();
    closed.previous = Some(closed.digest().unwrap());
    closed.revision += 1;
    closed.closed = true;
    let closed = current.roster.update(closed, &keys[7]).unwrap();
    current.roster.successor(&closed, true).unwrap();
    assert!(closed.delegate(keys[0].public_key(), &keys[7]).is_err());
}
#[test]
fn handoff_rejects_forgery_replay_cross_call_and_legacy_downgrade() {
    let (state, keys) = state(2);
    let roster = state.roster;
    let proof = roster.delegate(keys[1].public_key(), &keys[0]).unwrap();
    for index in 0..proof.signature.len() {
        let mut changed = proof.clone();
        changed.signature[index] ^= 1;
        assert!(roster.transfer(changed, &keys[1]).is_err());
    }
    let mut changed = proof.clone();
    changed.controller = keys[2].public_key();
    assert!(roster.transfer(changed, &keys[2]).is_err());
    let outsider = IdentityKey::generate().unwrap();
    assert!(roster.delegate(outsider.public_key(), &keys[0]).is_err());
    assert!(roster.delegate(keys[1].public_key(), &keys[2]).is_err());
    assert!(roster.transfer(proof.clone(), &keys[2]).is_err());
    let mut other = roster.roster.clone();
    other.call = [2; 32];
    let other = other.sign(&keys[0]).unwrap();
    assert!(other.transfer(proof.clone(), &keys[1]).is_err());
    let next = roster.transfer(proof.clone(), &keys[1]).unwrap();
    assert!(next.transfer(proof, &keys[1]).is_err());
    assert!(SignedRoster::from_bytes(&next.to_bytes().unwrap()).unwrap() == next);
    let mut unsigned = next.clone();
    unsigned.delegations.clear();
    assert!(unsigned.verify().is_err());
    let mut legacy = roster.roster.clone();
    legacy.version = 1;
    legacy.controller = None;
    let legacy = legacy.sign(&keys[0]).unwrap();
    assert!(legacy.delegate(keys[1].public_key(), &keys[0]).is_err());
    assert!(legacy.successor(&next, true).is_err());
}

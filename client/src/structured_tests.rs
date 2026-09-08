use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, trust},
};
use sigil_crypto::Secret32;
use sigil_protocol::text::{parse_card, Origin, Parsed};
const CONVERSATION: Id = [81; 32];
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn card(client: &ClientStore, source: &str, now: u64) -> Card {
    let Parsed::Card(card) = parse_card(
        source,
        Origin {
            message: [82; 32],
            creator: client.account_reference().unwrap(),
            created_at: now,
            timezone: None,
        },
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!()
    };
    *card
}
fn poll(client: &ClientStore, now: u64) -> Card {
    card(client, "poll::closed::Choose\n- One\n- Two;", now)
}

#[test]
fn erased_card_and_action_caches_do_not_reappear_on_replay_without_recovery() {
    use sigil_protocol::conversation::{Action as CAction, Body, Reference as CReference};
    let (_dir, _fixture, mut a, _b, now) = pair();
    let card = poll(&a, now);
    let post = a
        .conversation_operation(
            card.id,
            CAction::Post {
                body: Body::Rich(card.to_bytes().unwrap()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    let conversation = a.note_to_self(&post, now, now).unwrap();
    let vote = action(&a, &card, None, 0, now);
    let post_vote = a
        .conversation_operation(
            vote.id().unwrap(),
            CAction::Post {
                body: Body::Rich(vote.to_bytes().unwrap()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    a.note_to_self(&post_vote, now, now).unwrap();
    let delete = a
        .conversation_operation(
            [245; 32],
            CAction::Delete {
                target: CReference {
                    author: card.creator,
                    message: card.id,
                },
            },
        )
        .unwrap();
    a.note_to_self(&delete, now, now).unwrap();
    a.maintain_history(now).unwrap();
    a.erase_obsolete_journals(now).unwrap();
    assert_eq!(
        a.db.query_row("SELECT count(*) FROM structured_actions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(matches!(
        a.card_state(conversation, Reference::of(&card).unwrap()),
        Err(Error::Obsolete)
    ));
    a.note_to_self(&post, now, now).unwrap();
    assert!(matches!(
        a.card_state(conversation, Reference::of(&card).unwrap()),
        Err(Error::Obsolete)
    ));
}
fn action(
    client: &ClientStore,
    card: &Card,
    previous: Option<Id>,
    choice: usize,
    now: u64,
) -> Action {
    let Construct::Poll(poll) = &card.content else {
        panic!()
    };
    Action {
        card: Reference::of(card).unwrap(),
        actor: client.account_reference().unwrap(),
        created_at: now,
        previous,
        revision: None,
        change: Change::Vote {
            choices: vec![poll.options[choice].id],
        },
    }
}
fn ingest_bytes(client: &mut ClientStore, record: Id, bytes: &[u8]) {
    let scope = account_context(&client.db, &client.key).unwrap().0;
    let tx = client
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    ingest(&tx, &client.key, scope, CONVERSATION, record, bytes).unwrap();
    tx.commit().unwrap();
}
fn insert_card(client: &mut ClientStore, card: &Card) {
    ingest_bytes(client, [83; 32], &card.to_bytes().unwrap())
}
fn insert(client: &mut ClientStore, action: &Action) {
    ingest_bytes(client, action.id().unwrap(), &action.to_bytes().unwrap())
}
fn drain(client: &mut ClientStore) {
    for _ in 0..1000 {
        let count = client.advance_structured_actions().unwrap();
        assert!(count <= BATCH);
        if count == 0 {
            return;
        }
    }
    panic!("structured work did not settle")
}
fn configure(client: &mut ClientStore, secret: u8) {
    let session = client.connection_session().unwrap().unwrap();
    client
        .configure_recovery(
            "chat.example",
            connection::decode_id(&session.account_id).unwrap(),
            Secret32::from_bytes([secret; 32]),
        )
        .unwrap();
}
fn flush(client: &mut ClientStore, now: u64) -> Id {
    for _ in 0..3 {
        if let Some(attempt) = client.resume_send_intents_online(now).unwrap().pop() {
            let session = attempt.result.unwrap();
            client.send_pending_online(session, now).unwrap();
            return session;
        }
    }
    panic!("no send intent")
}
#[test]
fn live_location_stop_survives_restart_and_delayed_encrypted_updates() {
    use sigil_protocol::text::{
        location::{Duration, Point},
        service::Coordinates,
        Text,
    };
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let point = |at| Point {
        coordinates: Coordinates {
            latitude_e6: 10_000_000,
            longitude_e6: -20_000_000,
        },
        accuracy_cm: Some(250),
        sampled_at: at,
    };
    let card = alice
        .location_card(
            [179; 32],
            LocationKind::Live(Duration::FifteenMinutes),
            point(now),
            Text::plain("Synthetic", Default::default()).unwrap(),
            now,
        )
        .unwrap();
    let reference = Reference::of(&card).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    assert_eq!(alice.location_jobs(None, now).unwrap().jobs.len(), 1);
    assert!(bob.location_jobs(None, now).unwrap().jobs.is_empty());
    let mut delayed = Vec::new();
    for elapsed in [10, 20] {
        let update = alice
            .update_location(conversation, reference, point(now + elapsed), now + elapsed)
            .unwrap();
        alice.queue_peer_action(b, &update, now + elapsed).unwrap();
        flush(&mut alice, now + elapsed);
        let delivery = bob
            .connected_client()
            .unwrap()
            .mailbox()
            .unwrap()
            .into_iter()
            .find(|v| v.message_id == crate::transport::hex(&update.id().unwrap()))
            .unwrap();
        delayed.push(delivery);
    }
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON location_jobs BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.stop_location(conversation, reference, now + 21),
        Err(Error::Storage(_))
    ));
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(alice.location_jobs(None, now + 21).unwrap().jobs.len(), 1);
    let stop = alice
        .stop_location(conversation, reference, now + 21)
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let batch = alice.location_jobs(None, now + 21).unwrap();
    assert!(batch.jobs.is_empty());
    assert_eq!(batch.stops.len(), 1);
    assert!(batch.stops[0].1 == stop);
    assert!(matches!(
        alice.update_location(conversation, reference, point(now + 30), now + 30),
        Err(Error::Obsolete)
    ));
    alice.queue_peer_action(b, &stop, now + 21).unwrap();
    flush(&mut alice, now + 21);
    let delivery = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|v| v.message_id == crate::transport::hex(&stop.id().unwrap()))
        .unwrap();
    bob.accept_delivery(&delivery).unwrap();
    assert!(
        bob.card_state(conversation, reference)
            .unwrap()
            .location
            .unwrap()
            .stopped
    );
    for delivery in delayed.into_iter().rev() {
        bob.accept_delivery(&delivery).unwrap();
        drain(&mut bob);
        assert!(
            bob.card_state(conversation, reference)
                .unwrap()
                .location
                .unwrap()
                .stopped
        );
    }
    let state = bob.card_state(conversation, reference).unwrap();
    assert_eq!(state.pending, 0);
    assert_eq!(state.rejected, 0);
    assert_eq!(state.location.unwrap().share.point.sampled_at, now + 20);
    assert!(alice
        .location_jobs(None, now + 22)
        .unwrap()
        .stops
        .is_empty());
    let card = alice
        .location_card(
            [178; 32],
            LocationKind::Live(Duration::FifteenMinutes),
            point(now + 22),
            Text::plain("Expiry", Default::default()).unwrap(),
            now + 22,
        )
        .unwrap();
    alice.queue_peer_card(b, &card, now + 22).unwrap();
    flush(&mut alice, now + 22);
    assert_eq!(alice.location_jobs(None, now + 22).unwrap().jobs.len(), 1);
    assert!(alice
        .location_jobs(None, now + 922)
        .unwrap()
        .jobs
        .is_empty());
}
#[test]
fn mixed_cards_share_atomic_delivery_and_root_deletion_authority() {
    use sigil_protocol::{
        conversation::{Action as ConversationAction, Reference as MessageReference},
        text::{composition, Document},
    };
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let conversation = alice.direct_conversation(b).unwrap();
    let Document::Composition(value) = composition::parse(
        "Before\npoll::open::Choose\n- One\n- Two;\nchecklist::Things\n- Water;\nAfter",
        Origin {
            message: [112; 32],
            creator: alice.account_reference().unwrap(),
            created_at: now,
            timezone: None,
        },
        Default::default(),
        None,
    )
    .unwrap()
    .content
    else {
        panic!();
    };
    let cards: Vec<_> = value.cards().cloned().collect();
    assert_eq!(cards.len(), 2);
    alice.queue_peer_composition(b, &value, now).unwrap();
    flush(&mut alice, now);
    let delivery = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON structured_origins WHEN (SELECT count(*) FROM structured_origins)>0 BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM structured_cards", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    bob.accept_delivery(&delivery).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM structured_sources", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let vote = action(&bob, &cards[0], None, 0, now + 1);
    bob.queue_peer_action(a, &vote, now).unwrap();
    flush(&mut bob, now);
    alice.accept_delivery(&next(&alice)).unwrap();
    for card in &cards {
        assert!(alice
            .card_state(conversation, Reference::of(card).unwrap())
            .is_ok());
    }
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let deletion = alice
        .conversation_operation(
            [113; 32],
            ConversationAction::Delete {
                target: MessageReference {
                    author: value.creator,
                    message: value.id,
                },
            },
        )
        .unwrap();
    alice
        .queue_peer_operation(b, &deletion, now + 2, now)
        .unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    for card in &cards {
        assert!(matches!(
            bob.card_state(conversation, Reference::of(card).unwrap()),
            Err(Error::Obsolete)
        ));
    }
    assert!(matches!(
        bob.queue_peer_action(a, &vote, now + 3),
        Err(Error::Obsolete)
    ));
}
#[test]
fn closed_poll_pages_freeze_observed_ballots_and_recover_out_of_order_across_restart() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    insert_card(&mut alice, &card);
    insert_card(&mut bob, &card);
    let mut votes = Vec::new();
    for i in 1..=130u8 {
        let mut vote = action(&alice, &card, None, usize::from(i % 2), now);
        vote.actor = [i; 32];
        insert(&mut alice, &vote);
        votes.push(vote);
    }
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON structured_close_tree BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.prepare_poll_close(CONVERSATION, reference, now),
        Err(Error::Storage(_))
    ));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM structured_close_parts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    let close = alice
        .prepare_poll_close(CONVERSATION, reference, now)
        .unwrap();
    let Change::ClosePoll(closure) = &close.change else {
        panic!();
    };
    assert_eq!(closure.voters, 130);
    assert_eq!(closure.pages(), 3);
    let pages: Vec<_> = (0..3)
        .map(|page| {
            alice
                .poll_close_page(CONVERSATION, reference, page)
                .unwrap()
        })
        .collect();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(
        alice
            .prepare_poll_close(CONVERSATION, reference, now)
            .unwrap()
            == close
    );
    assert!(alice.poll_close_page(CONVERSATION, reference, 1).unwrap() == pages[1]);
    for page in pages.iter().rev() {
        insert(&mut bob, page);
    }
    insert(&mut bob, &close);
    drain(&mut bob);
    let state = bob
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap();
    assert!(state.closed);
    assert_eq!(state.pending_pages, 3);
    assert!(state.counts.is_none());
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    for vote in votes.iter().rev() {
        insert(&mut bob, vote);
    }
    drain(&mut bob);
    let frozen = bob
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap();
    assert_eq!(frozen.pending_pages, 0);
    assert_eq!(frozen.voters, Some(130));
    assert_eq!(
        frozen
            .counts
            .as_ref()
            .unwrap()
            .iter()
            .map(|(_, count)| *count)
            .collect::<Vec<_>>(),
        vec![65, 65]
    );
    let mut late = votes[0].clone();
    late.previous = Some(late.id().unwrap());
    let Construct::Poll(poll) = &card.content else {
        panic!();
    };
    late.change = Change::Vote {
        choices: vec![poll.options[0].id],
    };
    insert(&mut bob, &late);
    drain(&mut bob);
    assert_eq!(
        bob.card_state(CONVERSATION, reference)
            .unwrap()
            .poll
            .unwrap()
            .counts,
        frozen.counts
    );
    assert!(matches!(
        bob.require_action(CONVERSATION, &late),
        Err(Error::InvalidEvent) | Err(Error::Obsolete)
    ));
    let mut forged = pages[0].clone();
    let Change::PollPage(page) = &mut forged.change else {
        panic!();
    };
    page.proof[0][0] ^= 1;
    insert(&mut bob, &forged);
    drain(&mut bob);
    assert_eq!(bob.card_state(CONVERSATION, reference).unwrap().rejected, 1);
    for page in &pages {
        insert(&mut bob, page);
    }
    assert_eq!(
        bob.card_state(CONVERSATION, reference)
            .unwrap()
            .poll
            .unwrap()
            .counts,
        frozen.counts
    );
    alice.discard_poll_close(CONVERSATION, reference).unwrap();
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM structured_close_tree", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn empty_poll_closes_without_pages_and_other_members_cannot_close_it() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    insert_card(&mut alice, &card);
    insert_card(&mut bob, &card);
    assert!(matches!(
        bob.prepare_poll_close(CONVERSATION, reference, now),
        Err(Error::InvalidEvent)
    ));
    let close = alice
        .prepare_poll_close(CONVERSATION, reference, now)
        .unwrap();
    insert(&mut alice, &close);
    let state = alice
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap();
    assert!(state.closed);
    assert_eq!(state.voters, Some(0));
    assert_eq!(state.pending_pages, 0);
    assert!(matches!(
        alice.require_action(CONVERSATION, &action(&alice, &card, None, 0, now)),
        Err(Error::Obsolete)
    ));
}
#[test]
fn poll_closure_uses_normal_encrypted_delivery_and_atomic_page_tallies() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    let vote = action(&bob, &card, None, 1, now);
    bob.queue_peer_action(a, &vote, now).unwrap();
    flush(&mut bob, now);
    alice.accept_delivery(&next(&alice)).unwrap();
    alice.acknowledge_incoming_online().unwrap();
    let close = alice
        .prepare_poll_close(conversation, reference, now)
        .unwrap();
    alice.queue_peer_action(b, &close, now).unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    let page = alice.poll_close_page(conversation, reference, 0).unwrap();
    alice.queue_peer_action(b, &page, now).unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    flush(&mut alice, now);
    let delivery = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON structured_closed_totals BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert!(bob
        .card_state(conversation, reference)
        .unwrap()
        .poll
        .unwrap()
        .counts
        .is_none());
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    bob.accept_delivery(&delivery).unwrap();
    assert!(bob.accept_delivery(&delivery).unwrap().duplicate);
    for client in [&alice, &bob] {
        let state = client
            .card_state(conversation, reference)
            .unwrap()
            .poll
            .unwrap();
        assert!(state.closed);
        assert_eq!(state.voters, Some(1));
        assert_eq!(
            state
                .counts
                .unwrap()
                .iter()
                .map(|(_, count)| *count)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }
}
#[test]
fn ballots_converge_across_order_restart_and_concurrent_same_account_devices() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    let first = action(&alice, &card, None, 0, now);
    let second = action(&alice, &card, Some(first.id().unwrap()), 1, now);
    let concurrent = action(&alice, &card, Some(first.id().unwrap()), 0, now);
    insert(&mut alice, &second);
    insert(&mut bob, &concurrent);
    insert_card(&mut alice, &card);
    insert_card(&mut bob, &card);
    assert_eq!(
        alice.card_state(CONVERSATION, reference).unwrap().pending,
        1
    );
    assert!(bob
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap()
        .counts
        .is_none());
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    insert(&mut alice, &concurrent);
    insert(&mut alice, &first);
    insert(&mut bob, &first);
    insert(&mut bob, &second);
    let b = action(&bob, &card, None, 1, now);
    insert(&mut alice, &b);
    insert(&mut bob, &b);
    for client in [&mut alice, &mut bob] {
        drain(client);
        let state = client.card_state(CONVERSATION, reference).unwrap();
        assert_eq!(state.pending, 0);
        assert_eq!(state.rejected, 0);
        assert_eq!(state.poll.unwrap().voters, Some(2));
    }
    let a = alice
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap();
    let b = bob
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap();
    assert_eq!(a.counts, b.counts);
    let expected = if second.wins_over(2, &concurrent, 2).unwrap() {
        second.id().unwrap()
    } else {
        concurrent.id().unwrap()
    };
    assert_eq!(a.operation, Some(expected));
    let before = a.counts;
    insert(&mut alice, &first);
    insert(&mut alice, &second);
    assert_eq!(
        alice
            .card_state(CONVERSATION, reference)
            .unwrap()
            .poll
            .unwrap()
            .counts,
        before
    );
    let mut withdrawal = action(&alice, &card, Some(expected), 0, now);
    withdrawal.change = Change::Vote {
        choices: Vec::new(),
    };
    insert(&mut alice, &withdrawal);
    insert(&mut bob, &withdrawal);
    assert!(alice
        .card_state(CONVERSATION, reference)
        .unwrap()
        .poll
        .unwrap()
        .counts
        .is_none());
    assert_eq!(
        bob.card_state(CONVERSATION, reference)
            .unwrap()
            .poll
            .unwrap()
            .voters,
        Some(1)
    );
}
#[test]
fn dependency_batches_survive_restart_and_invalid_ancestors_never_become_votes() {
    let (dir, _fixture, mut alice, _bob, now) = pair();
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    let mut chain = Vec::new();
    let mut previous = None;
    for i in 0..150 {
        let next = action(&alice, &card, previous, i % 2, now);
        previous = Some(next.id().unwrap());
        chain.push(next)
    }
    for action in chain.iter().rev() {
        insert(&mut alice, action)
    }
    insert_card(&mut alice, &card);
    assert!(alice.card_state(CONVERSATION, reference).unwrap().pending > 0);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    drain(&mut alice);
    let state = alice.card_state(CONVERSATION, reference).unwrap();
    assert_eq!(state.pending, 0);
    assert_eq!(state.poll.unwrap().operation, previous);
    let mut invalid = action(&alice, &card, None, 0, now);
    invalid.change = Change::Vote {
        choices: vec![[99; 32]],
    };
    let child = action(&alice, &card, Some(invalid.id().unwrap()), 1, now);
    insert(&mut alice, &child);
    insert(&mut alice, &invalid);
    drain(&mut alice);
    let state = alice.card_state(CONVERSATION, reference).unwrap();
    assert_eq!(state.pending, 0);
    assert_eq!(state.rejected, 2);
    assert_eq!(state.poll.unwrap().operation, previous);
    let index = card_index(
        &alice.key,
        &account_context(&alice.db, &alice.key).unwrap().0,
        &CONVERSATION,
        &reference,
    )
    .unwrap();
    let root = op_index(&alice.key, &index, &chain[0].id().unwrap()).unwrap();
    alice
        .db
        .execute(
            "UPDATE structured_actions SET status=0 WHERE id=?1",
            [root.as_slice()],
        )
        .unwrap();
    assert!(load_node(&alice.db, &alice.key, &root).is_err());
}
#[test]
fn direct_actions_commit_with_ratchet_history_and_deletion_blocks_queued_handoff() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let card = card(&alice, "poll::Choose\n- One\n- Two;", now);
    let reference = Reference::of(&card).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    flush(&mut alice, now);
    let initial = bob.accept_delivery(&next(&bob)).unwrap();
    let vote = action(&bob, &card, None, 1, now);
    bob.queue_peer_action(a, &vote, now).unwrap();
    let session = flush(&mut bob, now);
    let delivery = next(&alice);
    let before: Option<Vec<u8>> = alice
        .db
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [session.as_slice()],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON structured_totals BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.accept_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    let after: Option<Vec<u8>> = alice
        .db
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [session.as_slice()],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        alice
            .card_state(conversation, reference)
            .unwrap()
            .poll
            .unwrap()
            .voters,
        Some(0)
    );
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    let received = alice.accept_delivery(&delivery).unwrap();
    assert!(received.event().unwrap().content.action().unwrap() == vote);
    assert_eq!(
        alice
            .card_state(conversation, reference)
            .unwrap()
            .poll
            .unwrap()
            .voters,
        Some(1)
    );
    assert!(alice.accept_delivery(&delivery).unwrap().duplicate);
    let queued = action(&alice, &card, None, 0, now);
    alice.queue_peer_action(b, &queued, now).unwrap();
    let root = crate::event_history_id(&initial.event().unwrap(), &alice.identity().unwrap());
    let mut deleted = alice.recovery_record(root).unwrap();
    deleted.revision += 1;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    alice.retain_recovery_record(&deleted).unwrap();
    assert!(matches!(
        alice.card_state(conversation, reference),
        Err(Error::Obsolete)
    ));
    let mut denied = false;
    for _ in 0..3 {
        for attempt in alice.resume_send_intents_online(now).unwrap() {
            if attempt.id == queued.id().unwrap() {
                assert!(matches!(attempt.result, Err(Error::Obsolete)));
                denied = true;
            }
        }
        if denied {
            break;
        }
    }
    assert!(denied);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(matches!(
        alice.card_state(conversation, reference),
        Err(Error::Obsolete)
    ));
    assert!(matches!(
        alice.queue_peer_action(b, &queued, now),
        Err(Error::Obsolete)
    ));
}
#[test]
fn schema_51_migration_recovers_unarchived_direct_cards_without_recreating_identity() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    for client in [&alice, &bob] {
        crate::test_schema::rewind(&client.db, 51);
    }
    drop(alice);
    drop(bob);
    let alice = open(&dir.path().join("alice.db"));
    let bob = open(&dir.path().join("bob.db"));
    assert!(alice.card_state(conversation, reference).unwrap().card == card);
    assert!(bob.card_state(conversation, reference).unwrap().card == card);
}

#[test]
fn intact_card_actions_recover_on_a_fresh_reauthorized_device_without_live_keys() {
    for mode in 0..3 {
        recover_card_actions(mode);
    }
}
fn recover_card_actions(mode: u8) {
    let (dir, fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    configure(&mut bob, 8);
    let card = if mode == 2 {
        recurring_card(&alice, now)
    } else if mode == 1 {
        card(&alice, "checklist::task::Tasks\n- One;", now)
    } else {
        poll(&alice, now)
    };
    let reference = Reference::of(&card).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    let vote = if mode == 2 {
        recurring_check(&bob, &card, 0, now)
    } else if mode == 1 {
        completion(&bob, &card, now)
    } else {
        action(&bob, &card, None, 1, now)
    };
    bob.queue_peer_action(a, &vote, now).unwrap();
    flush(&mut bob, now);
    assert!(
        alice
            .accept_delivery(&next(&alice))
            .unwrap()
            .event()
            .unwrap()
            .content
            .action()
            .unwrap()
            == vote
    );
    let head = bob.prepare_recovery_upload(now).unwrap();
    assert_eq!(bob.upload_recovery_step().unwrap(), Some(head));
    let original = bob.connection_session().unwrap().unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server
        .invite_reauthorization(&original.account_id, 60, now)
        .unwrap();
    let path = dir.path().join("actions-recovered.db");
    let reopen = || {
        ClientStore::open(
            &path,
            StorageKey::new(Secret32::from_bytes([29; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut recovered = reopen();
    recovered
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic recovered device",
            true,
        )
        .unwrap();
    let enrolled = recovered.enroll_online().unwrap();
    assert_eq!(enrolled.account_id, original.account_id);
    assert_ne!(enrolled.device_id, original.device_id);
    configure(&mut recovered, 8);
    let mut done = false;
    for _ in 0..8 {
        if recovered.download_recovery_step(true).unwrap() == Some(head) {
            done = true;
            break;
        }
        drop(recovered);
        recovered = reopen();
    }
    assert!(done);
    drain(&mut recovered);
    let state = recovered.card_state(conversation, reference).unwrap();
    assert_eq!(state.pending, 0);
    if mode == 2 {
        let state = recovered
            .recurring_state(conversation, reference, now)
            .unwrap();
        assert!(state.checks[0].checked);
        assert_eq!(state.checks[0].actor, Some(vote.actor));
        assert_eq!(state.checks[0].operation, Some(vote.id().unwrap()));
        let reset = recovered
            .recurring_state(conversation, reference, state.period.end)
            .unwrap();
        assert_eq!(reset.checks.len(), 1);
        assert!(!reset.checks[0].checked);
    } else if mode == 1 {
        assert!(state.tasks[0].completed);
        assert_eq!(state.tasks[0].active_completions, 1);
        let page = recovered
            .task_completions(conversation, reference, state.tasks[0].item, None)
            .unwrap();
        assert_eq!(page.completions[0].actor, vote.actor);
        assert_eq!(page.completions[0].operation, vote.id().unwrap());
    } else {
        let poll = state.poll.unwrap();
        assert_eq!(poll.operation, Some(vote.id().unwrap()));
        assert_eq!(poll.voters, Some(1));
    }
    assert_eq!(
        recovered.account_reference().unwrap(),
        bob.account_reference().unwrap()
    );
    for table in ["sessions", "identity", "peers", "prekeys", "push_state"] {
        assert_eq!(
            recovered
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
}
#[test]
fn concurrent_checklist_writers_do_not_lose_independent_items_or_double_apply() {
    let (dir, _fixture, mut alice, _bob, now) = pair();
    let card = card(&alice, "checklist::Things\n- One\n- Two;", now);
    let reference = Reference::of(&card).unwrap();
    insert_card(&mut alice, &card);
    let Construct::Checklist(list) = &card.content else {
        panic!()
    };
    let first = Action {
        card: reference,
        actor: alice.account_reference().unwrap(),
        created_at: now,
        previous: None,
        revision: None,
        change: Change::Check {
            item: list.items[0].id,
            checked: true,
        },
    };
    let second = Action {
        change: Change::Check {
            item: list.items[1].id,
            checked: true,
        },
        ..first.clone()
    };
    let rival = Action {
        actor: [9; 32],
        change: Change::Check {
            item: list.items[0].id,
            checked: false,
        },
        ..first.clone()
    };
    let expected = if first.wins_over(1, &rival, 1).unwrap() {
        first.id().unwrap()
    } else {
        rival.id().unwrap()
    };
    let path = dir.path().join("alice.db");
    drop(alice);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            let mut client = open(&path);
            insert(&mut client, &first);
            insert(&mut client, &second);
        });
        let b = scope.spawn(|| {
            let mut client = open(&path);
            insert(&mut client, &rival);
            insert(&mut client, &second);
        });
        a.join().unwrap();
        b.join().unwrap();
    });
    let mut alice = open(&path);
    drain(&mut alice);
    let state = alice.card_state(CONVERSATION, reference).unwrap();
    assert_eq!(state.pending, 0);
    assert_eq!(state.checks[0].operation, Some(expected));
    assert_eq!(state.checks[1].operation, Some(second.id().unwrap()));
    assert!(state.checks[1].checked);
}

#[test]
fn an_archived_card_tombstone_preceding_delivery_cannot_be_resurrected_by_migration() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    configure(&mut bob, 8);
    let card = poll(&alice, now);
    let reference = Reference::of(&card).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    let session = flush(&mut alice, now);
    let plaintext = alice.outgoing_message(session, card.id).unwrap();
    let event = sigil_protocol::event::Direct::from_bytes(&plaintext).unwrap();
    let author = alice.identity().unwrap();
    let id = crate::event_history_id(&event, &author);
    bob.retain_recovery_record(&sigil_crypto::recovery::Record {
        id,
        revision: 1,
        conversation: event.conversation,
        author,
        created_at: now,
        direction: sigil_crypto::recovery::Direction::Incoming,
        content: sigil_crypto::recovery::Content::Deleted,
    })
    .unwrap();
    bob.accept_delivery(&next(&bob)).unwrap();
    assert!(matches!(
        bob.card_state(event.conversation, reference),
        Err(Error::Obsolete)
    ));
    crate::test_schema::rewind(&bob.db, 51);
    drop(bob);
    let bob = open(&dir.path().join("bob.db"));
    assert!(matches!(
        bob.card_state(event.conversation, reference),
        Err(Error::Obsolete)
    ));
}

fn completion(client: &ClientStore, card: &Card, now: u64) -> Action {
    let Construct::Checklist(list) = &card.content else {
        panic!()
    };
    Action {
        card: Reference::of(card).unwrap(),
        actor: client.account_reference().unwrap(),
        created_at: now,
        previous: None,
        revision: None,
        change: Change::Complete {
            item: list.items[0].id,
        },
    }
}
fn undo(completion: &Action, now: u64) -> Action {
    let Change::Complete { item } = completion.change else {
        panic!()
    };
    let id = completion.id().unwrap();
    Action {
        card: completion.card,
        actor: completion.actor,
        created_at: now,
        previous: Some(id),
        revision: None,
        change: Change::Undo {
            item,
            completion: id,
        },
    }
}
#[test]
fn task_completions_converge_out_of_order_without_cross_author_undo() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let card = card(&alice, "checklist::task::Tasks\n- One\n- Two;", now);
    let reference = Reference::of(&card).unwrap();
    let a = completion(&alice, &card, now);
    let b = completion(&bob, &card, now);
    let undo_a = undo(&a, now + 29);
    let mut forged = undo(&b, now + 29);
    forged.actor = a.actor;
    let late = undo(&b, now + 30);
    for action in [&undo_a, &late, &forged, &b, &a] {
        insert(&mut alice, action);
    }
    insert_card(&mut alice, &card);
    insert_card(&mut bob, &card);
    for action in [&a, &b, &forged, &late, &undo_a] {
        insert(&mut bob, action);
    }
    for client in [&mut alice, &mut bob] {
        drain(client);
        let state = client.card_state(CONVERSATION, reference).unwrap();
        assert_eq!(state.pending, 0);
        assert_eq!(state.rejected, 2);
        assert!(state.tasks[0].completed);
        assert_eq!(state.tasks[0].active_completions, 1);
        assert!(!state.tasks[1].completed);
        let page = client
            .task_completions(CONVERSATION, reference, state.tasks[0].item, None)
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.completions.len(), 1);
        assert!(page.next.is_none());
        assert_eq!(page.completions[0].actor, b.actor);
        assert_eq!(page.completions[0].operation, b.id().unwrap());
        assert_eq!(page.completions[0].undo_until, now + 30);
        insert(client, &undo_a);
        assert_eq!(
            client.card_state(CONVERSATION, reference).unwrap().tasks[0].active_completions,
            1
        );
        insert(client, &undo(&b, now + 28));
        assert!(!client.card_state(CONVERSATION, reference).unwrap().tasks[0].completed);
    }
}

#[test]
fn task_rollback_restart_and_pagination_preserve_attribution_and_counters() {
    let (dir, _fixture, mut alice, _bob, now) = pair();
    let card = card(&alice, "checklist::task::Tasks\n- One;", now);
    let reference = Reference::of(&card).unwrap();
    insert_card(&mut alice, &card);
    let first = completion(&alice, &card, now);
    let scope = account_context(&alice.db, &alice.key).unwrap().0;
    alice.db.execute_batch("CREATE TRIGGER task_failure BEFORE INSERT ON structured_task_completions BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let tx = alice.db.transaction().unwrap();
    assert!(ingest(
        &tx,
        &alice.key,
        scope,
        CONVERSATION,
        first.id().unwrap(),
        &first.to_bytes().unwrap()
    )
    .is_err());
    tx.rollback().unwrap();
    assert_eq!(
        alice.card_state(CONVERSATION, reference).unwrap().tasks[0].active_completions,
        0
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM structured_actions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice
        .db
        .execute_batch("DROP TRIGGER task_failure;")
        .unwrap();
    drop(alice);
    let path = dir.path().join("alice.db");
    let write = |range: std::ops::Range<u64>| {
        let mut client = open(&path);
        for n in range {
            let mut action = first.clone();
            action.created_at = now + n;
            insert(&mut client, &action);
        }
    };
    std::thread::scope(|scope| {
        let first = scope.spawn(|| write(0..65));
        let second = scope.spawn(|| write(65..130));
        first.join().unwrap();
        second.join().unwrap();
    });
    let alice = open(&dir.path().join("alice.db"));
    let state = alice.card_state(CONVERSATION, reference).unwrap();
    let item = state.tasks[0].item;
    assert_eq!(state.tasks[0].active_completions, 130);
    let mut after = None;
    let mut ids = std::collections::BTreeSet::new();
    for expected in [64, 64, 2] {
        let page = alice
            .task_completions(CONVERSATION, reference, item, after)
            .unwrap();
        assert_eq!(page.total, 130);
        assert_eq!(page.completions.len(), expected);
        for entry in page.completions {
            assert!(ids.insert(entry.operation));
        }
        after = page.next;
    }
    assert!(after.is_none());
    assert_eq!(ids.len(), 130);
    alice
        .db
        .execute("UPDATE structured_task_counts SET content=zeroblob(44)", [])
        .unwrap();
    assert!(alice.card_state(CONVERSATION, reference).is_err());
}

#[test]
fn task_completion_and_undo_share_encrypted_delivery_and_ratchet_commit() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    let card = card(&alice, "checklist::task::Tasks\n- One;", now);
    let reference = Reference::of(&card).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, now).unwrap();
    flush(&mut alice, now);
    bob.accept_delivery(&next(&bob)).unwrap();
    let complete = completion(&bob, &card, now);
    bob.queue_peer_action(a, &complete, now).unwrap();
    flush(&mut bob, now);
    let delivery = next(&alice);
    let before: Vec<Vec<u8>> = alice
        .db
        .prepare("SELECT state FROM sessions ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER task_failure BEFORE INSERT ON structured_task_counts BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.accept_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    let after: Vec<Vec<u8>> = alice
        .db
        .prepare("SELECT state FROM sessions ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before, after);
    assert!(!alice.card_state(conversation, reference).unwrap().tasks[0].completed);
    alice
        .db
        .execute_batch("DROP TRIGGER task_failure;")
        .unwrap();
    let received = alice.accept_delivery(&delivery).unwrap();
    assert!(received.event().unwrap().content.action().unwrap() == complete);
    alice.acknowledge_incoming_online().unwrap();
    // Completion/undo can occur in the same second. Do not advance the transport
    // clock ahead of the real HTTPS server and exceed its maximum expiry window.
    let undo = undo(&complete, now);
    bob.queue_peer_action(a, &undo, now).unwrap();
    flush(&mut bob, now);
    let received = alice.accept_delivery(&next(&alice)).unwrap();
    assert!(received.event().unwrap().content.action().unwrap() == undo);
    for client in [&alice, &bob] {
        let state = client.card_state(conversation, reference).unwrap();
        assert!(!state.tasks[0].completed);
        assert_eq!(state.tasks[0].active_completions, 0);
    }
}

#[test]
fn task_schema_52_upgrade_preserves_initial_completion_without_fabricated_undo() {
    let (dir, _fixture, mut alice, _bob, now) = pair();
    let mut card = card(&alice, "checklist::task::Tasks\n- One\n- Two;", now);
    let Construct::Checklist(ref mut list) = card.content else {
        panic!()
    };
    list.items[0].checked = true;
    let reference = Reference::of(&card).unwrap();
    insert_card(&mut alice, &card);
    crate::test_schema::rewind(&alice.db, 52);
    drop(alice);
    let alice = open(&dir.path().join("alice.db"));
    let state = alice.card_state(CONVERSATION, reference).unwrap();
    assert!(state.tasks[0].completed && state.tasks[0].initially_completed);
    assert_eq!(state.tasks[0].active_completions, 0);
    assert!(!state.tasks[1].completed);
    let page = alice
        .task_completions(CONVERSATION, reference, state.tasks[0].item, None)
        .unwrap();
    assert!(page.completions.is_empty());
    assert_eq!(page.total, 0);
    assert!(completion(&alice, &card, now).validate_for(&card).is_err());
    assert_eq!(
        alice
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        68
    );
}

fn recurring_card(client: &ClientStore, now: u64) -> Card {
    let mut card = card(client, "checklist::Items\n- One\n- Two;", now);
    let Construct::Checklist(ref mut list) = card.content else {
        panic!()
    };
    list.mode = sigil_protocol::text::structured::ListMode::Recurring(
        sigil_protocol::text::recurrence::Recurrence::new(
            sigil_protocol::text::recurrence::Interval::Weekly,
            "UTC",
            now,
        )
        .unwrap(),
    );
    list.items[0].persistent = true;
    card
}
#[test]
fn edits_add_remove_reorder_items_and_wait_for_the_exact_definition() {
    use sigil_protocol::text::{structured::ListItem, Text};
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let card = card(&alice, "checklist::Things\n- One\n- Two;", now);
    insert_card(&mut alice, &card);
    insert_card(&mut bob, &card);
    let reference = Reference::of(&card).unwrap();
    let mut content = card.content.clone();
    let Construct::Checklist(list) = &mut content else {
        panic!();
    };
    list.items.remove(0);
    list.items.insert(
        0,
        ListItem {
            id: [117; 32],
            text: Text::plain("New item", Default::default()).unwrap(),
            checked: false,
            persistent: false,
        },
    );
    let edit = alice
        .edit_card(CONVERSATION, reference, content.clone(), now)
        .unwrap();
    let check = Action {
        card: reference,
        actor: bob.account_reference().unwrap(),
        created_at: now,
        previous: None,
        revision: Some(edit.id().unwrap()),
        change: Change::Check {
            item: [117; 32],
            checked: true,
        },
    };
    insert(&mut bob, &check);
    assert_eq!(bob.card_state(CONVERSATION, reference).unwrap().pending, 1);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    insert(&mut bob, &edit);
    drain(&mut bob);
    let state = bob.card_state(CONVERSATION, reference).unwrap();
    assert!(state.definition.content == content);
    assert_eq!(state.checks[0].item, [117; 32]);
    assert!(state.checks[0].checked);
    assert_eq!(state.pending, 0);
    assert_eq!(state.rejected, 0);
    assert!(matches!(
        bob.edit_card(CONVERSATION, reference, content.clone(), now),
        Err(Error::InvalidEvent)
    ));
    let stale = Action {
        revision: None,
        ..check.clone()
    };
    assert!(matches!(
        bob.require_action(CONVERSATION, &stale),
        Err(Error::Obsolete)
    ));
    let forged = Action {
        actor: bob.account_reference().unwrap(),
        ..edit.clone()
    };
    insert(&mut alice, &forged);
    drain(&mut alice);
    assert_eq!(
        alice.card_state(CONVERSATION, reference).unwrap().rejected,
        1
    );
    insert(&mut alice, &edit);
    insert(&mut alice, &check);
    drain(&mut alice);
    assert!(alice.card_state(CONVERSATION, reference).unwrap().checks[0].checked);
}

#[test]
fn recurrence_revocation_dominates_a_delegates_later_branch_in_every_delivery_order() {
    use sigil_protocol::text::{recurrence::Interval, structured::ListMode};
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let card = recurring_card(&alice, now);
    let reference = Reference::of(&card).unwrap();
    insert_card(&mut alice, &card);
    insert_card(&mut bob, &card);
    let policy = alice
        .set_recurrence_editors(
            CONVERSATION,
            reference,
            vec![bob.account_reference().unwrap()],
            now,
        )
        .unwrap();
    insert(&mut alice, &policy);
    insert(&mut bob, &policy);
    let mut content = card.content.clone();
    let Construct::Checklist(list) = &mut content else {
        panic!();
    };
    let ListMode::Recurring(rule) = &mut list.mode else {
        panic!();
    };
    rule.interval = Interval::Monthly;
    let edit = bob
        .edit_card(CONVERSATION, reference, content.clone(), now)
        .unwrap();
    insert(&mut bob, &edit);
    let mut extra = content.clone();
    let Construct::Checklist(list) = &mut extra else {
        panic!();
    };
    list.items.reverse();
    assert!(matches!(
        bob.edit_card(CONVERSATION, reference, extra, now),
        Err(Error::InvalidEvent)
    ));
    let revoked = alice
        .set_recurrence_editors(CONVERSATION, reference, vec![], now)
        .unwrap();
    let later = bob
        .edit_card(CONVERSATION, reference, content, now)
        .unwrap();
    insert(&mut bob, &later);
    insert(&mut alice, &revoked);
    insert(&mut alice, &later);
    insert(&mut alice, &edit);
    insert(&mut bob, &revoked);
    drain(&mut alice);
    drain(&mut bob);
    for client in [&alice, &bob] {
        let state = client.card_state(CONVERSATION, reference).unwrap();
        assert_eq!(state.definition.policy, Some(revoked.id().unwrap()));
        assert!(state.definition.content == card.content);
        assert!(state.definition.editors.is_empty());
        assert_eq!(state.pending, 0);
        assert_eq!(state.rejected, 0);
    }
    assert!(matches!(
        bob.require_action(CONVERSATION, &later),
        Err(Error::Obsolete)
    ));
    assert!(matches!(
        bob.set_recurrence_editors(
            CONVERSATION,
            reference,
            vec![bob.account_reference().unwrap()],
            now
        ),
        Err(Error::InvalidEvent)
    ));
}
fn recurring_check(client: &ClientStore, card: &Card, item: usize, now: u64) -> Action {
    let Construct::Checklist(list) = &card.content else {
        panic!()
    };
    let sigil_protocol::text::structured::ListMode::Recurring(rule) = &list.mode else {
        panic!()
    };
    Action {
        card: Reference::of(card).unwrap(),
        actor: client.account_reference().unwrap(),
        created_at: now,
        previous: None,
        revision: None,
        change: Change::RecurringCheck {
            item: list.items[item].id,
            period: rule.period_at(now).unwrap().start,
        },
    }
}
#[test]
fn recurring_reset_ignores_delayed_checks_and_removes_one_offs_after_offline_restart() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let card = recurring_card(&alice, now);
    let reference = Reference::of(&card).unwrap();
    let initial = recurring_check(&alice, &card, 0, now);
    let one_off = recurring_check(&bob, &card, 1, now);
    let sigil_protocol::text::structured::Construct::Checklist(list) = &card.content else {
        panic!()
    };
    let sigil_protocol::text::structured::ListMode::Recurring(rule) = &list.mode else {
        panic!()
    };
    let reset = rule.next_reset(now).unwrap();
    let current = recurring_check(&bob, &card, 0, reset);
    let removed = recurring_check(&alice, &card, 1, reset);
    insert_card(&mut alice, &card);
    insert(&mut alice, &initial);
    insert(&mut alice, &one_off);
    assert!(alice
        .recurring_state(CONVERSATION, reference, reset - 1)
        .unwrap()
        .checks
        .iter()
        .all(|i| i.checked));
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let state = alice
        .recurring_state(CONVERSATION, reference, reset)
        .unwrap();
    assert_eq!(state.period.start, reset);
    assert_eq!(state.checks.len(), 1);
    assert!(!state.checks[0].checked);
    assert!(alice
        .card_state(CONVERSATION, reference)
        .unwrap()
        .checks
        .is_empty());
    insert(&mut alice, &initial);
    assert!(
        !alice
            .recurring_state(CONVERSATION, reference, reset)
            .unwrap()
            .checks[0]
            .checked
    );
    insert(&mut alice, &current);
    insert(&mut alice, &removed);
    insert(&mut bob, &removed);
    insert(&mut bob, &current);
    insert(&mut bob, &one_off);
    insert(&mut bob, &initial);
    insert_card(&mut bob, &card);
    for client in [&mut alice, &mut bob] {
        drain(client);
        assert_eq!(
            client.card_state(CONVERSATION, reference).unwrap().rejected,
            1
        );
        let current_state = client
            .recurring_state(CONVERSATION, reference, reset)
            .unwrap();
        assert!(current_state.checks[0].checked);
        assert_eq!(
            current_state.checks[0].operation,
            Some(current.id().unwrap())
        );
        let before = client
            .db
            .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap();
        let later = client
            .recurring_state(CONVERSATION, reference, reset + 10 * 365 * 86400)
            .unwrap();
        assert!(!later.checks[0].checked);
        assert_eq!(later.checks.len(), 1);
        assert_eq!(
            client
                .db
                .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
    }
}

use super::*;
use crate::claims::tests::pair;
use sigil_protocol::{
    conversation::{Action, Operation},
    text::{action::Action as CardAction, composition, structured, time::Dated, Origin},
};

fn post(client: &mut ClientStore, id: Id, body: Body, now: u64) -> (Id, Operation) {
    let op = client
        .conversation_operation(
            id,
            Action::Post {
                body,
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    (client.note_to_self(&op, now, now).unwrap(), op)
}
fn apply(client: &mut ClientStore, action: &CardAction, now: u64) {
    post(
        client,
        action.id().unwrap(),
        Body::Rich(action.to_bytes().unwrap()),
        now,
    );
}
fn reminder(client: &ClientStore, id: Id, now: u64, at: u64) -> Card {
    Card {
        id,
        creator: client.account_reference().unwrap(),
        created_at: now,
        content: Construct::Reminder(Dated {
            text: Text::plain("Call Example", Default::default()).unwrap(),
            at,
            timezone: "UTC".into(),
            tzdb: sigil_protocol::text::recurrence::TZDB_VERSION.into(),
        }),
    }
}
#[test]
fn notes_follow_original_edits_deletion_and_mixed_card_types() {
    let (_dir, _fixture, mut client, _other, now) = pair();
    let creator = client.account_reference().unwrap();
    let (conversation, original) = post(
        &mut client,
        [131; 32],
        Body::Text("Bring water".into()),
        now,
    );
    assert!(client
        .notes_page(conversation, None, now)
        .unwrap()
        .entries
        .is_empty());
    let target = MessageReference {
        author: creator,
        message: original.id,
    };
    let promotion = client
        .note_promotion([132; 32], target.clone(), true)
        .unwrap();
    client.note_to_self(&promotion, now, now).unwrap();
    let edit = client
        .conversation_operation(
            [133; 32],
            Action::Edit {
                target: target.clone(),
                body: Body::Text("Bring tea".into()),
            },
        )
        .unwrap();
    client.note_to_self(&edit, now, now).unwrap();
    let page = client.notes_page(conversation, None, now).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(matches!(&page.entries[0].message.body, Some(Body::Text(text)) if text == "Bring tea"));
    assert_eq!(page.entries[0].message.reference.author, creator);
    let value = composition::parse(
        "note::A note; timer::5m; poll::Choice\n- A\n- B;",
        Origin {
            message: [134; 32],
            creator,
            created_at: now,
            timezone: Some("UTC"),
        },
        Default::default(),
        None,
    )
    .unwrap()
    .content;
    post(
        &mut client,
        [134; 32],
        Body::Rich(value.to_bytes().unwrap()),
        now,
    );
    let page = client.notes_page(conversation, None, now).unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[1].cards.len(), 1);
    assert!(matches!(page.entries[1].cards[0].kind, NoteKind::Note));
    let deletion = client
        .conversation_operation([135; 32], Action::Delete { target })
        .unwrap();
    client.note_to_self(&deletion, now, now).unwrap();
    assert_eq!(
        client
            .notes_page(conversation, None, now)
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn alarm_versions_survive_restart_and_block_stale_disabled_deleted_or_replayed_callbacks() {
    let (dir, _fixture, mut client, _other, now) = pair();
    let card = reminder(&client, [136; 32], now, now + 120);
    let reference = Reference::of(&card).unwrap();
    let (conversation, original) = post(
        &mut client,
        card.id,
        Body::Rich(card.to_bytes().unwrap()),
        now,
    );
    let job = client.alarm_jobs(now).unwrap().jobs.remove(0);
    assert_eq!(job.at, Some(now + 120));
    assert!(client.acknowledge_alarm_job(job.id, job.version).unwrap());
    assert!(client
        .alarm_notification(job.id, job.version, now + 119)
        .unwrap()
        .is_none());
    drop(client);
    let mut client = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let mut content = card.content.clone();
    let Construct::Reminder(value) = &mut content else {
        panic!();
    };
    value.at = now + 240;
    value.text =
        sigil_protocol::text::parse("redact::SYNTHETIC_SECRET; public", Default::default())
            .unwrap();
    let edit = client
        .edit_card(conversation, reference, content, now)
        .unwrap();
    apply(&mut client, &edit, now);
    let current = client.alarm_jobs(now).unwrap().jobs.remove(0);
    assert_eq!(current.id, job.id);
    assert_ne!(current.version, job.version);
    assert_eq!(current.at, Some(now + 240));
    assert!(!client.acknowledge_alarm_job(job.id, job.version).unwrap());
    assert!(client
        .alarm_notification(job.id, job.version, now + 300)
        .unwrap()
        .is_none());
    client
        .set_alarm_enabled(conversation, reference, false)
        .unwrap();
    let cancel = client.alarm_jobs(now).unwrap().jobs.remove(0);
    assert!(cancel.at.is_none());
    assert!(client
        .alarm_notification(current.id, current.version, now + 300)
        .unwrap()
        .is_none());
    client
        .set_alarm_enabled(conversation, reference, true)
        .unwrap();
    let current = client.alarm_jobs(now).unwrap().jobs.remove(0);
    let notification = client
        .alarm_notification(current.id, current.version, now + 300)
        .unwrap()
        .unwrap();
    assert!(!notification.text.body().contains("SYNTHETIC_SECRET"));
    assert!(notification.text.body().contains("public"));
    client.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON structured_alarms BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        client.acknowledge_alarm_fired(current.id, current.version, now + 300),
        Err(Error::Storage(_))
    ));
    client.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert!(client
        .acknowledge_alarm_fired(current.id, current.version, now + 300)
        .unwrap());
    assert!(!client
        .acknowledge_alarm_fired(current.id, current.version, now + 300)
        .unwrap());
    client.note_to_self(&original, now, now).unwrap();
    assert!(client
        .alarm_jobs(now + 300)
        .unwrap()
        .jobs
        .remove(0)
        .at
        .is_none());
    let notes = client.notes_page(conversation, None, now + 300).unwrap();
    assert_eq!(notes.entries.len(), 1);
    assert!(!notes.entries[0].cards[0].active);
    let deletion = client
        .conversation_operation(
            [137; 32],
            Action::Delete {
                target: MessageReference {
                    author: card.creator,
                    message: card.id,
                },
            },
        )
        .unwrap();
    client.note_to_self(&deletion, now, now).unwrap();
    assert!(client
        .alarm_notification(current.id, current.version, now + 300)
        .unwrap()
        .is_none());
    assert!(client
        .notes_page(conversation, None, now + 300)
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn edited_tasks_expose_new_items_through_the_current_definition() {
    let (_dir, _fixture, mut client, _other, now) = pair();
    let sigil_protocol::text::Parsed::Card(card) = sigil_protocol::text::parse_card(
        "checklist::task::Tasks\n- Original;",
        Origin {
            message: [138; 32],
            creator: client.account_reference().unwrap(),
            created_at: now,
            timezone: None,
        },
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!();
    };
    let reference = Reference::of(&card).unwrap();
    let (conversation, _) = post(
        &mut client,
        card.id,
        Body::Rich(card.to_bytes().unwrap()),
        now,
    );
    let mut content = card.content.clone();
    let Construct::Checklist(list) = &mut content else {
        panic!();
    };
    list.items.push(structured::ListItem {
        id: [139; 32],
        text: Text::plain("Added", Default::default()).unwrap(),
        checked: false,
        persistent: false,
    });
    let edit = client
        .edit_card(conversation, reference, content, now)
        .unwrap();
    apply(&mut client, &edit, now);
    let state = client.card_state(conversation, reference).unwrap();
    assert_eq!(state.tasks.len(), 2);
    assert_eq!(state.tasks[1].item, [139; 32]);
    let complete = CardAction {
        card: reference,
        actor: card.creator,
        created_at: now,
        previous: None,
        revision: state.definition.revision,
        change: sigil_protocol::text::action::Change::Complete { item: [139; 32] },
    };
    apply(&mut client, &complete, now);
    assert!(client.card_state(conversation, reference).unwrap().tasks[1].completed);
}
#[test]
fn future_alarms_wait_for_restored_message_origins_without_blocking_other_jobs() {
    use sha2::{Digest, Sha256};
    let (_dir, _fixture, mut client, _other, now) = pair();
    let (scope, creator) = crate::structured::account_context(&client.db, &client.key).unwrap();
    let conversation: Id =
        Sha256::digest([b"Sigil/note-to-self/v0".as_slice(), &creator].concat()).into();
    let mut cards = Vec::new();
    for i in 1..=66u8 {
        let card = reminder(&client, [i; 32], now, now + 600);
        let tx = client.db.transaction().unwrap();
        crate::structured::ingest(
            &tx,
            &client.key,
            scope,
            conversation,
            [i; 32],
            &card.to_bytes().unwrap(),
        )
        .unwrap();
        tx.commit().unwrap();
        cards.push(card);
    }
    let first = client.alarm_jobs(now).unwrap();
    assert!(first.more);
    assert!(first.jobs.is_empty());
    let second = client.alarm_jobs(now).unwrap();
    assert!(!second.more);
    assert!(second.jobs.is_empty());
    let card = &cards[65];
    post(
        &mut client,
        card.id,
        Body::Rich(card.to_bytes().unwrap()),
        now,
    );
    let mut jobs = client.alarm_jobs(now).unwrap().jobs;
    jobs.extend(client.alarm_jobs(now).unwrap().jobs);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].at, Some(now + 600));
    let deletion = client
        .conversation_operation(
            [140; 32],
            Action::Delete {
                target: MessageReference {
                    author: creator,
                    message: card.id,
                },
            },
        )
        .unwrap();
    client.note_to_self(&deletion, now, now).unwrap();
    assert!(client
        .alarm_notification(jobs[0].id, jobs[0].version, now + 700)
        .unwrap()
        .is_none());
    let mut cancelled = client.alarm_jobs(now).unwrap().jobs;
    cancelled.extend(client.alarm_jobs(now).unwrap().jobs);
    assert_eq!(cancelled.len(), 1);
    assert!(cancelled[0].at.is_none());
}

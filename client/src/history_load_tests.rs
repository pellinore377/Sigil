use super::*;
use std::time::{Duration, Instant};

#[test]
#[ignore = "100,000-message resource benchmark; run client/tests/load.sh"]
fn retained_history_load() {
    let (_dir, _fixture, mut store, _other, now) = crate::claims::tests::pair();
    let own = peers::parse(&store.own_device_binding().unwrap())
        .unwrap()
        .binding;
    let author = event::account(&own);
    let device = peers::fingerprint(&own).unwrap();
    let conversation = event::direct_reference(author, author);
    for batch in 0..100u64 {
        let tx = store
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        for n in batch * 1000 + 1..=(batch + 1) * 1000 {
            let mut id = [0; 32];
            id[..8].copy_from_slice(&n.to_be_bytes());
            ingest(
                &tx,
                &store.key,
                Entry {
                    conversation,
                    author,
                    identity: own.identity,
                    timestamp: now,
                    seen: now,
                    operation: Operation {
                        id,
                        version: Version { device, counter: n },
                        action: Action::Post {
                            body: Body::Text(format!("synthetic message {n}")),
                            reply: None,
                            thread: None,
                            expires_at: None,
                            view_once: false,
                        },
                    },
                },
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let start = Instant::now();
    let mut cursor = None;
    let mut count = 0;
    let mut pages = Vec::new();
    loop {
        let start = Instant::now();
        let page = store
            .conversation_page(conversation, cursor, None, Some("synthetic"), now)
            .unwrap();
        pages.push(start.elapsed());
        count += page.messages.len();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(count, 100000);
    pages.sort();
    let p95 = pages[(pages.len() * 95).div_ceil(100) - 1];
    println!("100,000 encrypted messages: filtered 64-candidate page p95={p95:?}, full streamed scan={:?}",start.elapsed());
    assert!(p95 < Duration::from_millis(100));
}

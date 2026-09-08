use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, trust},
    network::tests::Fixture,
};
use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use sigil_crypto::Secret32;
use sigil_protocol::{
    services::{Kind, Provider},
    text::{action::Reference, service::ResultData, Text},
};
use sigil_server::{
    service_config::{Configure, EntryUpdate, SecretUpdate},
    store::Store,
};
use std::sync::{atomic::AtomicUsize, Arc};
#[path = "../../server/tests/fixtures/maps.rs"]
mod map_fixture;

#[test]
fn native_https_maps_return_local_tiles_and_assets() {
    let (dir, _fixture, alice, _bob, _) = pair();
    let mut store = Store::open(&dir.path().join("server.db")).unwrap();
    let settings = map_fixture::maps(dir.path());
    let client = alice.connected_client().unwrap();
    assert!(!client.map_availability().unwrap().enabled);
    store
        .configure_maps(sigil_server::maps::Configure {
            expected_revision: 0,
            settings: Some(settings),
        })
        .unwrap();
    assert!(client.map_availability().unwrap().enabled);
    let style: Value = serde_json::from_slice(&client.map_style().unwrap()).unwrap();
    assert_eq!(
        style["sources"]["local"]["url"],
        "/client/v0/maps/tiles.json"
    );
    assert_eq!(
        client.map_metadata().unwrap()["tiles"][0],
        "/client/v0/maps/tiles/{z}/{x}/{y}"
    );
    assert_eq!(&*client.map_asset("sprite.json").unwrap(), b"{}");
    assert!(client.map_asset("../style.json").is_err());
    let tile = client.map_tile(0, 0, 0).unwrap().unwrap();
    assert_eq!(&*tile.bytes, &[0x1a, 0]);
    assert!(tile.encoding.is_none());
    assert!(client.map_tile(1, 0, 0).unwrap().is_none());
    store
        .configure_maps(sigil_server::maps::Configure {
            expected_revision: 1,
            settings: None,
        })
        .unwrap();
    assert!(!client.map_availability().unwrap().enabled);
    assert!(client.map_tile(0, 0, 0).is_err());
}

#[test]
fn dictionary_cache_refresh_and_configuration_cutover_use_disclosed_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let fixture=Fixture::local_provider(Router::new().route("/definition/{word}",axum::routing::get(move||{let counter=counter.clone();async move{let n=counter.fetch_add(1,Ordering::SeqCst);Json(json!({"en":[{"partOfSpeech":"Noun","definitions":[{"definition":format!("Synthetic definition {n}")}]}]}))}})));
    let (dir, _server, alice, _bob, _) = pair();
    let provider = Provider {
        id: "dictionary".into(),
        kind: Kind::Wiktionary,
        endpoint: format!("https://127.0.0.1:{}/definition", fixture.port()),
        attribution: Text::plain("Synthetic dictionary", Default::default()).unwrap(),
        version: None,
        source_url: None,
    };
    let mut store = Store::open(&dir.path().join("server.db")).unwrap();
    let update = || EntryUpdate {
        provider: provider.clone(),
        secret: SecretUpdate::Clear,
        exceptions: vec![sigil_server::egress::Exception {
            host: "127.0.0.1".into(),
            port: fixture.port(),
            networks: vec!["127.0.0.1/32".into()],
            root_ca: Some(include_bytes!("../../server/tests/fixtures/provider-ca.der").to_vec()),
        }],
    };
    store
        .configure_services(Configure {
            expected_revision: 0,
            per_account_daily: 20,
            total_daily: 20,
            providers: vec![update()],
        })
        .unwrap();
    let client = alice.connected_client().unwrap();
    let mut request = Resolve {
        revision: 1,
        provider: provider.clone(),
        query: Query::Define {
            word: "synthetic".into(),
            language: "en".into(),
        },
        refresh: false,
    };
    let first = client.resolve_service(&request).unwrap();
    assert!(client.resolve_service(&request).unwrap() == first);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    request.refresh = true;
    let refreshed = client.resolve_service(&request).unwrap();
    assert!(refreshed != first);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    request.refresh = false;
    assert!(client.resolve_service(&request).unwrap() == refreshed);
    store
        .configure_services(Configure {
            expected_revision: 1,
            per_account_daily: 21,
            total_daily: 21,
            providers: vec![update()],
        })
        .unwrap();
    assert!(client.resolve_service(&request).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    request.revision = 2;
    client.resolve_service(&request).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    request.provider.endpoint.push_str("-changed");
    assert!(client.resolve_service(&request).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn translation(value: &str) -> Query {
    Query::Translate {
        text: Text::plain(value, Default::default()).unwrap(),
        source: None,
        target: "es".into(),
    }
}
fn flush(client: &mut ClientStore, now: u64) {
    for _ in 0..3 {
        if let Some(attempt) = client.resume_send_intents_online(now).unwrap().pop() {
            client
                .send_pending_online(attempt.result.unwrap(), now)
                .unwrap();
            return;
        }
    }
    panic!("no send intent");
}
#[test]
fn service_disclosure_cancellation_restart_and_encrypted_delivery_do_not_repeat_queries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let provider_fixture=Fixture::local_provider(Router::new().route("/translate",post(move|Json(body):Json<Value>|{let calls=counter.clone();let cancel=signal.clone();async move{calls.fetch_add(1,Ordering::SeqCst);if body["q"]=="cancel"{cancel.store(true,Ordering::Release);}Json(json!({"translatedText":"Synthetic result","detectedLanguage":{"language":"en"}}))}})));
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    alice.enable_history_recovery().unwrap();
    bob.enable_history_recovery().unwrap();
    let provider = Provider {
        id: "synthetic".into(),
        kind: Kind::LibreTranslate,
        endpoint: format!("https://127.0.0.1:{}/translate", provider_fixture.port()),
        attribution: Text::plain("Synthetic provider", Default::default()).unwrap(),
        version: None,
        source_url: None,
    };
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    server
        .configure_services(Configure {
            expected_revision: 0,
            per_account_daily: 20,
            total_daily: 20,
            providers: vec![EntryUpdate {
                provider: provider.clone(),
                secret: SecretUpdate::Clear,
                exceptions: vec![sigil_server::egress::Exception {
                    host: "127.0.0.1".into(),
                    port: provider_fixture.port(),
                    networks: vec!["127.0.0.1/32".into()],
                    root_ca: Some(
                        include_bytes!("../../server/tests/fixtures/provider-ca.der").to_vec(),
                    ),
                }],
            }],
        })
        .unwrap();
    let catalog = alice.connected_client().unwrap().service_catalog().unwrap();
    let disclosure = alice
        .prepare_service_query(
            [181; 32],
            &catalog,
            "synthetic",
            translation("Hello"),
            false,
        )
        .unwrap();
    assert_eq!(disclosure.home_server, "chat.example");
    assert!(disclosure.request.provider == provider);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    cancelled.store(true, Ordering::Release);
    assert!(matches!(
        alice.resolve_service_query([181; 32], &cancelled),
        Err(Error::Cancelled)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    cancelled.store(false, Ordering::Release);
    let result = alice.resolve_service_query([181; 32], &cancelled).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(alice.resolve_service_query([181; 32], &cancelled).unwrap() == result);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        alice.prepare_service_query(
            [181; 32],
            &catalog,
            "synthetic",
            translation("Changed"),
            false
        ),
        Err(Error::Conflict)
    ));
    let Resolved::Snapshot(snapshot) = &result else {
        panic!()
    };
    let at = snapshot.resolved_at.max(now);
    let card = alice.service_card([181; 32], [182; 32], at).unwrap();
    let conversation = alice.direct_conversation(b).unwrap();
    alice.queue_peer_card(b, &card, at).unwrap();
    flush(&mut alice, at);
    alice.discard_service_query([181; 32]).unwrap();
    bob.accept_delivery(&next(&bob)).unwrap();
    let state = bob
        .card_state(conversation, Reference::of(&card).unwrap())
        .unwrap();
    assert!(
        matches!(&state.card.content,Construct::Service(value) if matches!(&value.content,ResultData::Translation{translated,..} if translated.body()=="Synthetic result"))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM service_queries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice
        .prepare_service_query(
            [183; 32],
            &catalog,
            "synthetic",
            translation("cancel"),
            false,
        )
        .unwrap();
    assert!(matches!(
        alice.resolve_service_query([183; 32], &cancelled),
        Err(Error::Cancelled)
    ));
    assert!(alice.service_card([183; 32], [184; 32], at).is_err());
    cancelled.store(false, Ordering::Release);
    alice
        .prepare_service_query(
            [185; 32],
            &catalog,
            "synthetic",
            translation("commit"),
            false,
        )
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON service_queries BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.resolve_service_query([185; 32], &cancelled),
        Err(Error::Storage(_))
    ));
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert!(alice.service_card([185; 32], [186; 32], at).is_err());
    alice.resolve_service_query([185; 32], &cancelled).unwrap();
    let stored = std::fs::read(dir.path().join("alice.db")).unwrap();
    assert!(!stored
        .windows(b"Synthetic result".len())
        .any(|v| v == b"Synthetic result"));
}

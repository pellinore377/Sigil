use super::*;
use crate::{
    egress::tests::Fixture,
    service_config::{Configure, EntryUpdate, SecretUpdate},
    store::Store,
};
use axum::{
    extract::RawQuery,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
fn entry(fixture: &Fixture, kind: Kind, path: &str) -> StoredEntry {
    StoredEntry {
        provider: sigil_protocol::services::Provider {
            id: "synthetic".into(),
            kind,
            endpoint: fixture.uri("127.0.0.1", path),
            attribution: text("Synthetic provider").unwrap(),
            version: None,
            source_url: None,
        },
        secret: Some(Zeroizing::new("synthetic-key".into())),
        exceptions: vec![fixture.exception("127.0.0.1")],
    }
}
fn forecast() -> Value {
    let conditions = json!({"time":1000,"temperature_2m":12.25,"apparent_temperature":11.0,"relative_humidity_2m":55,"precipitation":0.2,"weather_code":2,"wind_speed_10m":3.5,"wind_direction_10m":90});
    let mut hourly = json!({});
    for (key, value) in conditions.as_object().unwrap() {
        hourly[key] = json!((0..168)
            .map(|i| if key == "time" {
                json!(1000 + i * 3600)
            } else {
                value.clone()
            })
            .collect::<Vec<_>>());
    }
    hourly["precipitation_probability"] = json!(vec![30; 168]);
    hourly["uv_index"] = json!(vec![2.5; 168]);
    json!({"timezone":"UTC","current_units":{"temperature_2m":"°C","wind_speed_10m":"m/s","precipitation":"mm","time":"unixtime"},"hourly_units":{"temperature_2m":"°C","wind_speed_10m":"m/s","precipitation":"mm","time":"unixtime"},"daily_units":{"temperature_2m_min":"°C","temperature_2m_max":"°C","time":"unixtime"},"current":conditions,"hourly":hourly,"daily":{"time":(0..7).map(|i|1000+i*86400).collect::<Vec<_>>(),"temperature_2m_min":vec![5;7],"temperature_2m_max":vec![15;7],"weather_code":vec![2;7],"precipitation_probability_max":vec![30;7]}})
}
#[test]
fn configured_https_providers_produce_bounded_canonical_snapshots() {
    let _network = crate::egress::tests::NETWORK.lock().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let weather_calls = calls.clone();
    let app=Router::new()
        .route("/google",post(|headers:HeaderMap,Json(body):Json<Value>|async move{assert_eq!(headers["x-goog-api-key"],"synthetic-key");assert_eq!(body["q"],"Hello");assert_eq!(body["format"],"text");Json(json!({"data":{"translations":[{"translatedText":"Hola &amp; &lt;world&gt;","detectedSourceLanguage":"en"}]}}))}))
        .route("/libre",post(|Json(body):Json<Value>|async move{assert_eq!(body["api_key"],"synthetic-key");assert_eq!(body["source"],"auto");Json(json!({"translatedText":"Hola","detectedLanguage":{"language":"en"}}))}))
        .route("/define/{word}",get(||async{Json(json!({"en":[{"partOfSpeech":"Noun","definitions":[{"definition":"A <b>synthetic</b> &amp; safe definition.","examples":["An <i>example</i>."]}]}]}))}))
        .route("/index",post(|Json(body):Json<Value>|async move{Json(ResultData::Definition{word:text(body["word"].as_str().unwrap()).unwrap(),language:"en".into(),pronunciation:None,audio_url:None,senses:vec![Sense{part_of_speech:text("noun").unwrap(),definition:text("Synthetic local extract").unwrap(),example:None,etymology:Some(text("Synthetic origin").unwrap()),synonyms:vec![],antonyms:vec![]}]})}))
        .route("/geocode",get(|RawQuery(query):RawQuery|async move{assert!(query.unwrap().contains("name=Synthetic%20%26%20town"));Json(json!({"results":[{"name":"Synthetic east","latitude":10,"longitude":20,"timezone":"UTC"},{"name":"Synthetic west","latitude":11,"longitude":21,"timezone":"UTC"}]}))}))
        .route("/weather",get(move|RawQuery(query):RawQuery|{let calls=weather_calls.clone();async move{calls.fetch_add(1,Ordering::SeqCst);let query=query.unwrap();assert!(query.contains("timezone=UTC"));assert!(query.contains("wind_speed_unit=ms"));Json(forecast())}}));
    let fixture = Fixture::local(app);
    let translation = Query::Translate {
        text: text("Hello").unwrap(),
        source: None,
        target: "es".into(),
    };
    for (kind, path) in [
        (Kind::GoogleTranslate, "/google"),
        (Kind::LibreTranslate, "/libre"),
        (Kind::Wiktionary, "/define"),
        (Kind::DictionaryIndex, "/index"),
    ] {
        let query = if matches!(kind, Kind::GoogleTranslate | Kind::LibreTranslate) {
            translation.clone()
        } else {
            Query::Define {
                word: "synthetic".into(),
                language: "en".into(),
            }
        };
        let Resolved::Snapshot(snapshot) =
            resolve(&entry(&fixture, kind, path), &query, 1000).unwrap()
        else {
            panic!()
        };
        let card = sigil_protocol::text::structured::Card {
            id: [1; 32],
            creator: [2; 32],
            created_at: 1000,
            content: sigil_protocol::text::structured::Construct::Service(snapshot.clone()),
        };
        let bytes = card.to_bytes().unwrap();
        assert!(sigil_protocol::text::structured::Card::from_bytes(&bytes).unwrap() == card);
        if kind == Kind::GoogleTranslate {
            assert!(
                matches!(&snapshot.content,ResultData::Translation{translated,..} if translated.body()=="Hola & <world>")
            );
        }
        if kind == Kind::Wiktionary {
            assert!(
                matches!(&snapshot.content,ResultData::Definition{senses,..} if senses[0].definition.body()=="A synthetic & safe definition.")
            );
        }
    }
    let Resolved::Places(places) = resolve(
        &entry(&fixture, Kind::Geocoder, "/geocode"),
        &Query::Locate {
            name: "Synthetic & town".into(),
            language: "en".into(),
        },
        1000,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(places.len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let Resolved::Snapshot(snapshot) = resolve(
        &entry(&fixture, Kind::OpenMeteo, "/weather"),
        &Query::Weather {
            place: places[0].clone(),
            forecast: true,
        },
        1000,
    )
    .unwrap() else {
        panic!()
    };
    let ResultData::Weather(weather) = &snapshot.content else {
        panic!()
    };
    assert_eq!(weather.hours.len(), 168);
    assert_eq!(weather.days.len(), 7);
    assert_eq!(weather.current.temperature_mc, 12250);
    assert_eq!(weather.current.wind_mms, 3500);
    assert_eq!(weather.current.precipitation_um, Some(200));
    let card = sigil_protocol::text::structured::Card {
        id: [1; 32],
        creator: [2; 32],
        created_at: 1000,
        content: sigil_protocol::text::structured::Construct::Service(snapshot),
    };
    assert!(card.to_bytes().unwrap().len() <= sigil_protocol::text::MAX_WIRE_BYTES);
}
#[test]
fn services_reject_redirects_private_targets_wrong_units_and_stalled_responses() {
    let _network = crate::egress::tests::NETWORK.lock().unwrap();
    let fixture = Fixture::local(
        Router::new()
            .route(
                "/redirect",
                post(|| async {
                    (
                        axum::http::StatusCode::FOUND,
                        [("location", "https://127.0.0.1/private")],
                    )
                }),
            )
            .route(
                "/slow",
                post(|| async {
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                    Json(json!({}))
                }),
            )
            .route(
                "/weather/{section}",
                get(
                    |axum::extract::Path(section): axum::extract::Path<String>| async move {
                        let mut value = forecast();
                        let key = if section == "daily_units" {
                            "temperature_2m_min"
                        } else {
                            "wind_speed_10m"
                        };
                        value[section][key] = json!("wrong unit");
                        Json(value)
                    },
                ),
            ),
    );
    let query = Query::Translate {
        text: text("Synthetic").unwrap(),
        source: None,
        target: "es".into(),
    };
    assert!(matches!(
        resolve(
            &entry(&fixture, Kind::LibreTranslate, "/redirect"),
            &query,
            1000
        ),
        Err(Error::Unavailable)
    ));
    let mut denied = entry(&fixture, Kind::LibreTranslate, "/redirect");
    denied.exceptions.clear();
    assert!(matches!(
        resolve(&denied, &query, 1000),
        Err(Error::Configuration)
    ));
    let start = std::time::Instant::now();
    assert!(matches!(
        resolve(
            &entry(&fixture, Kind::LibreTranslate, "/slow"),
            &query,
            1000
        ),
        Err(Error::Unavailable)
    ));
    assert!(start.elapsed() < std::time::Duration::from_millis(3800));
    let query = Query::Weather {
        place: Place {
            name: "Synthetic".into(),
            coordinates: weather::coordinates(&json!(0), &json!(0)).unwrap(),
            timezone: "UTC".into(),
            country: None,
            region: None,
        },
        forecast: true,
    };
    for section in ["current_units", "hourly_units", "daily_units"] {
        assert!(matches!(
            resolve(
                &entry(&fixture, Kind::OpenMeteo, &format!("/weather/{section}")),
                &query,
                1000
            ),
            Err(Error::Invalid)
        ));
    }
}
#[test]
fn provider_consent_and_quotas_survive_restart_and_failed_commits() {
    use crate::store::StoreError;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = Store::open(&path).unwrap();
    store
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: serde_json::from_value(json!({"server_name":"chat.example"})).unwrap(),
        })
        .unwrap();
    let invitation = store
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: "synthetic".into(),
                expires_in_seconds: 60,
            },
            1000,
        )
        .unwrap();
    let credential = crate::auth::random_secret().unwrap();
    store
        .enroll(
            sigil_protocol::accounts::Enrollment {
                invitation: invitation.secret,
                device_credential: credential.clone(),
                device_label: "Synthetic".into(),
            },
            1000,
        )
        .unwrap();
    let provider = sigil_protocol::services::Provider {
        id: "synthetic".into(),
        kind: Kind::LibreTranslate,
        endpoint: "https://provider.example/translate".into(),
        attribution: text("Synthetic").unwrap(),
        version: None,
        source_url: None,
    };
    let config = store
        .configure_services(Configure {
            expected_revision: 0,
            per_account_daily: 2,
            total_daily: 3,
            providers: vec![EntryUpdate {
                provider: provider.clone(),
                secret: SecretUpdate::Set(Zeroizing::new("synthetic-key".into())),
                exceptions: vec![],
            }],
        })
        .unwrap();
    assert!(!serde_json::to_string(&config)
        .unwrap()
        .contains("synthetic-key"));
    let mut request = sigil_protocol::services::Resolve {
        revision: config.revision,
        provider,
        query: Query::Translate {
            text: text("Hello").unwrap(),
            source: None,
            target: "es".into(),
        },
        refresh: false,
    };
    request.provider.endpoint = "https://different.example/translate".into();
    assert!(matches!(
        store.prepare_service(&credential, &request, 1000),
        Err(StoreError::Conflict)
    ));
    request.provider.endpoint = "https://provider.example/translate".into();
    store.0.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON service_budgets BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        store.prepare_service(&credential, &request, 1000),
        Err(StoreError::Database(_))
    ));
    store.0.execute_batch("DROP TRIGGER fail").unwrap();
    for _ in 0..2 {
        store.prepare_service(&credential, &request, 1000).unwrap();
    }
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(matches!(
        store.prepare_service(&credential, &request, 1000),
        Err(StoreError::Forbidden)
    ));
    store
        .prepare_service(&credential, &request, 1000 + 86400)
        .unwrap();
    let mut changed = request.provider.clone();
    changed.endpoint = "https://new.example/translate".into();
    assert!(matches!(
        store.configure_services(Configure {
            expected_revision: 1,
            per_account_daily: 2,
            total_daily: 3,
            providers: vec![EntryUpdate {
                provider: changed,
                secret: SecretUpdate::Keep,
                exceptions: vec![]
            }]
        }),
        Err(StoreError::Invalid(_))
    ));
}

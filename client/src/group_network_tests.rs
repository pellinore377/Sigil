use super::*;
use crate::{
    groups as domain,
    network::tests::{Fixture, CA},
};
use axum::{
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use sigil_crypto::{private_credentials::Issuer, IdentityKey, Secret32};

fn client(fixture: &Fixture) -> HttpsClient {
    HttpsClient::new(
        "chat.example",
        fixture.port(),
        &"ab".repeat(32),
        &[CA.to_vec()],
    )
    .unwrap()
}

#[test]
fn private_authority_https_issues_credentials_and_orders_a_real_signed_membership_change() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let configured = server
        .configure_groups(groups::Configure {
            expected_revision: 0,
            enabled: true,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    // The synthetic administrator supplies the initial signing fingerprint.
    let bytes = decode(configured.authority.as_deref().unwrap(), 431).unwrap();
    let fingerprint = sigil_crypto::private_group::authority_fingerprint(
        bytes[bytes.len() - 96..bytes.len() - 64]
            .try_into()
            .unwrap(),
    );
    let https = alice.connected_client().unwrap();
    let profile = https.group_authority(fingerprint, None).unwrap();
    let own = alice.own_device_binding().unwrap();
    https
        .publish_device_binding(&sigil_protocol::device::Statement {
            statement: hex(&own),
        })
        .unwrap();
    let credential = https.group_credential(&profile, &own, now).unwrap();
    let own_bob = bob.own_device_binding().unwrap();
    let bob_http = bob.connected_client().unwrap();
    bob_http
        .publish_device_binding(&sigil_protocol::device::Statement {
            statement: hex(&own_bob),
        })
        .unwrap();
    let bob_credential = bob_http.group_credential(&profile, &own_bob, now).unwrap();
    let group = alice.create_group(fingerprint).unwrap();
    let genesis = alice.group_genesis(group).unwrap();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &genesis).unwrap();
    let key = GroupKey::from_master(Secret32::from_bytes([3; 32])).unwrap();
    let encryption = sigil_crypto::storage::StorageKey::new(Secret32::from_bytes([4; 32])).unwrap();
    let encrypted = encryption.seal(&genesis, &group).unwrap();
    let member = |binding: &[u8], admin| groups::Member {
        ciphertext: hex(&key.ciphertext(
            &profile
                .issuance(binding, (now / 86400) as u32)
                .unwrap()
                .attributes()
                .unwrap(),
        )),
        admin,
    };
    let create = Operation::Create {
        public: hex(&key.public()),
        head: hex(&alice.group_status(group).unwrap().state.head()),
        control: hex(&encrypted),
        members: vec![member(&own, true)],
    };
    https
        .group_request(&profile, &credential, &key, group, &create, now - 61)
        .unwrap();
    let before = alice.group_status(group).unwrap().state;
    let change = alice
        .prepare_group_change(
            group,
            domain::Change::Add(
                domain::Member::new(
                    [2; 32],
                    domain::Role::Member,
                    std::slice::from_ref(&own_bob),
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let change = bob.approve_group_proposal(group, &change).unwrap();
    let change = alice.approve_group_proposal(group, &change).unwrap();
    let proposal = before.proposal_from_bytes(&change).unwrap();
    let mut members = vec![member(&own, true), member(&own_bob, false)];
    members.sort_by(|a, b| a.ciphertext.cmp(&b.ciphertext));
    let advance = Operation::Advance {
        predecessor: hex(&before.head()),
        head: hex(&proposal.head()),
        revision: 1,
        control: hex(&encryption.seal(&change, &group).unwrap()),
        members,
    };
    let receipt = https
        .group_request(&profile, &credential, &key, group, &advance, now)
        .unwrap();
    let retry = https
        .group_request(&profile, &credential, &key, group, &advance, now)
        .unwrap();
    assert_eq!(retry.commits[0].receipt, receipt.commits[0].receipt);
    let receipt = decode(receipt.commits[0].receipt.as_deref().unwrap(), 208).unwrap();
    assert_eq!(
        alice
            .commit_group_proposal(group, &change, &receipt)
            .unwrap(),
        domain::CommitResult::Applied
    );
    let page = bob_http
        .group_request(
            &profile,
            &bob_credential,
            &key,
            group,
            &Operation::Read { from_revision: 1 },
            now,
        )
        .unwrap();
    let opened = encryption
        .open(
            &decode(&page.commits[0].control, groups::MAX_CONTROL).unwrap(),
            &group,
        )
        .unwrap();
    assert_eq!(
        bob.commit_group_proposal(
            group,
            &opened,
            &decode(page.commits[0].receipt.as_deref().unwrap(), 208).unwrap()
        )
        .unwrap(),
        domain::CommitResult::Applied
    );
    assert_eq!(
        alice.group_status(group).unwrap().state.head(),
        bob.group_status(group).unwrap().state.head()
    );
    assert!(https.group_authority(fingerprint, Some([0; 32])).is_err());
}

#[test]
fn anonymous_https_omits_account_credentials_and_rejects_substituted_receipts() {
    let identity = IdentityKey::generate().unwrap();
    let issuer = Issuer::generate().unwrap();
    let profile = Authority::sign("chat.example", 1, issuer.public(), &identity).unwrap();
    let attributes = sigil_crypto::private_credentials::Attributes::for_uid(&[1; 16]).unwrap();
    let now = crate::schedule::clock().unwrap();
    let day = (now / 86400) as u32;
    let response = issuer.issue(&attributes, day, b"synthetic").unwrap();
    let credential =
        Credential::accept(profile.issuer(), attributes, day, b"synthetic", &response).unwrap();
    let key = GroupKey::from_master(Secret32::from_bytes([2; 32])).unwrap();
    let encoded = hex(&profile.to_bytes());
    let fingerprint = profile.fingerprint();
    let profile_id = hex(&profile.id());
    let identity = std::sync::Arc::new(identity);
    let app = Router::new()
        .route(
            "/groups/v0/authority",
            get(move |headers: HeaderMap| async move {
                assert!(!headers.contains_key(header::AUTHORIZATION));
                assert!(!headers.contains_key(header::COOKIE));
                Json(encoded)
            }),
        )
        .route(
            "/groups/v0/request",
            post(
                move |headers: HeaderMap, Json(request): Json<groups::Request>| async move {
                    assert!(!headers.contains_key(header::AUTHORIZATION));
                    assert!(!headers.contains_key(header::COOKIE));
                    let Operation::Advance {
                        head,
                        revision,
                        control,
                        ..
                    } = request.operation
                    else {
                        panic!()
                    };
                    // Valid signature, wrong predecessor: transport must not accept it.
                    let wrong =
                        Receipt::sign([3; 32], [99; 32], id(&head).unwrap(), revision, &identity)
                            .unwrap();
                    Json(Reply {
                        proposals: Vec::new(),
                        invitation: None,
                        authority: profile_id,
                        group: hex(&[3; 32]),
                        revision,
                        head: head.clone(),
                        restored: false,
                        commits: vec![groups::Commit {
                            revision,
                            head,
                            control,
                            receipt: Some(hex(&wrong.to_bytes())),
                        }],
                    })
                },
            ),
        );
    let fixture = Fixture::new(app);
    let client = client(&fixture);
    client
        .group_authority(fingerprint, Some(profile.id()))
        .unwrap();
    let operation = Operation::Advance {
        predecessor: hex(&[4; 32]),
        head: hex(&[5; 32]),
        revision: 1,
        control: "ab".into(),
        members: vec![],
    };
    assert!(matches!(
        client.group_request(&profile, &credential, &key, [3; 32], &operation, now),
        Err(Error::InvalidResponse)
    ));
}

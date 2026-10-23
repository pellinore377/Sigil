use super::*;
use crate::conversation::{self, Action, Operation, PageChange};
use serde::de::DeserializeOwned;

fn round<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
    let json = serde_json::to_string(value).unwrap();
    assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value);
    json
}
fn grant() -> Grant {
    Grant {
        host: "chat.example".into(),
        id: [1; 32],
        epoch: 2,
        key: [2; 32],
        access: [3; 32],
        manage: None,
    }
}

#[test]
fn route_bodies_round_trip_as_lowercase_hex_and_reject_unknown_fields() {
    let create = Create {
        id: Bytes([1; 32]),
        access_hash: Bytes([2; 32]),
        manage_hash: Bytes([3; 32]),
        expires_at: None,
        snapshot: Sealed(vec![0xab; 40]),
    };
    let json = round(&create);
    assert!(json.contains(&"01".repeat(32)) && json.contains(&"ab".repeat(40)));
    create.validate().unwrap();
    assert!(serde_json::from_str::<Create>(&json.replace("abab", "ABAB")).is_err());
    assert!(serde_json::from_str::<Create>(&json.replacen("0101", "010", 1)).is_err());
    let extra = json.replacen('{', "{\"owner\":1,", 1);
    assert!(serde_json::from_str::<Create>(&extra).is_err());
    let append = Append {
        epoch: 1,
        base: 7,
        request: Bytes([9; 16]),
        body: Sealed(vec![1; 64]),
    };
    assert_eq!(
        round(&append),
        format!(
            "{{\"epoch\":1,\"base\":7,\"request\":\"{}\",\"body\":\"{}\"}}",
            "09".repeat(16),
            "01".repeat(64)
        )
    );
    round(&Appended {
        seq: 8,
        missed: vec![Op {
            seq: 8,
            base: 6,
            epoch: 1,
            created: 1,
            request: Bytes([4; 16]),
            body: Sealed(vec![5; 3]),
        }],
    });
    round(&Status {
        epoch: 1,
        head: 3,
        snapshot_seq: 0,
        bytes: 99,
        expires_at: Some(5),
    });
    round(&Ops {
        epoch: 1,
        head: 0,
        ops: vec![],
    });
    round(&PutSnapshot {
        epoch: 1,
        seq: 3,
        body: Sealed(vec![1]),
    });
    round(&Snapshot {
        version: 1,
        seq: 3,
        epoch: 1,
        created: 4,
        body: Sealed(vec![1]),
        label: None,
    });
    round(&Versions {
        versions: vec![Version {
            version: 2,
            seq: 3,
            epoch: 1,
            created: 4,
            bytes: 5,
            label: Some(Sealed(vec![6])),
        }],
    });
    round(&Label {
        label: Sealed(vec![1]),
    });
    round(&PutPresence {
        epoch: 1,
        slot: Bytes([7; 16]),
        body: Sealed(vec![1]),
    });
    round(&Presence {
        epoch: 1,
        entries: vec![PresenceEntry {
            slot: Bytes([7; 16]),
            body: Sealed(vec![1]),
            updated: 2,
        }],
    });
    round(&Rotate {
        expected_epoch: 1,
        access_hash: Bytes([1; 32]),
        manage_hash: Bytes([2; 32]),
        seq: 4,
        snapshot: Sealed(vec![1]),
    });
    round(&Rotated { epoch: 2 });
    round(&Cursor {
        cursor: 1,
        selection_end: 4,
    });
    assert_eq!(
        serde_json::from_str::<OpsQuery>("{\"after\":4}").unwrap(),
        OpsQuery { after: 4, wait: 0 }
    );
    assert_eq!(
        OpsQuery {
            after: 0,
            wait: 600
        }
        .wait_seconds(),
        MAX_WAIT_SECONDS
    );
}

#[test]
fn route_bodies_enforce_sizes_epochs_and_distinct_tokens() {
    let append = |epoch, base, len| Append {
        epoch,
        base,
        request: Bytes([1; 16]),
        body: Sealed(vec![1; len]),
    };
    append(1, 0, MAX_OP_BYTES).validate().unwrap();
    for bad in [
        append(0, 0, 1),
        append(1, 0, 0),
        append(1, 0, MAX_OP_BYTES + 1),
        append(1, u64::MAX, 1),
        append(u64::MAX, 0, 1),
    ] {
        assert!(bad.validate().is_err());
    }
    let snapshot = |len| PutSnapshot {
        epoch: 1,
        seq: 0,
        body: Sealed(vec![1; len]),
    };
    snapshot(MAX_SNAPSHOT_BYTES).validate().unwrap();
    assert!(snapshot(MAX_SNAPSHOT_BYTES + 1).validate().is_err());
    let mut create = Create {
        id: Bytes([1; 32]),
        access_hash: Bytes([2; 32]),
        manage_hash: Bytes([2; 32]),
        expires_at: None,
        snapshot: Sealed(vec![1]),
    };
    assert!(create.validate().is_err());
    create.manage_hash = Bytes([3; 32]);
    create.expires_at = Some(0);
    assert!(create.validate().is_err());
    create.expires_at = None;
    create.id = Bytes([0; 32]);
    assert!(create.validate().is_err());
    let presence = |len| PutPresence {
        epoch: 1,
        slot: Bytes([1; 16]),
        body: Sealed(vec![1; len]),
    };
    presence(MAX_PRESENCE_BYTES).validate().unwrap();
    assert!(presence(MAX_PRESENCE_BYTES + 1).validate().is_err());
    assert!(Label {
        label: Sealed(vec![1; MAX_LABEL_BYTES + 1])
    }
    .validate()
    .is_err());
    let mut rotate = Rotate {
        expected_epoch: 1,
        access_hash: Bytes([1; 32]),
        manage_hash: Bytes([2; 32]),
        seq: 0,
        snapshot: Sealed(vec![1]),
    };
    rotate.validate().unwrap();
    rotate.expected_epoch = i64::MAX as u64;
    assert!(rotate.validate().is_err());
    rotate.expected_epoch = 1;
    rotate.manage_hash = rotate.access_hash;
    assert!(rotate.validate().is_err());
    let huge = format!("\"{}\"", "00".repeat(MAX_SNAPSHOT_BYTES + 1));
    assert!(serde_json::from_str::<Sealed>(&huge).is_err());
}

fn page_op(change: PageChange) -> Operation {
    Operation {
        id: [5; 32],
        version: conversation::Version {
            device: [6; 32],
            counter: 1,
        },
        action: Action::Page {
            page: [1; 32],
            change,
        },
    }
}

#[test]
fn page_operations_are_canonical_and_validated() {
    let edited = PageChange::Edited {
        seq: 12,
        title: "Groceries".into(),
        preview: "Milk\nEggs".into(),
        kind: PageKind::List,
        archived: false,
        done: 1,
        total: 2,
    };
    for change in [
        edited,
        PageChange::Key { grant: grant() },
        PageChange::Key {
            grant: Grant {
                manage: Some([4; 32]),
                ..grant()
            },
        },
        PageChange::KeyRequest { epoch: 2 },
        PageChange::Removed,
    ] {
        let op = page_op(change);
        let bytes = op.to_bytes().unwrap();
        assert!(Operation::from_bytes(&bytes).unwrap() == op);
        assert!(!op.ephemeral());
    }
    let mut other = grant();
    other.id = [9; 32];
    for change in [
        PageChange::Key { grant: other },
        PageChange::KeyRequest { epoch: 0 },
        PageChange::Edited {
            seq: 1,
            title: " ".into(),
            preview: String::new(),
            kind: PageKind::Page,
            archived: false,
            done: 0,
            total: 0,
        },
        PageChange::Edited {
            seq: 1,
            title: "t".into(),
            preview: String::new(),
            kind: PageKind::List,
            archived: false,
            done: 2,
            total: 1,
        },
    ] {
        assert!(page_op(change).to_bytes().is_err());
    }
    let mut zero = page_op(PageChange::Removed);
    zero.action = Action::Page {
        page: [0; 32],
        change: PageChange::Removed,
    };
    assert!(zero.to_bytes().is_err());
    let raw = page_op(PageChange::KeyRequest { epoch: 2 })
        .to_bytes()
        .unwrap();
    let text = std::str::from_utf8(&raw[6..]).unwrap();
    let unknown = text.replace("\"epoch\":2", "\"epoch\":2,\"x\":1");
    assert!(
        Operation::from_bytes(&[b"SGCO\0\x01".as_slice(), unknown.as_bytes()].concat()).is_err()
    );
    let mut bad_grant = raw.clone();
    bad_grant.truncate(6);
    bad_grant.extend(
        text.replace("KeyRequest\":{\"epoch\":2}", "Key\":{\"grant\":\"00\"}")
            .as_bytes(),
    );
    assert!(Operation::from_bytes(&bad_grant).is_err());
}

#[test]
fn earlier_operation_payloads_still_decode() {
    for json in [
        r#"{"id":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"version":{"device":[2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2],"counter":1},"action":{"Note":{"target":{"author":[3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3],"message":[4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4]},"active":true}}}"#,
        r#"{"id":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"version":{"device":[2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2],"counter":1},"action":{"Post":{"body":{"Text":"hello"},"reply":null,"thread":null,"expires_at":null,"view_once":false}}}"#,
        r#"{"id":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"version":{"device":[2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2],"counter":3},"action":{"Typing":{"active":true,"until":9}}}"#,
    ] {
        let bytes = [b"SGCO\0\x01".as_slice(), json.as_bytes()].concat();
        let op = Operation::from_bytes(&bytes).unwrap();
        assert_eq!(op.to_bytes().unwrap(), bytes);
    }
}

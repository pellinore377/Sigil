//! Opt-in container diagnostics for accepted local message envelopes.
use sigil_protocol::mailbox::Submit;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
};

#[derive(Clone, Default)]
pub(crate) struct CiphertextLog {
    sender: Option<SyncSender<Record>>,
    dropped: Arc<AtomicU64>,
}
pub(crate) struct Record {
    pub(crate) message_id: String,
    pub(crate) payload: String,
    pub(crate) sequence: i64,
}
impl CiphertextLog {
    pub(crate) fn from_env() -> Self {
        if !matches!(
            std::env::var("SIGIL_LOG_CIPHERTEXT").as_deref(),
            Ok("true" | "1")
        ) {
            return Self::default();
        }
        let (sender, receiver) = mpsc::sync_channel::<Record>(8);
        let dropped = Arc::new(AtomicU64::new(0));
        let count = dropped.clone();
        if std::thread::Builder::new()
            .name("ciphertext-log".into())
            .spawn(move || {
                let stdout = std::io::stdout();
                for record in receiver {
                    let mut output = stdout.lock();
                    let missed = count.swap(0, Ordering::Relaxed);
                    if missed != 0 {
                        let _ = writeln!(
                            output,
                            "{{\"event\":\"sigil.ciphertext_log_dropped\",\"count\":{missed}}}"
                        );
                    }
                    if record.write(&mut output).is_err() {
                        break;
                    }
                }
            })
            .is_err()
        {
            return Self::default();
        }
        Self {
            sender: Some(sender),
            dropped,
        }
    }
    pub(crate) fn capture(&self, request: &Submit) -> Option<Record> {
        self.sender.as_ref().map(|_| Record {
            message_id: request.message_id.clone(),
            payload: request.payload.clone(),
            sequence: 0,
        })
    }
    pub(crate) fn accepted(&self, record: Option<Record>, sequence: i64) {
        if let (Some(sender), Some(mut record)) = (&self.sender, record) {
            record.sequence = sequence;
            if sender.try_send(record).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn capture_for_test() -> (Self, mpsc::Receiver<Record>) {
        let (sender, receiver) = mpsc::sync_channel(8);
        (
            Self {
                sender: Some(sender),
                dropped: Arc::default(),
            },
            receiver,
        )
    }
}
impl Record {
    fn write(&self, writer: &mut impl Write) -> std::io::Result<()> {
        serde_json::to_writer(
            &mut *writer,
            &serde_json::json!({"event":"sigil.encrypted_message","message_id":self.message_id,
            "sequence":self.sequence,"bytes":self.payload.len()/2,"payload_hex":self.payload}),
        )?;
        writer.write_all(b"\n")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_are_opt_in_bounded_and_keep_the_exact_opaque_payload() {
        let request = Submit {
            message_id: "ab".repeat(32),
            recipient_device: "cd".repeat(32),
            payload: "ef".repeat(80),
            expires_at: 1,
        };
        assert!(CiphertextLog::default().capture(&request).is_none());
        let (log, records) = CiphertextLog::capture_for_test();
        for sequence in 1..=9 {
            log.accepted(log.capture(&request), sequence);
        }
        assert_eq!(log.dropped.load(Ordering::Relaxed), 1);
        let record = records.try_recv().unwrap();
        let mut output = Vec::new();
        record.write(&mut output).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["payload_hex"], request.payload);
        assert_eq!(value["bytes"], 80);
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert!(value.get("recipient_device").is_none());
    }
}

#[cfg(test)]
mod routes {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    #[tokio::test]
    async fn only_authenticated_accepted_messages_reach_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("server.db")).unwrap();
        store
            .configure(sigil_protocol::Configure {
                expected_revision: 0,
                settings: sigil_protocol::Settings {
                    server_name: "chat.example".into(),
                    default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
                    max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
                },
            })
            .unwrap();
        let now = crate::enrollment::now().unwrap();
        let mut devices = Vec::new();
        for (username, token) in [("alice", "01".repeat(32)), ("bob", "02".repeat(32))] {
            let invitation = store
                .invite(
                    sigil_protocol::accounts::InviteRequest {
                        username: username.into(),
                        expires_in_seconds: 60,
                    },
                    now,
                )
                .unwrap();
            devices.push(
                store
                    .enroll(
                        sigil_protocol::accounts::Enrollment {
                            invitation: invitation.secret,
                            device_credential: token,
                            device_label: "Synthetic".into(),
                        },
                        now,
                    )
                    .unwrap(),
            );
        }
        store
            .allow_sender(&"02".repeat(32), &devices[0].device_id, now)
            .unwrap();
        let token =
            crate::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
        let (log, records) = CiphertextLog::capture_for_test();
        let app = crate::application_with_log(store, token, log).0;
        for (credential, payload, status) in [
            ("03".repeat(32), "ef".repeat(80), StatusCode::UNAUTHORIZED),
            (
                "01".repeat(32),
                "not a packet\n".into(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("01".repeat(32), "ef".repeat(80), StatusCode::ACCEPTED),
        ] {
            let request = Submit {
                recipient_device: devices[1].device_id.clone(),
                message_id: "ab".repeat(32),
                payload: payload.clone(),
                expires_at: now + 3600,
            };
            let response = app
                .clone()
                .oneshot(
                    Request::post("/client/v0/messages")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {credential}"))
                        .body(Body::from(serde_json::to_vec(&request).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            if status == StatusCode::ACCEPTED {
                assert_eq!(records.try_recv().unwrap().payload, payload);
            } else {
                assert!(matches!(records.try_recv(), Err(mpsc::TryRecvError::Empty)));
            }
        }
    }
}

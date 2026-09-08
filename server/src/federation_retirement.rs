use super::*;
use rusqlite::Transaction;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetirePeer {
    pub expected_revision: u64,
}
#[derive(Deserialize, Serialize)]
pub struct PeerRetirement {
    pub revision: u64,
    pub complete: bool,
}
pub(super) fn pending(tx: &Transaction<'_>, server: &str) -> Result<bool, StoreError> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM federation_outbox WHERE destination=?1 AND state=0) OR EXISTS(SELECT 1 FROM mailbox WHERE remote_server=?1 AND payload IS NOT NULL) OR EXISTS(SELECT 1 FROM federation_senders WHERE server=?1 AND grant_id IS NOT NULL)", [server], |r| r.get(0))?)
}
impl Store {
    pub fn retire_federation_peer(
        &mut self,
        server: &str,
        request: RetirePeer,
    ) -> Result<PeerRetirement, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = peer(&tx, server)?.ok_or(StoreError::NotFound)?;
        let revision = if old.error.as_deref() == Some("retired") {
            if request.expected_revision != old.revision
                && request.expected_revision.checked_add(1) != Some(old.revision)
            {
                return Err(StoreError::Conflict);
            }
            old.revision
        } else {
            if request.expected_revision != old.revision {
                return Err(StoreError::Conflict);
            }
            let revision = crate::push_config::next(old.revision)?;
            let hash = Sha256::digest(
                [
                    b"Sigil/retire-federation-peer/v0".as_slice(),
                    &old.revision.to_be_bytes(),
                ]
                .concat(),
            );
            tx.execute("UPDATE federation_peers SET revision=?2,allowed=0,approval=NULL,observed=NULL,checked_at=0,not_before=0,error='retired',request_hash=?3 WHERE server=?1", (server, sql(revision)?, hash.as_slice()))?;
            revision
        };
        let jobs = tx.prepare("SELECT sender,message_id FROM federation_outbox WHERE destination=?1 AND state=0 ORDER BY sender,message_id LIMIT 64")?.query_map([server], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
        for (sender, message) in jobs {
            crate::federation_outbox::terminal(
                &tx,
                &sender,
                &message,
                4,
                None,
                Some("peer_retired"),
            )?;
        }
        let messages = tx.prepare("SELECT sequence FROM mailbox WHERE remote_server=?1 AND payload IS NOT NULL ORDER BY sequence LIMIT 64")?.query_map([server], |r| r.get::<_,i64>(0))?.collect::<Result<Vec<_>,_>>()?;
        for sequence in messages {
            crate::federation_mailbox::release_payload(&tx, sequence)?;
        }
        let grants = tx.prepare("SELECT recipient,device,grant_id,revision FROM federation_senders WHERE server=?1 AND grant_id IS NOT NULL ORDER BY recipient,device LIMIT 64")?.query_map([server], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,unsigned(r,3)?)))?.collect::<Result<Vec<_>,_>>()?;
        for (recipient, device, grant, revision) in grants {
            tx.execute(
                "INSERT INTO federation_revocations VALUES(?1,?2)",
                (&grant, &recipient),
            )?;
            tx.execute("UPDATE federation_senders SET grant_id=NULL,revision=?4,request_hash=X'' WHERE recipient=?1 AND server=?2 AND device=?3", (&recipient,server,&device,sql(crate::push_config::next(revision)?)?))?;
        }
        let complete = !pending(&tx, server)?;
        tx.commit()?;
        Ok(PeerRetirement { revision, complete })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_cancels_frozen_work_preserves_pins_and_replay_evidence_across_restart() {
        let mut pair =
            crate::federation_outbox::tests::Pair::new(1000, "chat.example", "remote.example");
        let old = pair.source.federation_peer("remote.example").unwrap();
        pair.source
            .queue_federated_message(&pair.alice, pair.request.clone(), 1000)
            .unwrap();
        let frozen = pair
            .source
            .federated_outbound(&pair.alice, &pair.request.message_id, 1000)
            .unwrap();
        pair.source.0.execute_batch("CREATE TRIGGER fail_retirement BEFORE UPDATE OF body ON federation_outbox BEGIN SELECT RAISE(ABORT,'synthetic retirement failure'); END;").unwrap();
        assert!(pair
            .source
            .retire_federation_peer(
                "remote.example",
                RetirePeer {
                    expected_revision: old.revision
                }
            )
            .is_err());
        assert!(
            pair.source
                .federation_peer("remote.example")
                .unwrap()
                .allowed
        );
        pair.source
            .0
            .execute_batch("DROP TRIGGER fail_retirement")
            .unwrap();
        let retired = pair
            .source
            .retire_federation_peer(
                "remote.example",
                RetirePeer {
                    expected_revision: old.revision,
                },
            )
            .unwrap();
        assert!(retired.complete);
        let peer = pair.source.federation_peer("remote.example").unwrap();
        assert!(!peer.allowed);
        assert_eq!(peer.pinned, old.pinned);
        assert!(matches!(
            pair.source.begin_federation_refresh("remote.example", 1000),
            Err(StoreError::Forbidden)
        ));
        let mut reopened = Store::open(&pair.dir.path().join("source.db")).unwrap();
        assert!(
            reopened
                .retire_federation_peer(
                    "remote.example",
                    RetirePeer {
                        expected_revision: old.revision
                    }
                )
                .unwrap()
                .complete
        );
        let status = reopened
            .queue_federated_message(&pair.alice, pair.request.clone(), 1000)
            .unwrap();
        assert_eq!(
            status.state,
            sigil_protocol::federation::OutboundState::Revoked
        );
        assert_eq!(status.error.as_deref(), Some("peer_retired"));
        assert_eq!(status.request_hash, frozen.request_hash);
        reopened
            .configure_federation_peer(
                "remote.example",
                ConfigurePeer {
                    expected_revision: retired.revision,
                    allowed: true,
                    port: 443,
                    approve_key: None,
                },
            )
            .unwrap();
        assert_eq!(
            reopened
                .queue_federated_message(&pair.alice, pair.request.clone(), 1000)
                .unwrap()
                .state,
            sigil_protocol::federation::OutboundState::Revoked
        );
    }
    #[test]
    fn retirement_drains_grants_in_bounded_batches_and_blocks_reenable_until_complete() {
        use sigil_protocol::federation::{ConfigureSender, RemoteSender};
        let mut p =
            crate::federation_outbox::tests::Pair::new(1000, "chat.example", "remote.example");
        for n in 1..=65 {
            p.sink
                .configure_federation_sender(
                    &p.bob,
                    ConfigureSender {
                        expected_revision: 0,
                        sender: RemoteSender {
                            server: "chat.example".into(),
                            account: format!("{n:064x}"),
                            device: format!("{n:064x}"),
                        },
                        allowed: true,
                    },
                    1000,
                )
                .unwrap();
        }
        let own = p.source.session(&p.alice, 1000).unwrap();
        let submit = sigil_protocol::federation::Submit {
            sender_account: own.account_id,
            sender_device: own.device_id,
            recipient_device: p.request.recipient_device.clone(),
            message_id: p.request.message_id.clone(),
            payload: p.request.payload.clone(),
            expires_at: p.request.expires_at,
        };
        let body = serde_json::to_vec(&submit).unwrap();
        let config = read(&p.source.0).unwrap();
        let headers = auth::sign(
            config.key.as_ref().unwrap(),
            &auth::Request {
                origin: "chat.example",
                destination: "remote.example",
                path: sigil_protocol::federation::DELIVER_PATH,
                body: &body,
            },
            1000,
            [90; 32],
        )
        .unwrap();
        p.sink
            .receive_federated_message(&body, &headers, 1000)
            .unwrap();
        assert_eq!(p.sink.mailbox_after(&p.bob, 0, 1000).unwrap().len(), 1);
        let old = p.sink.federation_peer("chat.example").unwrap();
        let first = p
            .sink
            .retire_federation_peer(
                "chat.example",
                RetirePeer {
                    expected_revision: old.revision,
                },
            )
            .unwrap();
        assert!(!first.complete);
        assert!(p.sink.mailbox_after(&p.bob, 0, 1000).unwrap().is_empty());
        let remaining: u32 = p
            .sink
            .0
            .query_row(
                "SELECT count(*) FROM federation_senders WHERE grant_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 2);
        assert!(matches!(
            p.sink.configure_federation_peer(
                "chat.example",
                ConfigurePeer {
                    expected_revision: first.revision,
                    allowed: true,
                    port: 443,
                    approve_key: None,
                }
            ),
            Err(StoreError::Busy)
        ));
        drop(p.sink);
        let mut sink = Store::open(&p.dir.path().join("sink.db")).unwrap();
        assert!(
            sink.retire_federation_peer(
                "chat.example",
                RetirePeer {
                    expected_revision: first.revision
                }
            )
            .unwrap()
            .complete
        );
        assert!(sink.federated_senders(&p.bob, 1000).unwrap().is_empty());
        sink.configure_federation_peer(
            "chat.example",
            ConfigurePeer {
                expected_revision: first.revision,
                allowed: true,
                port: 443,
                approve_key: None,
            },
        )
        .unwrap();
        assert_eq!(
            sink.federation_peer("chat.example").unwrap().pinned,
            old.pinned
        );
        assert!(sink.mailbox_after(&p.bob, 0, 1000).unwrap().is_empty());
    }
}

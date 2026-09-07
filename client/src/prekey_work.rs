//! Resume explicitly prepared publications; this does not infer server inventory.
use super::*;

#[derive(Debug)]
pub struct PrekeyAttempt {
    pub slot: Id,
    /// Original publication receipt, not evidence that the key remains unclaimed.
    pub result: Result<PublishedPrekey, Error>,
}

#[derive(Debug)]
pub enum PrekeySupply {
    Stocked {
        available: u32,
    },
    /// Finish existing ambiguous publications before allocating another slot.
    Pending {
        available: u32,
    },
    Published {
        available_before: u32,
        receipt: PublishedPrekey,
    },
}

fn generation(db: &Connection) -> Result<i64, Error> {
    Ok(
        db.query_row("SELECT coalesce(max(rowid),0) FROM prekeys", [], |r| {
            r.get(0)
        })?,
    )
}

fn cursor(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
) -> Result<(Vec<u8>, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db.query_row(
        "SELECT CASE WHEN length(state) IN (36,68) THEN state END FROM prekey_cursor WHERE id=1",
        [], |r| r.get(0),
    ).optional()?;
    let after = match &sealed {
        Some(value) => key.open(value, &binding(34, own, b"prekey scan"))?.to_vec(),
        None => Vec::new(),
    };
    if !after.is_empty() && after.len() != 32 {
        return Err(Error::InvalidStore);
    }
    Ok((after, sealed))
}

impl ClientStore {
    /// Target eight available bundles, adding at most one per call. Inventory is
    /// a server hint, never erasure authority. Existing local/lifetime limits apply.
    /// A failed HTTP/receipt commit leaves a prepared publication for the scheduler.
    pub fn replenish_prekey_online(&mut self) -> Result<PrekeySupply, Error> {
        let before = generation(&self.db)?;
        let available = self.connected_client()?.prekey_inventory()?.available;
        if available >= 8 {
            return Ok(PrekeySupply::Stocked { available });
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Reject a stale inventory response when another worker allocated a slot.
        if generation(&tx)? != before {
            return Err(Error::Conflict);
        }
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM prekey_publications WHERE retire_at IS NULL)",
            [],
            |r| r.get(0),
        )?;
        if pending {
            return Ok(PrekeySupply::Pending { available });
        }
        let mut slot = [0; 32];
        getrandom::fill(&mut slot).map_err(|_| sigil_crypto::Error::Entropy)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM prekeys WHERE id=?1)",
            [slot.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict);
        }
        prepare_in(&tx, &self.key, slot, true, 604800)?;
        tx.commit()?;
        let receipt = self.publish_prekey_online(slot)?;
        Ok(PrekeySupply::Published {
            available_before: available,
            receipt,
        })
    }
    /// Resume at most 16 explicitly prepared publications. Retention uses the local
    /// wall clock after acknowledgement, as in manual publication. Exact retries
    /// preserve the original bundle and expiry.
    /// Local errors remain per-slot results; network errors stop the pass for caller
    /// backoff. A sealed cyclic cursor prevents one failed slot starving others.
    /// An empty pass wraps it, including zero-valued IDs on the following pass.
    /// This neither creates keys nor retires private material.
    pub fn resume_prekey_publications_online(&mut self) -> Result<Vec<PrekeyAttempt>, Error> {
        self.connected_client()?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        let slots: Vec<Vec<u8>> = self.db.prepare(
            "SELECT slot FROM prekey_publications WHERE retire_at IS NULL AND slot>?1 ORDER BY slot LIMIT 16"
        )?.query_map([after], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let mut attempts = Vec::new();
        let mut next = Vec::new();
        for bytes in slots {
            let slot: Id = bytes.try_into().map_err(|_| Error::InvalidStore)?;
            let result = self.publish_prekey_online(slot);
            let stop = matches!(result, Err(Error::Network(_)));
            attempts.push(PrekeyAttempt { slot, result });
            next = slot.to_vec();
            if stop {
                break;
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cursor(&tx, &self.key, &own)?.1 == expected {
            tx.execute("INSERT INTO prekey_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
                [self.key.seal(&next, &binding(34, &own, b"prekey scan"))?])?;
        }
        tx.commit()?;
        Ok(attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{claims::tests::pair, incoming::tests::trust};
    fn reopen(path: &Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn supply_targets_eight_and_replaces_claimed_and_expired_server_stock() {
        let (dir, _fixture, mut alice, _bob, _now) = pair();
        for available in 0..8 {
            assert!(
                matches!(alice.replenish_prekey_online().unwrap(),PrekeySupply::Published { available_before, .. } if available_before == available)
            );
        }
        assert!(matches!(
            alice.replenish_prekey_online().unwrap(),
            PrekeySupply::Stocked { available: 8 }
        ));
        let device =
            sigil_protocol::device::SignedBinding::from_bytes(&alice.own_device_binding().unwrap())
                .unwrap()
                .binding
                .device;
        alice
            .connected_client()
            .unwrap()
            .claim_prekey(&transport::hex(&device), &transport::hex(&[90; 32]))
            .unwrap();
        assert_eq!(generation(&alice.db).unwrap(), 8);
        assert!(matches!(
            alice.replenish_prekey_online().unwrap(),
            PrekeySupply::Published {
                available_before: 7,
                ..
            }
        ));
        assert_eq!(generation(&alice.db).unwrap(), 9);
        let server = Connection::open(dir.path().join("server.db")).unwrap();
        server
            .execute(
                "UPDATE prekeys SET expires_at=1 WHERE device_id=?1",
                [transport::hex(&device)],
            )
            .unwrap();
        assert!(matches!(
            alice.replenish_prekey_online().unwrap(),
            PrekeySupply::Published {
                available_before: 0,
                ..
            }
        ));
        // Inventory never authorizes private-key deletion, even when it reports zero.
        assert_eq!(
            alice
                .db
                .query_row("SELECT count(state) FROM prekeys", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            10
        );
    }

    #[test]
    fn failed_supply_preparation_is_atomic_and_ambiguous_publication_blocks_new_allocation() {
        let (dir, _fixture, mut alice, _bob, _now) = pair();
        alice.db.execute_batch("CREATE TRIGGER fail_prepare BEFORE INSERT ON prekey_publications BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(matches!(
            alice.replenish_prekey_online(),
            Err(Error::Storage(_))
        ));
        assert_eq!(generation(&alice.db).unwrap(), 0);
        alice.db.execute_batch("DROP TRIGGER fail_prepare; CREATE TRIGGER fail_receipt BEFORE UPDATE ON prekey_publications BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(matches!(
            alice.replenish_prekey_online(),
            Err(Error::Storage(_))
        ));
        assert_eq!(generation(&alice.db).unwrap(), 1);
        assert!(matches!(
            alice.replenish_prekey_online().unwrap(),
            PrekeySupply::Pending { available: 1 }
        ));
        assert_eq!(generation(&alice.db).unwrap(), 1);
        alice
            .db
            .execute_batch("DROP TRIGGER fail_receipt;")
            .unwrap();
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        assert!(alice.resume_prekey_publications_online().unwrap()[0]
            .result
            .is_ok());
        assert_eq!(generation(&alice.db).unwrap(), 1);
        assert!(matches!(
            alice.replenish_prekey_online().unwrap(),
            PrekeySupply::Published {
                available_before: 1,
                ..
            }
        ));
        assert_eq!(generation(&alice.db).unwrap(), 2);
    }

    #[test]
    fn exhausted_private_slot_capacity_reports_supply_failure_without_stopping_queued_messages() {
        let (_dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        crate::incoming::tests::start(&mut alice, b, now);
        bob.receive_mailbox_online(now).unwrap();
        bob.acknowledge_incoming_online().unwrap();
        alice
            .send_text(
                [3; 32],
                [99; 32],
                "capacity does not stop messages",
                now,
                now,
            )
            .unwrap();
        for n in 0..64u8 {
            alice.create_prekey([n; 32], true).unwrap();
        }
        let step = alice.sync_step_online(now);
        assert!(step.failure.is_none());
        assert!(matches!(step.prekey_supply, Some(Err(Error::Limit))));
        assert_eq!(step.outbound[0].result.as_ref().unwrap().accepted, 1);
        let received = bob.receive_mailbox_online(now).unwrap();
        assert!(
            matches!(&received[0].result,Ok(MailboxEvent::Text(t)) if t.text().unwrap().body == "capacity does not stop messages")
        );
    }

    #[test]
    fn ambiguous_publication_and_cursor_failure_resume_without_replacing_keys() {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        trust(&mut alice, &mut bob);
        let id = alice
            .prepare_prekey_publication([0; 32], true, 3600)
            .unwrap();
        let private: Vec<u8> = alice
            .db
            .query_row(
                "SELECT state FROM prekeys WHERE id=?1",
                [[0; 32].as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        alice.db.execute_batch("CREATE TRIGGER fail_publication BEFORE UPDATE ON prekey_publications BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let first = alice.sync_step_online(now);
        assert!(first.failure.is_none());
        assert_eq!(first.prekeys.len(), 1);
        assert!(matches!(first.prekeys[0].result, Err(Error::Storage(_))));
        let device =
            sigil_protocol::device::SignedBinding::from_bytes(&alice.own_device_binding().unwrap())
                .unwrap()
                .binding
                .device;
        let claimed = bob
            .connected_client()
            .unwrap()
            .claim_prekey(&transport::hex(&device), &transport::hex(&[71; 32]))
            .unwrap();
        assert_eq!(claimed.prekey_id, transport::hex(&id));
        alice
            .db
            .execute_batch("DROP TRIGGER fail_publication;")
            .unwrap();
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        assert!(alice
            .resume_prekey_publications_online()
            .unwrap()
            .is_empty());
        alice.db.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE ON prekey_cursor BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(matches!(
            alice.resume_prekey_publications_online(),
            Err(Error::Storage(_))
        ));
        let identity = alice.identity().unwrap();
        let (publication, _, _) = load(&alice.db, &alice.key, &[0; 32], &identity).unwrap();
        assert_eq!(publication.expiry, Some(claimed.expires_at));
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(
            (now + 3600 + DELIVERY_LIFETIME..=after + 3600 + DELIVERY_LIFETIME)
                .contains(&publication.retire_at.unwrap())
        );
        assert!(!publication.retired);
        let server = Connection::open(dir.path().join("server.db")).unwrap();
        assert_eq!(
            server
                .query_row(
                    "SELECT count(*) FROM prekeys WHERE device_id=?1",
                    [transport::hex(&device)],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            server
                .query_row(
                    "SELECT request_id FROM prekeys WHERE id=?1",
                    [transport::hex(&id)],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            transport::hex(&[71; 32])
        );
        assert_eq!(
            alice
                .db
                .query_row(
                    "SELECT state FROM prekeys WHERE id=?1",
                    [[0; 32].as_slice()],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            private
        );
        alice.db.execute_batch("DROP TRIGGER fail_cursor;").unwrap();
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        assert!(alice
            .resume_prekey_publications_online()
            .unwrap()
            .is_empty());
        // Successful publication is not stock: a new claim cannot reuse the assigned key.
        assert!(bob
            .connected_client()
            .unwrap()
            .claim_prekey(&transport::hex(&device), &transport::hex(&[72; 32]))
            .is_err());
    }

    #[test]
    fn bounded_scan_includes_zero_and_publishes_beyond_corrupt_records_after_restart() {
        let (dir, _fixture, mut alice, _bob, _now) = pair();
        for n in 0..17u8 {
            alice
                .prepare_prekey_publication([n; 32], n % 2 == 0, 3600)
                .unwrap();
        }
        alice
            .db
            .execute(
                "UPDATE prekey_publications SET state=x'00' WHERE slot<?1",
                [[16; 32].as_slice()],
            )
            .unwrap();
        let first = alice.resume_prekey_publications_online().unwrap();
        assert_eq!(first.len(), 16);
        assert_eq!(first[0].slot, [0; 32]);
        assert!(first.iter().all(|a| a.result.is_err()));
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        let second = alice.resume_prekey_publications_online().unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].slot, [16; 32]);
        assert!(second[0].result.is_ok());
        assert!(alice
            .resume_prekey_publications_online()
            .unwrap()
            .is_empty());
        assert_eq!(
            alice.resume_prekey_publications_online().unwrap()[0].slot,
            [0; 32]
        );
        alice
            .db
            .execute("UPDATE prekey_cursor SET state=zeroblob(68)", [])
            .unwrap();
        assert!(alice.resume_prekey_publications_online().is_err());
    }

    #[test]
    fn migration_and_network_failure_preserve_prepared_slots_and_advance_cursor() {
        let (dir, fixture, mut alice, _bob, _now) = pair();
        for n in 0..2u8 {
            alice
                .prepare_prekey_publication([n; 32], true, 3600)
                .unwrap();
        }
        crate::test_schema::rewind(&alice.db, 37);
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        drop(fixture);
        let first = alice.resume_prekey_publications_online().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].slot, [0; 32]);
        assert!(matches!(first[0].result, Err(Error::Network(_))));
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        let second = alice.resume_prekey_publications_online().unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].slot, [1; 32]);
        assert!(matches!(second[0].result, Err(Error::Network(_))));
        assert_eq!(
            alice
                .db
                .query_row(
                    "SELECT count(*) FROM prekeys WHERE state IS NOT NULL",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(alice.db.query_row("SELECT count(*) FROM prekey_publications WHERE expires_at IS NULL AND retire_at IS NULL", [], |r| r.get::<_,i64>(0)).unwrap(),2);
    }
}

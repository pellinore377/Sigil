//! Bounded recovery work; all protocol evidence lives in the existing ledgers.
use super::*;
enum Finish {
    Cancel,
    Obsolete(u64),
}

pub struct RetryAttempt {
    pub id: Id,
    /// Server acceptance only, including an already durably recorded receipt.
    pub result: Result<Receipt, Error>,
}
fn cursor(db: &Connection, key: &StorageKey, own: &Id) -> Result<(Id, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=68 THEN state END FROM retry_cursor WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let id = match &sealed {
        Some(bytes) => key
            .open(bytes, &binding(23, own, b"retry scan"))?
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?,
        None => [0; 32],
    };
    Ok((id, sealed))
}
pub(super) fn session(id: &Id) -> Id {
    Sha256::digest([b"Sigil/retry-session/v0".as_slice(), id].concat()).into()
}
impl ClientStore {
    /// Resume at most 16 explicitly authorized outgoing recovery controls.
    /// Completed receipts are authenticated by the existing send path.
    pub fn resume_retry_controls_online(&mut self, now: u64) -> Result<Vec<RetryAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let expected:Option<Vec<u8>>=self.db.query_row("SELECT CASE WHEN length(state) IN (36,68) THEN state END FROM retry_send_cursor WHERE id=1",[],|r|r.get(0)).optional()?;
        let after = match &expected {
            Some(s) => self
                .key
                .open(s, &binding(38, &own, b"retry send"))?
                .to_vec(),
            None => Vec::new(),
        };
        if !after.is_empty() && after.len() != 32 {
            return Err(Error::InvalidStore);
        }
        let ids: Vec<Vec<u8>> = self
            .db
            .prepare("SELECT id FROM retry_outbox WHERE complete=0 AND id>?1 ORDER BY id LIMIT 16")?
            .query_map([after], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut results = Vec::new();
        let mut next = Vec::new();
        for raw in ids {
            let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
            let result = self.send_retry_request_online(id, now);
            if result.is_ok() {
                self.db.execute(
                    "UPDATE retry_outbox SET complete=1 WHERE id=?1",
                    [id.as_slice()],
                )?;
            }
            let stop = matches!(result, Err(Error::Network(_)));
            results.push(RetryAttempt { id, result });
            next = id.to_vec();
            if stop {
                break;
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<Vec<u8>> = tx
            .query_row("SELECT state FROM retry_send_cursor WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        if current == expected {
            tx.execute("INSERT INTO retry_send_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",[self.key.seal(&next,&binding(38,&own,b"retry send"))?])?;
        }
        tx.commit()?;
        Ok(results)
    }
    fn resume_retry_online(&mut self, id: Id, now: u64) -> Result<Receipt, Error> {
        let record = read(&self.db, &self.key, "retry_requests", &id)?.ok_or(Error::NotFound)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        if record.receipt.is_some()
            || request.target != device_fingerprint(&self.own_device_binding()?)?
        {
            return Err(Error::InvalidStore);
        }
        let session = session(&id);
        if self.finish_retry(id, Finish::Obsolete(now))? {
            return Err(Error::Cancelled);
        }
        match self.delivery_receipt(session, id) {
            Ok(Some(receipt)) => {
                if receipt.expires_at != request.expires_at {
                    return Err(Error::InvalidStore);
                }
                return Ok(receipt);
            }
            Ok(None) => {}
            Err(Error::NotFound) => {
                let work = self.retry_request(id, now)?;
                self.claim_prekey_online(work.claim, now)?;
                self.resend_event(id, now)?;
            }
            Err(error) => return Err(error),
        }
        // Recheck deletion and trust after any claim I/O, and send this response
        // alone rather than unrelated messages subsequently queued on its session.
        if self.finish_retry(id, Finish::Obsolete(now))? {
            return Err(Error::Cancelled);
        }
        self.retry_request(id, now)?;
        super::super::load(&self.db, &self.key, &session)?;
        let peer = peers::known(&self.db, &self.key, &record.peer)?;
        let packet =
            self.prepare_delivery(session, id, peer.binding.device, request.expires_at, now)?;
        let receipt = self.connected_client()?.submit(&packet)?;
        self.acknowledge_sent(session, id, &receipt)?;
        Ok(receipt)
    }
    /// Resume at most 16 accepted controls using a trusted caller clock. Failures
    /// remain per-item results; a sealed cyclic cursor prevents starvation. Network
    /// errors stop the batch so callers can honor Retry-After before invoking again.
    pub fn resume_retries_online(&mut self, now: u64) -> Result<Vec<RetryAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        self.connected_client()?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        let ids: Vec<Vec<u8>> = self
            .db
            .prepare(
                "SELECT id FROM retry_requests WHERE finished=0 AND id>?1 ORDER BY id LIMIT 16",
            )?
            .query_map([after.as_slice()], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut attempts = Vec::new();
        let mut next = [0; 32];
        for bytes in ids {
            let id: Id = bytes.try_into().map_err(|_| Error::InvalidStore)?;
            let result = self.resume_retry_online(id, now);
            let stop = matches!(result, Err(Error::Network(_)));
            attempts.push(RetryAttempt { id, result });
            next = id;
            if stop {
                break;
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cursor(&tx, &self.key, &own)?.1 == expected {
            tx.execute("INSERT INTO retry_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [self.key.seal(&next, &binding(23, &own, b"retry scan"))?])?;
        }
        tx.commit()?;
        Ok(attempts)
    }
}

impl ClientStore {
    /// Stop an accepted recovery request, including any prepared response.
    /// Returns false if server acceptance is already durably known. In-flight
    /// packets cannot be recalled; late receipts remain recordable.
    pub fn cancel_retry_request(&mut self, id: Id) -> Result<bool, Error> {
        self.finish_retry(id, Finish::Cancel)
    }
    fn finish_retry(&mut self, id: Id, mode: Finish) -> Result<bool, Error> {
        let own = self.own_device_binding()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = read(&tx, &self.key, "retry_requests", &id)?.ok_or(Error::NotFound)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        if record.receipt.is_some() || request.target != device_fingerprint(&own)? {
            return Err(Error::InvalidStore);
        }
        let session = session(&id);
        let receipt = match transport::receipt(&tx, &self.key, session, id) {
            Ok(value) => value,
            Err(Error::NotFound) => None,
            Err(error) => return Err(error),
        };
        if let Some(receipt) = receipt {
            if receipt.expires_at != request.expires_at {
                return Err(Error::InvalidStore);
            }
            record.finished = 1;
        } else {
            if record.finished == 1 {
                return Err(Error::Conflict);
            }
            if record.finished == 2 {
                return Ok(true);
            }
            if let Finish::Obsolete(now) = mode {
                if now == 0 || now > i64::MAX as u64 {
                    return Err(Error::Expired);
                }
                // Only an authenticated deadline or explicit retained-content
                // obsolescence is terminal. Conflicts, clock rollback, missing
                // evidence, corruption and trust failures never cancel work.
                if request.expires_at > now {
                    match inspect(&tx, &self.key, &own, record.peer, &request, now) {
                        Ok(_) => return Ok(false),
                        Err(Error::Obsolete) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            cancel_in(&tx, &self.key, &own, id, &mut record)?;
        }
        save(&tx, &self.key, "retry_requests", &id, &record)?;
        tx.commit()?;
        Ok(record.finished == 2)
    }
}
pub(super) fn cancel_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    id: Id,
    record: &mut Record,
) -> Result<(), Error> {
    let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
    let session = session(&id);
    match transport::receipt(tx, key, session, id) {
        Ok(Some(receipt)) => {
            if receipt.expires_at != request.expires_at {
                return Err(Error::InvalidStore);
            }
            record.finished = 1;
            return Ok(());
        }
        Ok(None) | Err(Error::NotFound) => {}
        Err(error) => return Err(error),
    }
    let claim = Sha256::digest([b"Sigil/retry-claim/v0".as_slice(), &id].concat()).into();
    let identity = SignedBinding::from_bytes(own)
        .map_err(|_| Error::InvalidStore)?
        .binding
        .identity;
    claims::abandon(tx, key, &claim, &identity)?;
    tx.execute(
        "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
        (session.as_slice(), id.as_slice()),
    )?;
    record.finished = 2;
    Ok(())
}

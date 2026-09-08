//! Frozen direct-text intents before any network claim or session selection.
use super::*;
#[cfg(test)]
#[path = "send_intent_tests.rs"]
mod tests;

pub struct SendIntentAttempt {
    pub id: Id,
    pub result: Result<Id, Error>,
}
struct Intent {
    peer: Id,
    generation: u32,
    started: u64,
    text: Zeroizing<Vec<u8>>,
}
fn read(db: &Connection, key: &StorageKey, own: &Id, id: &Id) -> Result<(Intent, Vec<u8>), Error> {
    let sealed: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=?2 THEN state END FROM send_intents WHERE id=?1",
            (id.as_slice(), MAX_PLAINTEXT as i64 + 80),
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let raw = key.open(&sealed, &binding(36, id, own))?;
    if raw.len() < 44 {
        return Err(Error::InvalidStore);
    }
    let intent = Intent {
        peer: raw[..32].try_into().map_err(|_| Error::InvalidStore)?,
        generation: u32::from_be_bytes(raw[32..36].try_into().map_err(|_| Error::InvalidStore)?),
        started: u64::from_be_bytes(raw[36..44].try_into().map_err(|_| Error::InvalidStore)?),
        text: Zeroizing::new(raw[44..].to_vec()),
    };
    let text = Direct::from_bytes(&intent.text).map_err(|_| Error::InvalidStore)?;
    if text.message != *id
        || text.sender != *own
        || intent.started == 0
        || intent.started > i64::MAX as u64
    {
        return Err(Error::InvalidStore);
    }
    Ok((intent, sealed))
}
fn seal(key: &StorageKey, own: &Id, id: &Id, intent: &Intent) -> Result<Vec<u8>, Error> {
    let raw = Zeroizing::new(
        [
            intent.peer.as_slice(),
            &intent.generation.to_be_bytes(),
            &intent.started.to_be_bytes(),
            &intent.text,
        ]
        .concat(),
    );
    Ok(key.seal(&raw, &binding(36, id, own))?)
}
fn work_id(own: &Id, id: &Id, generation: u32, role: &[u8]) -> Id {
    Sha256::digest(
        [
            b"Sigil/send-intent/v0".as_slice(),
            own,
            id,
            &generation.to_be_bytes(),
            role,
        ]
        .concat(),
    )
    .into()
}
fn existing(
    tx: &Transaction<'_>,
    key: &StorageKey,
    peer: Id,
    id: Id,
    text: &[u8],
    now: u64,
) -> Result<Option<Id>, Error> {
    let row: Option<Vec<u8>> = tx
        .query_row(
            "SELECT session FROM deliveries WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = row else {
        return Ok(None);
    };
    let session: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
    if session_peer(tx, &session)? != Some(peer) {
        return Err(Error::Conflict);
    }
    load(tx, key, &session)?;
    let content: Vec<u8> = tx.query_row(
        "SELECT content FROM outbox WHERE session=?1 AND id=?2 AND length(content)<=65572",
        (session.as_slice(), id.as_slice()),
        |r| r.get(0),
    )?;
    if key.open(&content, &binding(9, &session, &id))?.as_slice()
        != crate::retained_payload(key, text)?.as_ref()
    {
        return Err(Error::Conflict);
    }
    if transport::receipt(tx, key, session, id)?.is_none() {
        let destination = peers::destination(tx, key, &peer)?;
        transport::prepare(tx, key, session, id, destination, None, now)?;
    }
    Ok(Some(session))
}
fn cursor(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
) -> Result<(Vec<u8>, Option<Vec<u8>>), Error> {
    let sealed:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state) IN (36,68) THEN state END FROM send_intent_cursor WHERE id=1",[],|r|r.get(0)).optional()?;
    let after = match &sealed {
        Some(s) => key.open(s, &binding(37, own, b"send intents"))?.to_vec(),
        None => Vec::new(),
    };
    if !after.is_empty() && after.len() != 32 {
        return Err(Error::InvalidStore);
    }
    Ok((after, sealed))
}
impl ClientStore {
    fn advance_send_claim(
        &mut self,
        fingerprint: &Id,
        id: Id,
        intent: &mut Intent,
        expected: &mut Vec<u8>,
        now: u64,
    ) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read(&tx, &self.key, fingerprint, &id)?.1 != *expected {
            return Err(Error::Conflict);
        }
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1)",
            [id.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict);
        }
        let claim = work_id(fingerprint, &id, intent.generation, b"claim");
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM prekey_claims WHERE id=?1)",
            [claim.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            let identity = handshake::identity(&tx, &self.key)?.public_key();
            claims::abandon(&tx, &self.key, &claim, &identity)?;
        }
        intent.generation = intent.generation.checked_add(1).ok_or(Error::Limit)?;
        intent.started = now;
        *expected = seal(&self.key, fingerprint, &id, intent)?;
        tx.execute(
            "UPDATE send_intents SET state=?1 WHERE id=?2",
            (expected.as_slice(), id.as_slice()),
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Freeze text and the independently verified peer before network work.
    /// The sync worker selects/establishes a session and queues exact ciphertext.
    /// Repeating an ID requires identical peer, body and original timestamp.
    pub fn queue_peer_text(
        &mut self,
        peer: Id,
        id: Id,
        body: &str,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        self.queue_peer_content(peer, id, Content::Text(body), timestamp, now)
    }
    pub(crate) fn queue_peer_content(
        &mut self,
        peer: Id,
        id: Id,
        body: Content<'_>,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        if matches!(body, Content::Conversation(_)) {
            crate::conversations::time_floor(&self.db, &self.key, now)?;
        }
        let own = self.own_device_binding()?;
        let fingerprint = device_fingerprint(&own)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let text = context(&tx, &self.key, &own, &peer)?.encode(id, body, timestamp)?;
        if crate::conversations::cancelled(&tx, &self.key, &[0; 32], &id)? {
            return Err(Error::Obsolete);
        }
        crate::conversations::check_send(&tx, &self.key, &text, now)?;
        if matches!(body, Content::File(_) | Content::Rich(_)) {
            super::require_resend(
                &tx,
                &self.key,
                &own,
                &peer,
                &Direct::from_bytes(&text).map_err(|_| Error::InvalidEvent)?,
                true,
            )?;
        }
        match read(&tx, &self.key, &fingerprint, &id) {
            Ok((prior, _)) => {
                return if prior.peer == peer && prior.text == text {
                    Ok(())
                } else {
                    Err(Error::Conflict)
                }
            }
            Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
        if matches!(body, Content::Conversation(_)) {
            super::retain(&tx, &self.key, &own, &peer, &text, true)?;
        }
        if existing(&tx, &self.key, peer, id, &text, now)?.is_none() {
            if tx.query_row("SELECT count(*) FROM send_intents", [], |r| {
                r.get::<_, i64>(0)
            })? >= 256
            {
                return Err(Error::Limit);
            }
            let intent = Intent {
                peer,
                generation: 0,
                started: now,
                text,
            };
            tx.execute(
                "INSERT INTO send_intents VALUES(?1,?2)",
                (id.as_slice(), seal(&self.key, &fingerprint, &id, &intent)?),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    fn prepare_send_intent_online(&mut self, id: Id, now: u64) -> Result<Id, Error> {
        let own = self.own_device_binding()?;
        let fingerprint = device_fingerprint(&own)?;
        let (mut intent, mut expected) = read(&self.db, &self.key, &fingerprint, &id)?;
        match crate::conversations::check_send(&self.db, &self.key, &intent.text, now) {
            Ok(()) => {}
            Err(Error::Obsolete) => {
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                if tx.execute(
                    "DELETE FROM send_intents WHERE id=?1 AND state=?2",
                    (id.as_slice(), expected),
                )? == 1
                {
                    crate::conversations::mark_cancelled(&tx, &self.key, &[0; 32], &id)?;
                }
                tx.commit()?;
                return Err(Error::Obsolete);
            }
            Err(e) => return Err(e),
        }
        if now < intent.started {
            return Err(Error::Expired);
        }
        let text = Direct::from_bytes(&intent.text).map_err(|_| Error::InvalidStore)?;
        if context(&self.db, &self.key, &own, &intent.peer)?.encode(
            id,
            text.content,
            text.timestamp,
        )? != intent.text
        {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if matches!(text.content, Content::Rich(_)) {
            super::require_resend(&tx, &self.key, &own, &intent.peer, &text, true)?;
        }
        let queued = existing(&tx, &self.key, intent.peer, id, &intent.text, now)?;
        tx.commit()?;
        if let Some(session) = queued {
            self.db.execute(
                "DELETE FROM send_intents WHERE id=?1 AND state=?2",
                (id.as_slice(), expected),
            )?;
            return Ok(session);
        }
        let session =
            match self.send_peer_content(intent.peer, id, text.content, text.timestamp, now) {
                Ok((session, _)) => session,
                Err(error) => {
                    // Never reroute an already-frozen delivery, even when expired.
                    if self.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1)",
                        [id.as_slice()],
                        |r| r.get::<_, bool>(0),
                    )? {
                        return Err(error);
                    }
                    match error {
                        Error::Unprepared if self.active_session(intent.peer)?.is_none() => {}
                        Error::Expired => {}
                        other => return Err(other),
                    }
                    if now - intent.started >= 604800 {
                        self.advance_send_claim(&fingerprint, id, &mut intent, &mut expected, now)?;
                    }
                    let claim = work_id(&fingerprint, &id, intent.generation, b"claim");
                    let session = work_id(&fingerprint, &id, intent.generation, b"session");
                    self.prepare_peer_claim(claim, intent.peer)?;
                    if let Err(error) = self.claim_prekey_online(claim, now) {
                        if matches!(error, Error::Expired) {
                            self.advance_send_claim(
                                &fingerprint,
                                id,
                                &mut intent,
                                &mut expected,
                                now,
                            )?;
                        }
                        return Err(error);
                    }
                    // Bind the intended fingerprint again after claim I/O.
                    let text = Direct::from_bytes(&intent.text).map_err(|_| Error::InvalidStore)?;
                    let tx = self
                        .db
                        .transaction_with_behavior(TransactionBehavior::Immediate)?;
                    if read(&tx, &self.key, &fingerprint, &id)?.1 != expected
                        || context(&tx, &self.key, &own, &intent.peer)?.encode(
                            id,
                            text.content,
                            text.timestamp,
                        )? != intent.text
                    {
                        return Err(Error::Conflict);
                    }
                    claims::start_in(
                        &tx,
                        &self.key,
                        claim,
                        session,
                        id,
                        (&intent.text, Some(&own), None),
                        now,
                    )?;
                    tx.commit()?;
                    session
                }
            };
        self.db.execute(
            "DELETE FROM send_intents WHERE id=?1 AND state=?2",
            (id.as_slice(), expected),
        )?;
        Ok(session)
    }
    /// At most 16 frozen intents per pass. Network errors stop for caller backoff;
    /// missing stock (HTTP 404) remains queued. Trust/local failures stay per-item.
    pub fn resume_send_intents_online(
        &mut self,
        now: u64,
    ) -> Result<Vec<SendIntentAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        let ids: Vec<Vec<u8>> = self
            .db
            .prepare("SELECT id FROM send_intents WHERE id>?1 ORDER BY id LIMIT 16")?
            .query_map([after], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut results = Vec::new();
        let mut next = Vec::new();
        for raw in ids {
            let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
            let result = self.prepare_send_intent_online(id, now);
            let stop = matches!(result, Err(Error::Network(_)));
            results.push(SendIntentAttempt { id, result });
            next = id.to_vec();
            if stop {
                break;
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cursor(&tx, &self.key, &own)?.1 == expected {
            tx.execute("INSERT INTO send_intent_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",[self.key.seal(&next,&binding(37,&own,b"send intents"))?])?;
        }
        tx.commit()?;
        Ok(results)
    }
}

//! Verified, bounded inbound routing and crash-safe mailbox acknowledgement.
use super::*;
use crate::connection::decode_id;
use sigil_protocol::mailbox::Delivery;

pub(super) const MIGRATION: &str = "
CREATE INDEX sessions_peer ON sessions(peer);
CREATE TABLE incoming(sequence INTEGER PRIMARY KEY CHECK(sequence>0), acknowledged INTEGER NOT NULL CHECK(acknowledged IN (0,1)), state BLOB NOT NULL);
PRAGMA user_version=15;";
#[cfg(test)]
#[path = "message_lifetime_tests.rs"]
mod lifetime_tests;
#[cfg(test)]
#[path = "incoming_tests.rs"]
pub(crate) mod tests;

pub struct Incoming {
    pub peer: Id,
    pub session: Id,
    pub message: Id,
    pub plaintext: Zeroizing<Vec<u8>>,
    /// Already accepted logical content; transport bookkeeping may still be new.
    pub duplicate: bool,
}
enum DeliveryResult {
    Accepted(Incoming),
    Group(Id),
}
pub struct IncomingAttempt {
    pub sequence: i64,
    pub result: Result<MailboxEvent, Error>,
    pub recovery: crate::RecoveryAdvice,
    /// Packet type tag (first four payload bytes as hex) for diagnostics.
    pub kind: String,
}
/// One acknowledgement the pass could not complete; the slot waits for its condition to change.
#[derive(Debug)]
pub struct AcknowledgeAttempt {
    pub sequence: i64,
    pub result: Result<(), Error>,
}
/// Per-item verdicts stay here; only transport and store faults fail the stage.
#[derive(Debug, Default)]
pub struct Acknowledgements {
    pub acknowledged: usize,
    pub rejected: Vec<AcknowledgeAttempt>,
}
pub enum MailboxEvent {
    Text(Incoming),
    File(Incoming),
    SigilText(Incoming),
    Conversation,
    Call(Id),
    GroupDistribution(groups::DistributionReceipt),
    GroupInvitation(Id),
    GroupHistory(Id),
    GroupText(groups::GroupMessage),
    GroupFile(groups::GroupMessage),
    GroupSigilText(groups::GroupMessage),
    Retry(RetryRequest),
    /// Authenticated control durably resolved without scheduling another response.
    DiscardedRetry(Id),
    RecoveredDelivery(Id),
}
struct Record {
    peer: Id,
    session: Id,
    message: Id,
    digest: Id,
    expires: u64,
    acknowledged: bool,
}
impl Record {
    fn seal(&self, key: &StorageKey, own: &Id, sequence: i64) -> Result<Vec<u8>, Error> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(137));
        bytes.extend_from_slice(&self.peer);
        bytes.extend_from_slice(&self.session);
        bytes.extend_from_slice(&self.message);
        bytes.extend_from_slice(&self.digest);
        bytes.extend_from_slice(&self.expires.to_be_bytes());
        bytes.push(u8::from(self.acknowledged));
        Ok(key.seal(&bytes, &binding(16, own, &sequence.to_be_bytes()))?)
    }
    fn message(&self, db: &Connection, key: &StorageKey) -> Result<Incoming, Error> {
        let content: Vec<u8> = db.query_row(
            "SELECT content FROM inbox WHERE session=?1 AND id=?2 AND length(content)<=65572",
            (self.session.as_slice(), self.message.as_slice()),
            |r| r.get(0),
        )?;
        let plaintext = key.open(&content, &binding(2, &self.session, &self.message))?;
        crate::erasure::require_retained(&plaintext)?;
        Ok(Incoming {
            peer: self.peer,
            session: self.session,
            message: self.message,
            plaintext,
            duplicate: true,
        })
    }
}
fn record(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    sequence: i64,
) -> Result<Option<Record>, Error> {
    let row: Option<(bool, Vec<u8>)> = db.query_row(
        "SELECT acknowledged,CASE WHEN length(state)=173 THEN state END FROM incoming WHERE sequence=?1", [sequence],
        |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((acknowledged, sealed)) = row else {
        return Ok(None);
    };
    if sealed.len() != 173 {
        return Err(Error::InvalidStore);
    }
    let bytes = key.open(&sealed, &binding(16, own, &sequence.to_be_bytes()))?;
    if bytes.len() != 137 || bytes[136] != u8::from(acknowledged) {
        return Err(Error::InvalidStore);
    }
    Ok(Some(Record {
        peer: bytes[..32].try_into().map_err(|_| Error::InvalidStore)?,
        session: bytes[32..64].try_into().map_err(|_| Error::InvalidStore)?,
        message: bytes[64..96].try_into().map_err(|_| Error::InvalidStore)?,
        digest: bytes[96..128].try_into().map_err(|_| Error::InvalidStore)?,
        expires: u64::from_be_bytes(
            bytes[128..136]
                .try_into()
                .map_err(|_| Error::InvalidStore)?,
        ),
        acknowledged,
    }))
}

/// (session, message) of every unacknowledged delivery; None when any cannot be decoded.
pub(crate) fn unacknowledged(
    db: &Connection,
    key: &StorageKey,
    own: Option<&Id>,
) -> Result<Option<std::collections::BTreeSet<(Id, Id)>>, Error> {
    let sequences: Vec<i64> = db
        .prepare("SELECT sequence FROM incoming WHERE acknowledged=0")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut pending = std::collections::BTreeSet::new();
    for sequence in sequences {
        let Some(own) = own else { return Ok(None) };
        match record(db, key, own, sequence) {
            Ok(Some(record)) => {
                pending.insert((record.session, record.message));
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(pending))
}

impl ClientStore {
    pub(super) fn retained_incoming_event(&mut self, sequence: i64) -> Result<Incoming, Error> {
        let own_statement = self.own_device_binding()?;
        let own = decode_id(
            &self
                .connection_session()?
                .ok_or(Error::Unprepared)?
                .device_id,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = record(&tx, &self.key, &own, sequence)?.ok_or(Error::NotFound)?;
        let incoming = record.message(&tx, &self.key)?;
        event::validate(
            &tx,
            &self.key,
            &own_statement,
            &record.peer,
            &record.message,
            &incoming.plaintext,
            record.expires,
        )?;
        tx.commit()?;
        Ok(incoming)
    }
    /// Direct messages require a verified device. An unverified device may only
    /// install an initial distribution authorized by signed group membership.
    /// At most eight bound direct sessions are tried. Content, key changes and
    /// the acknowledgement journal commit atomically.
    pub fn accept_delivery(&mut self, delivery: &Delivery) -> Result<Incoming, Error> {
        self.accept_delivery_at(delivery, crate::conversations::now())
    }
    pub fn accept_delivery_at(&mut self, delivery: &Delivery, now: u64) -> Result<Incoming, Error> {
        match self.accept_delivery_inner(delivery, false, now)? {
            DeliveryResult::Accepted(incoming) => Ok(incoming),
            DeliveryResult::Group(_) => Err(Error::InvalidStore),
        }
    }
    /// Authenticate a distribution in a rolled-back transaction, refresh its
    /// pinned authority, then reauthenticate and commit against the new state.
    pub fn accept_delivery_online(
        &mut self,
        delivery: &Delivery,
        now: u64,
    ) -> Result<Incoming, Error> {
        match self.accept_delivery_inner(delivery, true, now)? {
            DeliveryResult::Accepted(incoming) => Ok(incoming),
            DeliveryResult::Group(group) => {
                self.refresh_group_authority_for_send(group, now)?;
                self.accept_delivery_at(delivery, now)
            }
        }
    }
    fn accept_delivery_inner(
        &mut self,
        delivery: &Delivery,
        preflight: bool,
        now: u64,
    ) -> Result<DeliveryResult, Error> {
        if delivery.sequence <= 0
            || delivery.expires_at == 0
            || delivery.expires_at > i64::MAX as u64
            || !network::valid_hex(
                &delivery.payload,
                32,
                sigil_protocol::mailbox::MAX_PAYLOAD_HEX,
            )
        {
            return Err(Error::Conflict);
        }
        let sender = decode_id(&delivery.sender_device)?;
        let message = decode_id(&delivery.message_id)?;
        let connection = self.connection_session()?.ok_or(Error::Unprepared)?;
        let own = decode_id(&connection.device_id)?;
        let server = connection
            .address
            .split_once(':')
            .ok_or(Error::InvalidStore)?
            .1;
        let peer = crate::federation::delivery_peer(&self.db, &self.key, server, delivery)?;
        let own_statement = self.own_device_binding()?;
        let packet: Vec<u8> = delivery
            .payload
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| Error::Conflict)?, 16)
                    .map_err(|_| Error::Conflict)
            })
            .collect::<Result<_, _>>()?;
        let digest: Id = Sha256::digest(&packet).into();
        // This lookup is an authenticated routing hint, not handshake acceptance.
        let initial = sigil_protocol::initial::decode(&packet).is_ok();
        let slot = if initial {
            Some(self.initial_prekey_slot(&packet)?)
        } else {
            None
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM retry_incoming WHERE sequence=?1) OR EXISTS(SELECT 1 FROM recovered_deliveries WHERE sequence=?1) OR EXISTS(SELECT 1 FROM group_incoming WHERE sequence=?1)",
            [delivery.sequence],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict);
        }
        let known = peers::known(&tx, &self.key, &peer)?;
        if known.binding.device != sender {
            return Err(Error::Conflict);
        }
        if known.blocked
            || known.changed_fingerprint.is_some()
            || known.replaced_by.is_some()
            || (!known.trusted && !initial)
        {
            return Err(Error::Unprepared);
        }
        if let Some(prior) = record(&tx, &self.key, &own, delivery.sequence)? {
            if prior.peer != peer
                || prior.message != message
                || prior.digest != digest
                || prior.expires != delivery.expires_at
            {
                return Err(Error::Conflict);
            }
            let incoming = prior.message(&tx, &self.key)?;
            if !known.trusted
                && incoming.distribution()?.is_none()
                && calls::receipt_message(&incoming.plaintext)?.is_none()
            {
                return Err(Error::Unprepared);
            }
            event::validate(
                &tx,
                &self.key,
                &own_statement,
                &peer,
                &message,
                &incoming.plaintext,
                prior.expires,
            )?;
            event::remember(
                &tx,
                &self.key,
                &own_statement,
                &peer,
                &incoming.session,
                &incoming.plaintext,
            )?;
            tx.commit()?;
            return Ok(DeliveryResult::Accepted(incoming));
        }
        let (session, mut plaintext, fresh) = if let Some(slot) = slot {
            let session: Id = Sha256::digest(
                [
                    b"Sigil/incoming-session/v0".as_slice(),
                    &own,
                    &peer,
                    &Sha256::digest(
                        sigil_protocol::initial::decode(&packet)
                            .map_err(|_| Error::UnsupportedSession)?
                            .0,
                    ),
                ]
                .concat(),
            )
            .into();
            let expected = known.binding.identity;
            let fresh = !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM inbox WHERE session=?1 AND id=?2)",
                (session.as_slice(), message.as_slice()),
                |r| r.get::<_, bool>(0),
            )?;
            let plaintext =
                handshake::accept(&tx, &self.key, slot, session, message, expected, &packet)?;
            if known.trusted
                && !groups::is_distribution_wire(&plaintext)
                && groups::distribution_receipt(&plaintext)?.is_none()
                && !calls::scoped_wire(&plaintext)?
                && (calls::receipt_message(&plaintext)?.is_none()
                    || session_peer(&tx, &session)?.is_some())
            {
                peers::bind_session(&tx, &self.key, &session, &peer)?;
            }
            (session, plaintext, fresh)
        } else {
            let parsed = Packet::from_bytes(&packet)?;
            let sessions: Vec<Vec<u8>> = tx
                .prepare(
                    "SELECT id FROM sessions WHERE peer=?1 AND retired=0 ORDER BY id LIMIT ?2",
                )?
                .query_map((peer.as_slice(), peers::MAX_SESSIONS as i64 + 1), |r| {
                    r.get(0)
                })?
                .collect::<Result<_, _>>()?;
            if sessions.len() > peers::MAX_SESSIONS {
                return Err(Error::Limit);
            }
            let mut selected = None;
            let tried = sessions.len() as u8;
            let (mut replay, mut limit, mut other) = (0, 0, 0);
            for session in sessions {
                let session: Id = session.try_into().map_err(|_| Error::InvalidStore)?;
                let mut state = load(&tx, &self.key, &session)?;
                // Authenticate stored checkpoints separately: corrupt local state
                // must not be disguised as a failed candidate packet.
                match state.1.receive(&parsed) {
                    Ok(plaintext) => {
                        if selected
                            .replace((session, state, Zeroizing::new(plaintext)))
                            .is_some()
                        {
                            return Err(Error::Conflict);
                        }
                    }
                    Err(sigil_crypto::Error::Entropy) => {
                        return Err(Error::Crypto(sigil_crypto::Error::Entropy))
                    }
                    Err(sigil_crypto::Error::Replay) => replay += 1,
                    Err(sigil_crypto::Error::Limit) => limit += 1,
                    Err(sigil_crypto::Error::Authentication) => {}
                    Err(_) => other += 1,
                }
            }
            let (session, state, plaintext) = selected.ok_or(Error::ReceiveAuthentication {
                sessions: tried,
                replay,
                limit,
                other,
            })?;
            commit_received(
                &tx, &self.key, session, message, &packet, &state, &plaintext,
            )?;
            (session, plaintext, true)
        };
        // An unverified identity can authenticate a candidate initial packet,
        // but only a distribution authorized by signed group membership may
        // commit. Ordinary messages and channel selection remain direct-only.
        if !known.trusted
            && !groups::is_distribution_wire(&plaintext)
            && groups::distribution_receipt(&plaintext)?.is_none()
            && !calls::scoped_wire(&plaintext)?
            && calls::receipt_message(&plaintext)?.is_none()
        {
            return Err(Error::Unprepared);
        }
        if preflight && groups::is_distribution_wire(&plaintext) {
            let group = groups::distribution_group(&plaintext)?;
            return Ok(DeliveryResult::Group(group));
        }
        if let Some(marker) = groups::install_distribution(
            &tx,
            &self.key,
            &own_statement,
            &peer,
            &message,
            &plaintext,
            delivery.expires_at,
        )? {
            plaintext = Zeroizing::new(marker);
        }
        if let Some(marker) = groups::install_invitation(
            &tx,
            &self.key,
            &own_statement,
            &peer,
            &message,
            &plaintext,
            delivery.expires_at,
        )? {
            plaintext = Zeroizing::new(marker);
        }
        if let Some(marker) = crate::calls::install(
            &tx,
            &self.key,
            &own_statement,
            &peer,
            &message,
            &plaintext,
            now,
        )? {
            plaintext = Zeroizing::new(marker);
        }
        event::validate(
            &tx,
            &self.key,
            &own_statement,
            &peer,
            &message,
            &plaintext,
            delivery.expires_at,
        )?;
        let duplicate =
            event::remember(&tx, &self.key, &own_statement, &peer, &session, &plaintext)?;
        event::retain(&tx, &self.key, &own_statement, &peer, &plaintext, false)?;
        let group_initial = initial
            && session_peer(&tx, &session)?.is_none()
            && groups::distribution_receipt(&plaintext)?.is_some();
        if group_initial {
            let receipt = groups::distribution_receipt(&plaintext)?.ok_or(Error::InvalidStore)?;
            groups::mark_channel(
                &tx,
                &self.key,
                &device_fingerprint(&own_statement)?,
                &session,
                &receipt.context.group,
                &known.fingerprint,
            )?;
        }
        let call_initial = initial
            && session_peer(&tx, &session)?.is_none()
            && calls::receipt_message(&plaintext)?.is_some();
        if fresh && known.trusted && !group_initial && !call_initial {
            if initial {
                selection::activate(&tx, &self.key, &peer, &session)?;
            } else {
                selection::converge(&tx, &self.key, &peer, &session)?;
            }
        }
        let record = Record {
            peer,
            session,
            message,
            digest,
            expires: delivery.expires_at,
            acknowledged: false,
        };
        tx.execute(
            "INSERT INTO incoming VALUES(?1,0,?2)",
            (
                delivery.sequence,
                record.seal(&self.key, &own, delivery.sequence)?,
            ),
        )?;
        tx.commit()?;
        Ok(DeliveryResult::Accepted(Incoming {
            peer,
            session,
            message,
            plaintext,
            duplicate,
        }))
    }

    /// Connection and mailbox cursor for a platform-owned wait request.
    pub fn mailbox_watch(&self) -> Result<(network::HttpsClient, i64), Error> {
        let network = self.connected_client()?;
        let own = decode_id(
            &self
                .connection_session()?
                .ok_or(Error::Unprepared)?
                .device_id,
        )?;
        Ok((network, cursor(&self.db, &self.key, &own)?.0))
    }
    pub fn mailbox_watch_target(&self) -> Result<(String, Zeroizing<String>, i64), Error> {
        let (network, after) = self.mailbox_watch()?;
        Ok((network.api_origin()?, network.credential(), after))
    }
    /// One bounded fetch. A durable scan cursor moves past individual failures
    /// without acknowledging them and wraps after reaching the end. No peer is
    /// trusted or key replaced by this operation. Acknowledge separately.
    pub fn receive_mailbox_online(&mut self, now: u64) -> Result<Vec<IncomingAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let network = self.connected_client()?;
        let connection = self.connection_session()?.ok_or(Error::Unprepared)?;
        let own = decode_id(&connection.device_id)?;
        let server = connection
            .address
            .split_once(':')
            .ok_or(Error::InvalidStore)?
            .1
            .to_owned();
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        let deliveries = network.mailbox_after(after)?;
        let next = deliveries.last().map_or(0, |delivery| delivery.sequence);
        let attempts = deliveries
            .iter()
            .map(|delivery| {
                let result = if delivery
                    .payload
                    .get(..8)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("53475252"))
                {
                    self.route_retry(delivery, now, true)
                } else if groups::is_envelope(&delivery.payload)
                    || delivery
                        .payload
                        .get(..8)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("53474b4d"))
                {
                    self.accept_group_delivery_online(delivery, now)
                        .and_then(|message| {
                            match crate::conversations::require_payload(
                                &self.db,
                                &self.key,
                                &message.plaintext,
                                now,
                                false,
                            ) {
                                Ok(()) => {}
                                Err(Error::Obsolete) => return Ok(MailboxEvent::Conversation),
                                Err(error) => return Err(error),
                            }
                            match message.event()?.content {
                                sigil_protocol::event::Content::Conversation(_) => {
                                    Ok(MailboxEvent::Conversation)
                                }
                                sigil_protocol::event::Content::File(_) => {
                                    Ok(MailboxEvent::GroupFile(message))
                                }
                                sigil_protocol::event::Content::Rich(_) => {
                                    Ok(MailboxEvent::GroupSigilText(message))
                                }
                                sigil_protocol::event::Content::Text(_) => {
                                    Ok(MailboxEvent::GroupText(message))
                                }
                            }
                        })
                } else {
                    match self.accept_delivery_online(delivery, now) {
                        Ok(message) => {
                            match crate::conversations::require_payload(
                                &self.db,
                                &self.key,
                                &message.plaintext,
                                now,
                                false,
                            ) {
                                Ok(()) => ordinary_event(message),
                                Err(Error::Obsolete) => Ok(MailboxEvent::Conversation),
                                Err(error) => Err(error),
                            }
                        }
                        Err(Error::Obsolete) => Ok(MailboxEvent::Conversation),
                        // A replaced device never regains acceptance; release its slot.
                        Err(Error::Unprepared) if self.sender_replaced(&server, delivery) => {
                            Err(Error::Obsolete)
                        }
                        Err(error) => match self.resolve_failed_delivery(delivery) {
                            Ok(true) => {
                                decode_id(&delivery.message_id).map(MailboxEvent::RecoveredDelivery)
                            }
                            Ok(false) => Err(error),
                            Err(proof_error) => Err(proof_error),
                        },
                    }
                };
                let recovery = self.recovery_advice(delivery, &result, now);
                if let Err(error) = &result {
                    let _ = self.abandon_unreadable(delivery.sequence, error);
                }
                IncomingAttempt {
                    sequence: delivery.sequence,
                    kind: delivery.payload.get(..8).unwrap_or_default().to_owned(),
                    result,
                    recovery,
                }
            })
            .collect();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cursor(&tx, &self.key, &own)?.1 == expected {
            tx.execute("INSERT INTO incoming_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
                [self.key.seal(&next.to_be_bytes(), &binding(17, &own, b"mailbox scan"))?])?;
        }
        tx.commit()?;
        Ok(attempts)
    }

    /// Replacement is approved from the peer's device inventory and never cleared.
    fn sender_replaced(&self, server: &str, delivery: &Delivery) -> bool {
        crate::federation::delivery_peer(&self.db, &self.key, server, delivery)
            .and_then(|peer| peers::known(&self.db, &self.key, &peer))
            .is_ok_and(|known| known.replaced_by.is_some())
    }
    /// Retry at most 16 durable acknowledgements. Network success followed by a
    /// local failure safely retries the same sequence after restart.
    /// A delivery keeps its slot only while another attempt could still succeed: a local
    /// storage fault, a network fault, or a peer this device has yet to accept. Anything
    /// else has failed for good, and leaving it on the server costs the sender one of its
    /// few queue slots until it expires, which jams the pair. Record it so the next pass
    /// releases the slot; where recovery applies, the sender is asked for a fresh copy.
    fn abandon_unreadable(&mut self, sequence: i64, error: &Error) -> Result<(), Error> {
        let retryable = matches!(
            error,
            Error::Storage(_)
                | Error::Io(_)
                | Error::InvalidStore
                | Error::Network(_)
                | Error::Unprepared
                | Error::Crypto(sigil_crypto::Error::Entropy | sigil_crypto::Error::State)
        );
        if retryable {
            return Ok(());
        }
        // The sequence comes from the server; one that cannot be acknowledged must
        // never be recorded, or it fails every later acknowledgement in the pass.
        if sequence <= 0 {
            return Ok(());
        }
        self.db.execute(
            "INSERT OR IGNORE INTO abandoned_deliveries(sequence) VALUES(?1)",
            [sequence],
        )?;
        Ok(())
    }
    fn acknowledge_abandoned_online(&mut self, sequence: i64) -> Result<(), Error> {
        match self.connected_client()?.acknowledge_delivery(sequence) {
            Ok(()) => {}
            // The server no longer holds it, or it was never addressable at all. The
            // slot is already free, and keeping the record would block every later
            // acknowledgement, which in turn stops this device sending anything.
            Err(crate::network::Error::Status { code: 404, .. })
            | Err(crate::network::Error::Configuration) => {}
            Err(error) => return Err(Error::Network(error)),
        }
        self.db.execute(
            "DELETE FROM abandoned_deliveries WHERE sequence=?1",
            [sequence],
        )?;
        Ok(())
    }
    /// The first rejected item is the error, as before; the report form keeps them apart.
    pub fn acknowledge_incoming_online(&mut self) -> Result<usize, Error> {
        let report = self.acknowledge_incoming_report_online()?;
        match report.rejected.into_iter().next() {
            Some(attempt) => Err(attempt.result.err().ok_or(Error::InvalidStore)?),
            None => Ok(report.acknowledged),
        }
    }
    /// Err only for faults a backoff can help; per-item verdicts are reported, not raised.
    pub fn acknowledge_incoming_report_online(&mut self) -> Result<Acknowledgements, Error> {
        let network = self.connected_client()?;
        let own_statement = self.own_device_binding()?;
        let own = decode_id(
            &self
                .connection_session()?
                .ok_or(Error::Unprepared)?
                .device_id,
        )?;
        let sequences: Vec<(i64, u8)> = self
            .db
            .prepare(
                "SELECT sequence,0 FROM incoming WHERE acknowledged=0 UNION ALL SELECT sequence,1 FROM retry_incoming WHERE acknowledged=0 UNION ALL SELECT sequence,2 FROM recovered_deliveries WHERE acknowledged=0 UNION ALL SELECT sequence,3 FROM group_incoming WHERE acknowledged=0 UNION ALL SELECT sequence,4 FROM abandoned_deliveries ORDER BY sequence LIMIT 16",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut count = 0;
        // Ordinary deliveries validate in order; controls are independent of them.
        let mut ready = Vec::new();
        let mut halted = None;
        let mut failures = Vec::new();
        for (sequence, control) in sequences {
            // One unusable ordinary delivery stops the ordered chain, not the controls.
            if control == 0 && halted.is_some() {
                continue;
            }
            let result = match control {
                4 => self.acknowledge_abandoned_online(sequence),
                3 => groups::acknowledge_group(self, sequence),
                2 => self.acknowledge_recovered_online(sequence),
                1 => self.acknowledge_retry_online(sequence),
                _ => match self.commit_incoming(&own_statement, &own, sequence) {
                    Ok(()) => Ok(ready.push(sequence)),
                    // Accepted and sealed already; a later owner action or deadline retired it.
                    Err(error) if terminal(&error) => self.abandon_committed(&own, sequence),
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = result {
                if control == 0 {
                    halted = Some((sequence, error));
                } else {
                    failures.push((sequence, error));
                }
                continue;
            }
            if control != 0 {
                count += 1;
            }
        }
        for (sequence, result) in ready.iter().zip(network.acknowledge_deliveries(&ready)) {
            match result {
                Ok(()) => {
                    let tx = self
                        .db
                        .transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let mut current =
                        record(&tx, &self.key, &own, *sequence)?.ok_or(Error::InvalidStore)?;
                    current.acknowledged = true;
                    tx.execute(
                        "UPDATE incoming SET acknowledged=1,state=?1 WHERE sequence=?2",
                        (current.seal(&self.key, &own, *sequence)?, sequence),
                    )?;
                    tx.commit()?;
                    count += 1;
                }
                Err(error) => failures.push((*sequence, Error::from(error))),
            }
        }
        let mut rejected: Vec<AcknowledgeAttempt> = halted
            .into_iter()
            .chain(failures)
            .map(|(sequence, error)| AcknowledgeAttempt { sequence, result: Err(error) })
            .collect();
        // Transport and store faults fail the stage so the schedule backs off; verdicts do not.
        if let Some(index) = rejected
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_) | Error::Storage(_) | Error::Io(_))))
        {
            return Err(rejected.swap_remove(index).result.err().ok_or(Error::InvalidStore)?);
        }
        Ok(Acknowledgements { acknowledged: count, rejected })
    }
    /// The sealed record proves acceptance; release the slot without re-validating.
    fn abandon_committed(&mut self, own: &Id, sequence: i64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = record(&tx, &self.key, own, sequence)?.ok_or(Error::InvalidStore)?;
        current.acknowledged = true;
        tx.execute(
            "UPDATE incoming SET acknowledged=1,state=?1 WHERE sequence=?2",
            (current.seal(&self.key, own, sequence)?, sequence),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO abandoned_deliveries(sequence) VALUES(?1)",
            [sequence],
        )?;
        Ok(tx.commit()?)
    }
    /// Validates and remembers a delivery before its acknowledgement leaves.
    fn commit_incoming(&mut self, own_statement: &[u8], own: &Id, sequence: i64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior = record(&tx, &self.key, own, sequence)?.ok_or(Error::InvalidStore)?;
        let committed = prior.message(&tx, &self.key)?;
        event::validate(
            &tx,
            &self.key,
            own_statement,
            &prior.peer,
            &prior.message,
            &committed.plaintext,
            prior.expires,
        )?;
        event::remember(
            &tx,
            &self.key,
            own_statement,
            &prior.peer,
            &prior.session,
            &committed.plaintext,
        )?;
        Ok(tx.commit()?)
    }
}
/// Failures no retry can clear; local, network and crypto faults stay retryable.
fn terminal(error: &Error) -> bool {
    matches!(
        error,
        Error::Obsolete | Error::Expired | Error::InvalidEvent | Error::Conflict
    )
}
fn cursor(db: &Connection, key: &StorageKey, own: &Id) -> Result<(i64, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=44 THEN state END FROM incoming_cursor WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let value = match &sealed {
        Some(sealed) => i64::from_be_bytes(
            key.open(sealed, &binding(17, own, b"mailbox scan"))?
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidStore)?,
        ),
        None => 0,
    };
    if value < 0 {
        return Err(Error::InvalidStore);
    }
    Ok((value, sealed))
}

fn ordinary_event(message: Incoming) -> Result<MailboxEvent, Error> {
    if let Some(id) = crate::calls::receipt_call(&message.plaintext)? {
        return Ok(MailboxEvent::Call(id));
    }
    if let Some(id) = groups::invitation_reference(&message.plaintext)? {
        return Ok(MailboxEvent::GroupInvitation(id));
    }
    if let Some(receipt) = message.distribution()? {
        if let Some(id) = receipt.history_share() {
            return Ok(MailboxEvent::GroupHistory(id));
        }
        return Ok(MailboxEvent::GroupDistribution(receipt));
    }
    Ok(match message.event()?.content {
        sigil_protocol::event::Content::Conversation(_) => MailboxEvent::Conversation,
        sigil_protocol::event::Content::File(_) => MailboxEvent::File(message),
        sigil_protocol::event::Content::Rich(_) => MailboxEvent::SigilText(message),
        sigil_protocol::event::Content::Text(_) => MailboxEvent::Text(message),
    })
}

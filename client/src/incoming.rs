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
pub struct IncomingAttempt {
    pub sequence: i64,
    pub result: Result<MailboxEvent, Error>,
    pub recovery: crate::RecoveryAdvice,
}
pub enum MailboxEvent {
    Text(Incoming),
    File(Incoming),
    SigilText(Incoming),
    GroupDistribution(groups::DistributionReceipt),
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
        Ok(Incoming {
            peer: self.peer,
            session: self.session,
            message: self.message,
            plaintext: key.open(&content, &binding(2, &self.session, &self.message))?,
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
    /// Route only through an explicitly verified device. At most eight bound
    /// sessions are tried; failed candidate decryptions never change live state.
    /// The returned content and acknowledgement journal commit atomically.
    pub fn accept_delivery(&mut self, delivery: &Delivery) -> Result<Incoming, Error> {
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
        let peer = peers::reference(server, &sender);
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
        if peers::destination(&tx, &self.key, &peer)? != sender {
            return Err(Error::Conflict);
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
            return Ok(incoming);
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
            let expected = peers::identity(&tx, &self.key, &peer)?;
            let fresh = !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM inbox WHERE session=?1 AND id=?2)",
                (session.as_slice(), message.as_slice()),
                |r| r.get::<_, bool>(0),
            )?;
            let plaintext =
                handshake::accept(&tx, &self.key, slot, session, message, expected, &packet)?;
            peers::bind_session(&tx, &self.key, &session, &peer)?;
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
                    Err(_) => {}
                }
            }
            let (session, state, plaintext) =
                selected.ok_or(Error::Crypto(sigil_crypto::Error::Authentication))?;
            commit_received(
                &tx, &self.key, session, message, &packet, &state, &plaintext,
            )?;
            (session, plaintext, true)
        };
        if let Some(marker) = groups::install_distribution(
            &tx,
            &self.key,
            &own_statement,
            &peer,
            &message,
            &plaintext,
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
        if fresh {
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
        Ok(Incoming {
            peer,
            session,
            message,
            plaintext,
            duplicate,
        })
    }

    /// One bounded fetch. A durable scan cursor moves past individual failures
    /// without acknowledging them and wraps after reaching the end. No peer is
    /// trusted or key replaced by this operation. Acknowledge separately.
    pub fn receive_mailbox_online(&mut self, now: u64) -> Result<Vec<IncomingAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let network = self.connected_client()?;
        let own = decode_id(
            &self
                .connection_session()?
                .ok_or(Error::Unprepared)?
                .device_id,
        )?;
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
                } else if delivery
                    .payload
                    .get(..8)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("53474b4d"))
                {
                    self.accept_group_delivery(delivery).and_then(|message| {
                        match message.event()?.content {
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
                    match self.accept_delivery(delivery) {
                        Ok(message) => match message.distribution() {
                            Ok(Some(receipt)) => Ok(MailboxEvent::GroupDistribution(receipt)),
                            Ok(None) => match message.event().map(|event| event.content) {
                                Ok(sigil_protocol::event::Content::File(_)) => {
                                    Ok(MailboxEvent::File(message))
                                }
                                Ok(sigil_protocol::event::Content::Rich(_)) => {
                                    Ok(MailboxEvent::SigilText(message))
                                }
                                Ok(sigil_protocol::event::Content::Text(_)) => {
                                    Ok(MailboxEvent::Text(message))
                                }
                                Err(error) => Err(error),
                            },
                            Err(error) => Err(error),
                        },
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
                IncomingAttempt {
                    sequence: delivery.sequence,
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

    /// Retry at most 16 durable acknowledgements. Network success followed by a
    /// local failure safely retries the same sequence after restart.
    pub fn acknowledge_incoming_online(&mut self) -> Result<usize, Error> {
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
                "SELECT sequence,0 FROM incoming WHERE acknowledged=0 UNION ALL SELECT sequence,1 FROM retry_incoming WHERE acknowledged=0 UNION ALL SELECT sequence,2 FROM recovered_deliveries WHERE acknowledged=0 UNION ALL SELECT sequence,3 FROM group_incoming WHERE acknowledged=0 ORDER BY sequence LIMIT 16",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut count = 0;
        for (sequence, control) in sequences {
            if control == 3 {
                groups::acknowledge_group(self, sequence)?;
                count += 1;
                continue;
            }
            if control == 2 {
                self.acknowledge_recovered_online(sequence)?;
                count += 1;
                continue;
            }
            if control == 1 {
                self.acknowledge_retry_online(sequence)?;
                count += 1;
                continue;
            }
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let prior = record(&tx, &self.key, &own, sequence)?.ok_or(Error::InvalidStore)?;
            let committed = prior.message(&tx, &self.key)?;
            event::validate(
                &tx,
                &self.key,
                &own_statement,
                &prior.peer,
                &prior.message,
                &committed.plaintext,
                prior.expires,
            )?;
            event::remember(
                &tx,
                &self.key,
                &own_statement,
                &prior.peer,
                &prior.session,
                &committed.plaintext,
            )?;
            tx.commit()?;
            network.acknowledge_delivery(sequence)?;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut current = record(&tx, &self.key, &own, sequence)?.ok_or(Error::InvalidStore)?;
            current.acknowledged = true;
            tx.execute(
                "UPDATE incoming SET acknowledged=1,state=?1 WHERE sequence=?2",
                (current.seal(&self.key, &own, sequence)?, sequence),
            )?;
            tx.commit()?;
            count += 1;
        }
        Ok(count)
    }
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

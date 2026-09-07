//! Signed retry control and durable fresh-claim staging. Never replaces peer trust.
use super::*;
use crate::connection::decode_id;
use serde::{Deserialize, Serialize};
use sigil_protocol::{
    device::SignedBinding,
    mailbox::{Delivery, Receipt, Submit},
    retry::Request,
};
#[path = "recovery_actions.rs"]
mod actions;
#[path = "retry_resolution.rs"]
mod resolution;
#[path = "retry_work.rs"]
mod work;
pub use actions::{RecoveryAction, RecoveryAdvice, RecoveryBlock};
pub use work::RetryAttempt;
#[path = "retry_gc.rs"]
mod gc;
pub use gc::{ControlCleanup, JournalCleanup};

#[derive(Serialize, Deserialize)]
struct Record {
    peer: Id,
    packet: Vec<u8>,
    receipt: Option<Receipt>,
    /// 0 active, 1 server accepted, 2 explicitly cancelled. Never removes proof.
    #[serde(default)]
    finished: u8,
}
fn require_active(record: &Record) -> Result<(), Error> {
    match record.finished {
        0 => Ok(()),
        1 => Err(Error::AlreadyDelivered),
        2 => Err(Error::Cancelled),
        _ => Err(Error::InvalidStore),
    }
}
pub(super) fn cancelled(
    db: &Connection,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<bool, Error> {
    if *session != work::session(id) {
        return Ok(false);
    }
    Ok(read(db, key, "retry_requests", id)?.is_some_and(|record| record.finished == 2))
}
pub(super) fn accepted(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
    expires: u64,
) -> Result<(), Error> {
    if *session != work::session(id) {
        return Ok(());
    }
    if let Some(mut record) = read(tx, key, "retry_requests", id)? {
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        if record.receipt.is_some() || request.expires_at != expires {
            return Err(Error::InvalidStore);
        }
        record.finished = 1;
        save(tx, key, "retry_requests", id, &record)?;
    }
    Ok(())
}
#[derive(Debug, PartialEq, Eq)]
pub struct RetryRequest {
    pub id: Id,
    pub peer: Id,
    pub original_session: Id,
    pub message: Id,
    pub claim: Id,
    pub expires_at: u64,
}
fn id(request: &Request) -> Id {
    Sha256::digest(
        [
            b"Sigil/retry-request-id/v0".as_slice(),
            &request.requester,
            &request.target,
            &request.message,
        ]
        .concat(),
    )
    .into()
}
fn read(db: &Connection, key: &StorageKey, table: &str, id: &Id) -> Result<Option<Record>, Error> {
    let record = decode_record(db, key, table, id)?;
    if let Some(record) = &record {
        let (parent, expires): (Vec<u8>, i64) = db
            .query_row(
                "SELECT parent,expires FROM control_dependencies WHERE kind=?1 AND id=?2",
                (u8::from(table == "retry_requests"), id.as_slice()),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::InvalidStore)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        if parent != request.message || expires as u64 != request.expires_at {
            return Err(Error::InvalidStore);
        }
    }
    Ok(record)
}
fn decode_record(
    db: &Connection,
    key: &StorageKey,
    table: &str,
    id: &Id,
) -> Result<Option<Record>, Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            &format!(
                "SELECT CASE WHEN length(state)<=2048 THEN state END FROM {table} WHERE id=?1"
            ),
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    sealed
        .map(|sealed| {
            let bytes = key.open(&sealed, &binding(21, id, table.as_bytes()))?;
            let record: Record = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
            let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
            if self::id(&request) != *id {
                return Err(Error::InvalidStore);
            }
            let finished: u8 = if table == "retry_requests" {
                db.query_row(
                    "SELECT finished FROM retry_requests WHERE id=?1",
                    [id.as_slice()],
                    |r| r.get(0),
                )?
            } else {
                0
            };
            if record.finished > 2 || finished != record.finished {
                return Err(Error::InvalidStore);
            }
            if record
                .receipt
                .as_ref()
                .is_some_and(|r| r.sequence <= 0 || r.expires_at != request.expires_at)
            {
                return Err(Error::InvalidStore);
            }
            Ok(record)
        })
        .transpose()
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    table: &str,
    id: &Id,
    record: &Record,
) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(record).map_err(|_| Error::InvalidStore)?);
    if bytes.len() + 36 > 2048 {
        return Err(Error::Limit);
    }
    let sealed = key.seal(&bytes, &binding(21, id, table.as_bytes()))?;
    let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
    tx.execute("INSERT INTO control_dependencies VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET parent=excluded.parent,expires=excluded.expires",(u8::from(table=="retry_requests"),id.as_slice(),request.message.as_slice(),request.expires_at as i64))?;
    if table == "retry_requests" {
        tx.execute("INSERT INTO retry_requests(id,state,finished) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,finished=excluded.finished", (id.as_slice(),sealed,record.finished))?;
    } else if table == "retry_outbox" {
        tx.execute("INSERT INTO retry_outbox(id,state,complete) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,complete=excluded.complete",(id.as_slice(),sealed,record.receipt.is_some()))?;
    } else {
        tx.execute(&format!("INSERT INTO {table}(id,state) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state"), (id.as_slice(),sealed))?;
    }
    Ok(())
}
fn verify(request: &Request, identity: &Id) -> Result<(), Error> {
    sigil_crypto::verify_signature(
        identity,
        &request.signing_bytes().map_err(|_| Error::Conflict)?,
        &request.signature,
    )?;
    Ok(())
}
pub(super) fn migrate_lifetime(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    for (kind, table) in [(0, "retry_outbox"), (1, "retry_requests")] {
        let mut query = tx.prepare(&format!("SELECT id FROM {table} ORDER BY id"))?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
            let record = decode_record(tx, key, table, &id)?.ok_or(Error::InvalidStore)?;
            let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
            tx.execute(
                "INSERT INTO control_dependencies VALUES(?1,?2,?3,?4)",
                (
                    kind,
                    id.as_slice(),
                    request.message.as_slice(),
                    request.expires_at as i64,
                ),
            )?;
            if kind == 0 {
                tx.execute(
                    "UPDATE retry_outbox SET complete=?1 WHERE id=?2",
                    (record.receipt.is_some(), id.as_slice()),
                )?;
            }
        }
    }
    let mut query =
        tx.prepare("SELECT sequence,state,acknowledged FROM retry_incoming ORDER BY sequence")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let sealed: Vec<u8> = row.get(1)?;
        let acknowledged: bool = row.get(2)?;
        let own = peers::own(tx, key)?;
        let bytes = key.open(
            &sealed,
            &binding(22, &device_fingerprint(&own)?, &sequence.to_be_bytes()),
        )?;
        if bytes.len() != 33 || bytes[32] != u8::from(acknowledged) {
            return Err(Error::InvalidStore);
        }
        tx.execute(
            "INSERT INTO control_journals VALUES(?1,?2)",
            (sequence, &bytes[..32]),
        )?;
        journal_read(tx, key, &own, sequence)?.ok_or(Error::InvalidStore)?;
    }
    Ok(())
}
fn decode(text: &str) -> Result<Vec<u8>, Error> {
    if !network::valid_hex(
        text,
        sigil_protocol::retry::BYTES * 2,
        sigil_protocol::retry::BYTES * 2,
    ) {
        return Err(Error::Conflict);
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| Error::Conflict)?, 16)
                .map_err(|_| Error::Conflict)
        })
        .collect()
}
fn live(request: &Request, now: u64) -> Result<(), Error> {
    if now == 0
        || now > i64::MAX as u64
        || request.expires_at <= now
        || request.expires_at > now.saturating_add(604800)
    {
        return Err(Error::Expired);
    }
    Ok(())
}
fn inspect(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: Id,
    request: &Request,
    now: u64,
) -> Result<(RetryRequest, Peer), Error> {
    live(request, now)?;
    let (work, peer, obsolete) = inspect_evidence(db, key, own, peer, request)?;
    if obsolete {
        return Err(Error::Obsolete);
    }
    Ok((work, peer))
}
fn inspect_evidence(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: Id,
    request: &Request,
) -> Result<(RetryRequest, Peer, bool), Error> {
    let known = peers::known(db, key, &peer)?;
    if !known.verified
        || request.requester != known.fingerprint
        || request.target != device_fingerprint(own)?
    {
        return Err(Error::Unprepared);
    }
    verify(request, &known.binding.identity)?;
    let (session,metadata):(Vec<u8>,Vec<u8>)=db.query_row("SELECT session,metadata FROM deliveries WHERE id=?1 AND length(session)=32 AND length(metadata)=76",[request.message.as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::NotFound)?;
    let session: Id = session.try_into().map_err(|_| Error::InvalidStore)?;
    if session_peer(db, &session)? != Some(peer) {
        return Err(Error::Conflict);
    }
    let fields = key.open(&metadata, &binding(7, &session, &request.message))?;
    if fields.len() != 40 || fields[..32] != known.binding.device {
        return Err(Error::Conflict);
    }
    let content: Vec<u8> = db.query_row("SELECT content FROM outbox WHERE session=?1 AND id=?2 AND content IS NOT NULL AND length(content)<=65572", (session.as_slice(),request.message.as_slice()), |r| r.get(0)).optional()?.ok_or(Error::NotFound)?;
    let plaintext = key.open(&content, &binding(9, &session, &request.message))?;
    let text =
        sigil_protocol::event::Direct::from_bytes(&plaintext).map_err(|_| Error::InvalidEvent)?;
    if text.sender != request.target || text.recipient != request.requester {
        return Err(Error::Conflict);
    }
    if chain(
        db,
        key,
        "retry_requests",
        (&peer, &request.requester, &request.target),
        request.message,
        text.message,
    )? >= 3
    {
        return Err(Error::Limit);
    }
    let obsolete = match event::require_resend(db, key, own, &peer, &text, true) {
        Ok(()) => false,
        Err(Error::Obsolete) => true,
        Err(error) => return Err(error),
    };
    let id = id(request);
    let claim = Sha256::digest([b"Sigil/retry-claim/v0".as_slice(), &id].concat()).into();
    Ok((
        RetryRequest {
            id,
            peer,
            original_session: session,
            message: request.message,
            claim,
            expires_at: request.expires_at,
        },
        known,
        obsolete,
    ))
}
impl ClientStore {
    /// Reload staged work after restart, rechecking current trust and expiry.
    pub fn retry_request(&mut self, id: Id, now: u64) -> Result<RetryRequest, Error> {
        let own = self.own_device_binding()?;
        let tx = self.db.transaction()?;
        let record = read(&tx, &self.key, "retry_requests", &id)?.ok_or(Error::NotFound)?;
        if record.receipt.is_some() {
            return Err(Error::InvalidStore);
        }
        require_active(&record)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        Ok(inspect(&tx, &self.key, &own, record.peer, &request, now)?.0)
    }
    /// Caller explicitly reports an undecryptable delivery. No payload is echoed;
    /// this control exposes its message ID and device fingerprints to the server.
    pub fn prepare_retry_request(&mut self, failed: &Delivery, now: u64) -> Result<Id, Error> {
        self.prepare_retry_for(failed, now, None)
    }
    fn prepare_retry_for(
        &mut self,
        failed: &Delivery,
        now: u64,
        expected: Option<Id>,
    ) -> Result<Id, Error> {
        if failed.sequence <= 0 {
            return Err(Error::Conflict);
        }
        let own = SignedBinding::from_bytes(&self.own_device_binding()?)
            .map_err(|_| Error::InvalidStore)?;
        let peer = peers::reference(&own.binding.server, &decode_id(&failed.sender_device)?);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let known = peers::known(&tx, &self.key, &peer)?;
        if expected.is_some_and(|id| id != known.fingerprint) {
            return Err(Error::Conflict);
        }
        if !known.verified {
            return Err(Error::Unprepared);
        }
        let mut request = Request {
            message: decode_id(&failed.message_id)?,
            requester: device_fingerprint(&own.to_bytes().map_err(|_| Error::InvalidStore)?)?,
            target: known.fingerprint,
            expires_at: now.saturating_add(604800),
            signature: [0; 64],
        };
        let id = id(&request);
        if gc::retired(&tx, &self.key, &id, &request.requester)?.is_some() {
            return Err(Error::Expired);
        }
        if let Some(prior) = read(&tx, &self.key, "retry_outbox", &id)? {
            let saved = Request::from_bytes(&prior.packet).map_err(|_| Error::InvalidStore)?;
            if prior.peer != peer
                || saved.requester != request.requester
                || saved.target != request.target
                || saved.message != request.message
            {
                return Err(Error::Conflict);
            }
            verify(&saved, &own.binding.identity)?;
            if prior.receipt.is_none() {
                live(&saved, now)?;
            }
            return Ok(id);
        }
        live(&request, now)?;
        if tx.query_row(
            "SELECT count(*) FROM retry_outbox r JOIN control_dependencies d ON d.kind=0 AND d.id=r.id WHERE r.complete=0 AND d.expires>?1",
            [now as i64],
            |r| r.get::<_, i64>(0),
        )? >= 1024
        {
            return Err(Error::Limit);
        }
        request.signature = handshake::identity(&tx, &self.key)?
            .sign(&request.signing_bytes().map_err(|_| Error::Conflict)?)?;
        save(
            &tx,
            &self.key,
            "retry_outbox",
            &id,
            &Record {
                peer,
                packet: request.to_bytes().map_err(|_| Error::Conflict)?,
                receipt: None,
                finished: 0,
            },
        )?;
        tx.commit()?;
        Ok(id)
    }
    /// Sends one exact persisted control. A receipt means server acceptance only.
    pub fn send_retry_request_online(&mut self, id: Id, now: u64) -> Result<Receipt, Error> {
        let network = self.connected_client()?;
        let own = self.own_device_binding()?;
        let record = read(&self.db, &self.key, "retry_outbox", &id)?.ok_or(Error::NotFound)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        let peer = peers::known(&self.db, &self.key, &record.peer)?;
        if !peer.verified
            || request.target != peer.fingerprint
            || request.requester != device_fingerprint(&own)?
        {
            return Err(Error::Unprepared);
        }
        verify(
            &request,
            &SignedBinding::from_bytes(&own)
                .map_err(|_| Error::InvalidStore)?
                .binding
                .identity,
        )?;
        if let Some(receipt) = record.receipt {
            return Ok(receipt);
        }
        live(&request, now)?;
        let receipt = network.submit(&Submit {
            recipient_device: transport::hex(&peer.binding.device),
            message_id: transport::hex(&id),
            payload: transport::hex(&record.packet),
            expires_at: request.expires_at,
        })?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = read(&tx, &self.key, "retry_outbox", &id)?.ok_or(Error::NotFound)?;
        if current.peer != record.peer
            || current.packet != record.packet
            || current.receipt.as_ref().is_some_and(|r| r != &receipt)
        {
            return Err(Error::Conflict);
        }
        current.receipt = Some(Receipt {
            sequence: receipt.sequence,
            expires_at: receipt.expires_at,
        });
        save(&tx, &self.key, "retry_outbox", &id, &current)?;
        tx.commit()?;
        Ok(receipt)
    }
    /// Authenticate and stage a fresh peer-bound claim. No ratchet or trust is
    /// replaced, and no plaintext is resent. Mailbox acknowledgement follows commit.
    pub fn accept_retry_request(
        &mut self,
        delivery: &Delivery,
        now: u64,
    ) -> Result<RetryRequest, Error> {
        match self.route_retry(delivery, now, false)? {
            MailboxEvent::Retry(request) => Ok(request),
            _ => Err(Error::InvalidStore),
        }
    }
    pub(super) fn route_retry(
        &mut self,
        delivery: &Delivery,
        now: u64,
        allow_discard: bool,
    ) -> Result<MailboxEvent, Error> {
        if delivery.sequence <= 0 {
            return Err(Error::Conflict);
        }
        let packet = decode(&delivery.payload)?;
        let request = Request::from_bytes(&packet).map_err(|_| Error::Conflict)?;
        if !allow_discard {
            live(&request, now)?;
        }
        if now == 0 || now > i64::MAX as u64 || request.expires_at > now.saturating_add(604800) {
            return Err(Error::Expired);
        }
        let id = id(&request);
        if decode_id(&delivery.message_id)? != id || delivery.expires_at != request.expires_at {
            return Err(Error::Conflict);
        }
        let own = self.own_device_binding()?;
        let binding = SignedBinding::from_bytes(&own)
            .map_err(|_| Error::InvalidStore)?
            .binding;
        let peer = peers::reference(&binding.server, &decode_id(&delivery.sender_device)?);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (result, known, obsolete) = inspect_evidence(&tx, &self.key, &own, peer, &request)?;
        if let Some(prior) = gc::accepted_retired(&tx, &self.key, &id, &device_fingerprint(&own)?)?
        {
            if prior.peer != peer
                || prior.digest != Id::from(Sha256::digest(&packet))
                || prior.expires != request.expires_at
            {
                return Err(Error::Conflict);
            }
            if !allow_discard {
                return Err(Error::Cancelled);
            }
            journal(&tx, &self.key, &own, delivery.sequence, id)?;
            tx.commit()?;
            return Ok(MailboxEvent::DiscardedRetry(id));
        }
        if obsolete && !allow_discard {
            return Err(Error::Obsolete);
        }
        let discard = obsolete || request.expires_at <= now;
        if let Some(mut prior) = read(&tx, &self.key, "retry_requests", &id)? {
            if prior.peer != peer || prior.packet != packet || prior.receipt.is_some() {
                return Err(Error::Conflict);
            }
            if discard && prior.finished == 0 {
                work::cancel_in(&tx, &self.key, &own, id, &mut prior)?;
                save(&tx, &self.key, "retry_requests", &id, &prior)?;
            }
            journal(&tx, &self.key, &own, delivery.sequence, id)?;
            tx.commit()?;
            return Ok(if allow_discard && (discard || prior.finished != 0) {
                MailboxEvent::DiscardedRetry(id)
            } else {
                MailboxEvent::Retry(result)
            });
        }
        if tx.query_row(
            "SELECT count(*) FROM retry_requests r JOIN control_dependencies d ON d.kind=1 AND d.id=r.id WHERE r.finished=0 AND d.expires>?1",
            [now as i64],
            |r| r.get::<_, i64>(0),
        )? >= 4096
        {
            return Err(Error::Limit);
        }
        if !discard {
            claims::prepare(
                &tx,
                &self.key,
                &binding.identity,
                result.claim,
                known.binding.device,
                known.binding.identity,
                Some(peer),
            )?;
        }
        save(
            &tx,
            &self.key,
            "retry_requests",
            &id,
            &Record {
                peer,
                packet,
                receipt: None,
                finished: if discard { 2 } else { 0 },
            },
        )?;
        journal(&tx, &self.key, &own, delivery.sequence, id)?;
        tx.commit()?;
        Ok(if discard {
            MailboxEvent::DiscardedRetry(id)
        } else {
            MailboxEvent::Retry(result)
        })
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

fn chain(
    db: &Connection,
    key: &StorageKey,
    table: &str,
    scope: (&Id, &Id, &Id),
    mut current: Id,
    root: Id,
) -> Result<usize, Error> {
    for depth in 0..=3 {
        if current == root {
            return Ok(depth);
        }
        if depth == 3 {
            return Err(Error::Limit);
        }
        let record = read(db, key, table, &current)?.ok_or(Error::Conflict)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        if record.peer != *scope.0 || request.requester != *scope.1 || request.target != *scope.2 {
            return Err(Error::Conflict);
        }
        current = request.message;
    }
    Err(Error::Limit)
}
pub(super) fn response_allowed(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    peer: &Peer,
    transport: Id,
    text: &sigil_protocol::event::Direct<'_>,
    expires: u64,
) -> Result<(), Error> {
    let record = read(db, key, "retry_outbox", &transport)?.ok_or(Error::InvalidEvent)?;
    let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
    if expires > request.expires_at {
        return Err(Error::Expired);
    }
    chain(
        db,
        key,
        "retry_outbox",
        (&peer.id, own, &peer.fingerprint),
        transport,
        text.message,
    )?;
    Ok(())
}
pub(super) fn requested(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    peer: &Peer,
    message: Id,
) -> Result<bool, Error> {
    let request = Request {
        message,
        requester: *own,
        target: peer.fingerprint,
        expires_at: 1,
        signature: [0; 64],
    };
    let id = id(&request);
    if read(db, key, "retry_outbox", &id)?.is_none() {
        return Ok(false);
    }
    chain(
        db,
        key,
        "retry_outbox",
        (&peer.id, own, &peer.fingerprint),
        id,
        message,
    )?;
    Ok(true)
}
impl ClientStore {
    /// Re-encrypt retained text on the staged fresh claim. The request ID becomes
    /// its transport ID; the inner logical ID/body/timestamp remain unchanged.
    pub fn resend_event(&mut self, id: Id, now: u64) -> Result<(Id, Vec<u8>), Error> {
        let own = self.own_device_binding()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read(&tx, &self.key, "retry_requests", &id)?.ok_or(Error::NotFound)?;
        require_active(&record)?;
        let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
        let (request, _) = inspect(&tx, &self.key, &own, record.peer, &request, now)?;
        let sealed:Vec<u8>=tx.query_row("SELECT content FROM outbox WHERE session=?1 AND id=?2 AND content IS NOT NULL AND length(content)<=65572",(request.original_session.as_slice(),request.message.as_slice()),|r|r.get(0))?;
        let plaintext = self.key.open(
            &sealed,
            &binding(9, &request.original_session, &request.message),
        )?;
        let session = work::session(&id);
        let packet = claims::start_in(
            &tx,
            &self.key,
            request.claim,
            session,
            id,
            (&plaintext, Some(&own), Some(request.expires_at)),
            now,
        )?;
        tx.commit()?;
        Ok((session, packet))
    }
    pub(super) fn check_retry_send(&mut self, id: Id, now: u64) -> Result<(), Error> {
        if read(&self.db, &self.key, "retry_requests", &id)?.is_some() {
            self.retry_request(id, now)?;
        }
        Ok(())
    }
}

fn journal_read(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    sequence: i64,
) -> Result<Option<(Id, bool)>, Error> {
    let row: Option<(bool, Vec<u8>)> = db.query_row("SELECT acknowledged,CASE WHEN length(state)=69 THEN state END FROM retry_incoming WHERE sequence=?1", [sequence], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((acknowledged, sealed)) = row else {
        return Ok(None);
    };
    let bytes = key.open(
        &sealed,
        &binding(22, &device_fingerprint(own)?, &sequence.to_be_bytes()),
    )?;
    if bytes.len() != 33 || bytes[32] != u8::from(acknowledged) {
        return Err(Error::InvalidStore);
    }
    let id: Id = bytes[..32].try_into().map_err(|_| Error::InvalidStore)?;
    let indexed: Vec<u8> = db
        .query_row(
            "SELECT control FROM control_journals WHERE sequence=?1",
            [sequence],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::InvalidStore)?;
    if indexed != id {
        return Err(Error::InvalidStore);
    }
    let Some(record) = read(db, key, "retry_requests", &id)? else {
        gc::accepted_retired(db, key, &id, &device_fingerprint(own)?)?
            .ok_or(Error::InvalidStore)?;
        return Ok(Some((id, acknowledged)));
    };
    let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
    if record.receipt.is_some() || request.target != device_fingerprint(own)? {
        return Err(Error::InvalidStore);
    }
    Ok(Some((id, acknowledged)))
}
fn journal_save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    sequence: i64,
    id: Id,
    acknowledged: bool,
) -> Result<(), Error> {
    let mut bytes = id.to_vec();
    bytes.push(u8::from(acknowledged));
    let sealed = key.seal(
        &bytes,
        &binding(22, &device_fingerprint(own)?, &sequence.to_be_bytes()),
    )?;
    tx.execute("INSERT INTO retry_incoming VALUES(?1,?2,?3) ON CONFLICT(sequence) DO UPDATE SET acknowledged=excluded.acknowledged,state=excluded.state", (sequence, acknowledged, sealed))?;
    tx.execute("INSERT INTO control_journals VALUES(?1,?2) ON CONFLICT(sequence) DO UPDATE SET control=excluded.control", (sequence,id.as_slice()))?;
    Ok(())
}
fn journal(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    sequence: i64,
    id: Id,
) -> Result<(), Error> {
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM incoming WHERE sequence=?1) OR EXISTS(SELECT 1 FROM recovered_deliveries WHERE sequence=?1) OR EXISTS(SELECT 1 FROM group_incoming WHERE sequence=?1)",
        [sequence],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Conflict);
    }
    if let Some((prior, _)) = journal_read(tx, key, own, sequence)? {
        return if prior == id {
            Ok(())
        } else {
            Err(Error::Conflict)
        };
    }
    journal_save(tx, key, own, sequence, id, false)
}
impl ClientStore {
    pub(super) fn acknowledge_retry_online(&mut self, sequence: i64) -> Result<(), Error> {
        let network = self.connected_client()?;
        let own = self.own_device_binding()?;
        let prior =
            journal_read(&self.db, &self.key, &own, sequence)?.ok_or(Error::InvalidStore)?;
        if prior.1 {
            return Ok(());
        }
        // This proves durable authenticated acceptance, independent of subsequent
        // expiry, deletion or peer blocking. It never authorizes another resend.
        network.acknowledge_delivery(sequence)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = journal_read(&tx, &self.key, &own, sequence)?.ok_or(Error::InvalidStore)?;
        if current.0 != prior.0 {
            return Err(Error::Conflict);
        }
        journal_save(&tx, &self.key, &own, sequence, current.0, true)?;
        tx.commit()?;
        Ok(())
    }
}

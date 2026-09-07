//! Frozen local mailbox requests. Transport authentication and peer/device mapping are external.
use super::*;
use sigil_protocol::mailbox::{Receipt, Submit};

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn metadata(recipient: &Id, expires: u64) -> [u8; 40] {
    let mut bytes = [0; 40];
    bytes[..32].copy_from_slice(recipient);
    bytes[32..].copy_from_slice(&expires.to_be_bytes());
    bytes
}
fn expiry_binding(session: &Id, id: &Id) -> Vec<u8> {
    let mut aad = binding(7, session, id);
    aad.extend_from_slice(b"Sigil/expired-delivery/v0");
    aad
}
pub(super) fn expired(
    db: &Connection,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<bool, Error> {
    type Row = (Option<Vec<u8>>, Option<Vec<u8>>);
    let row: Option<Row> = db.query_row("SELECT CASE WHEN length(metadata)=76 THEN metadata END, CASE WHEN expired IS NULL THEN NULL WHEN length(expired)=44 THEN expired ELSE X'' END FROM deliveries WHERE session=?1 AND id=?2", (session.as_slice(), id.as_slice()), |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((metadata, marker)) = row else {
        return Ok(false);
    };
    let metadata = metadata.ok_or(Error::InvalidStore)?;
    let Some(marker) = marker else {
        return Ok(false);
    };
    let fields = key.open(&metadata, &binding(7, session, id))?;
    let expires = key.open(&marker, &expiry_binding(session, id))?;
    if fields.len() != 40 || expires.len() != 8 || fields[32..] != expires[..] {
        return Err(Error::InvalidStore);
    }
    Ok(true)
}
fn request(
    db: &Connection,
    key: &StorageKey,
    session: &Id,
    id: &Id,
    sealed: &[u8],
    packet: &[u8],
    now: u64,
) -> Result<Submit, Error> {
    let bytes = key.open(sealed, &binding(7, session, id))?;
    if bytes.len() != 40 {
        return Err(Error::InvalidStore);
    }
    if let Some(peer) = session_peer(db, session)? {
        if bytes[..32] != peers::destination(db, key, &peer)? {
            return Err(Error::Conflict);
        }
    }
    let expires_at = u64::from_be_bytes(bytes[32..].try_into().map_err(|_| Error::InvalidStore)?);
    if expires_at <= now || expires_at > i64::MAX as u64 {
        return Err(Error::Expired);
    }
    let packet = open_packet(key, packet, session, id)?;
    Ok(Submit {
        recipient_device: hex(&bytes[..32]),
        message_id: hex(id),
        payload: hex(&packet),
        expires_at,
    })
}
impl ClientStore {
    /// Stop retrying a prepared packet after its frozen expiry. This is not proof
    /// of non-delivery: a delayed authenticated receipt may still record acceptance.
    /// The caller supplies a trusted clock; history, IDs and ratchet state remain.
    pub fn expire_delivery(&mut self, session: Id, id: Id, now: u64) -> Result<bool, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = expire_in(&tx, &self.key, session, id, now)?;
        tx.commit()?;
        Ok(changed)
    }
    /// Whether an authenticated local expiry is recorded; false also covers IDs
    /// without a prepared delivery. Server acceptance supersedes local expiry.
    pub fn delivery_expired(&self, session: Id, id: Id) -> Result<bool, Error> {
        expired(&self.db, &self.key, &session, &id)
    }
    /// Freeze transport fields before handing a committed packet to the network.
    /// A crash before this commit leaves an unprepared packet, never a partial request.
    pub fn prepare_delivery(
        &mut self,
        session: Id,
        id: Id,
        recipient: Id,
        expires_at: u64,
        now: u64,
    ) -> Result<Submit, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = prepare(
            &tx,
            &self.key,
            session,
            id,
            recipient,
            Some(expires_at),
            now,
        )?;
        tx.commit()?;
        Ok(value)
    }

    /// Returns at most 16 frozen requests in queue order. An expired or unprepared
    /// packet stops the batch; callers must resolve it rather than silently skipping it.
    pub fn pending_deliveries(&self, session: Id, now: u64) -> Result<Vec<Submit>, Error> {
        Ok(batch(&self.db, &self.key, session, now, false, 16)?.0)
    }

    pub(super) fn outgoing_batch(
        &mut self,
        session: Id,
        now: u64,
        limit: usize,
    ) -> Result<(Vec<Submit>, usize), Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (requests, expired) = batch(&tx, &self.key, session, now, true, limit)?;
        for id in &expired {
            expire_in(&tx, &self.key, session, *id, now)?;
        }
        tx.commit()?;
        Ok((requests, expired.len()))
    }
}

pub(super) fn expire_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    id: Id,
    now: u64,
) -> Result<bool, Error> {
    let (metadata, receipt, pending): (Vec<u8>, Option<Vec<u8>>, bool) = tx.query_row("SELECT d.metadata,d.receipt,o.packet IS NOT NULL FROM deliveries d JOIN outbox o ON o.session=d.session AND o.id=d.id WHERE d.session=?1 AND d.id=?2 AND length(d.metadata)=76 AND (d.receipt IS NULL OR length(d.receipt)=52)", (session.as_slice(), id.as_slice()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?.ok_or(Error::NotFound)?;
    let fields = key.open(&metadata, &binding(7, &session, &id))?;
    if fields.len() != 40 {
        return Err(Error::InvalidStore);
    }
    let expires = u64::from_be_bytes(fields[32..].try_into().map_err(|_| Error::InvalidStore)?);
    if expires == 0 || expires > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    let already_expired = expired(tx, key, &session, &id)?;
    if let Some(receipt) = receipt {
        if pending
            || already_expired
            || decode_receipt(key, &session, &id, &receipt)?.expires_at != expires
        {
            return Err(Error::InvalidStore);
        }
        return Ok(false);
    }
    if already_expired {
        return if pending {
            Err(Error::InvalidStore)
        } else {
            Ok(false)
        };
    }
    if retry::cancelled(tx, key, &session, &id)? {
        return if pending {
            Err(Error::InvalidStore)
        } else {
            Ok(false)
        };
    }
    if !pending {
        return Err(Error::InvalidStore);
    }
    if now < expires {
        return Err(Error::Expired);
    }
    let marker = key.seal(&expires.to_be_bytes(), &expiry_binding(&session, &id))?;
    tx.execute(
        "UPDATE deliveries SET expired=?1 WHERE session=?2 AND id=?3",
        (marker, session.as_slice(), id.as_slice()),
    )?;
    tx.execute(
        "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
        (session.as_slice(), id.as_slice()),
    )?;
    Ok(true)
}

fn batch(
    db: &Connection,
    key: &StorageKey,
    session: Id,
    now: u64,
    resolve_expired: bool,
    limit: usize,
) -> Result<(Vec<Submit>, Vec<Id>), Error> {
    load(db, key, &session)?;
    let mut statement=db.prepare("SELECT o.id,o.packet,d.metadata FROM outbox o LEFT JOIN deliveries d ON d.id=o.id AND d.session=o.session WHERE o.session=?1 AND o.packet IS NOT NULL ORDER BY o.rowid LIMIT ?2")?;
    let mut rows = statement.query((session.as_slice(), limit.min(16) as i64))?;
    let mut values = Vec::new();
    let mut expired = Vec::new();
    while let Some(row) = rows.next()? {
        let id = row.get_ref(0)?.as_blob().map_err(|_| Error::InvalidStore)?;
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let packet = row.get_ref(1)?.as_blob().map_err(|_| Error::InvalidStore)?;
        if packet.len() > 67266 {
            return Err(Error::InvalidStore);
        }
        let sealed = match row.get_ref(2)? {
            rusqlite::types::ValueRef::Null => return Err(Error::Unprepared),
            value => value.as_blob().map_err(|_| Error::InvalidStore)?,
        };
        if sealed.len() != 76 {
            return Err(Error::InvalidStore);
        }
        match request(db, key, &session, &id, sealed, packet, now) {
            Ok(request) => values.push(request),
            Err(Error::Expired) if resolve_expired => expired.push(id),
            Err(error) => return Err(error),
        }
    }
    Ok((values, expired))
}

fn decode_receipt(
    key: &StorageKey,
    session: &Id,
    id: &Id,
    sealed: &[u8],
) -> Result<Receipt, Error> {
    let bytes = key.open(sealed, &binding(8, session, id))?;
    if bytes.len() != 16 {
        return Err(Error::InvalidStore);
    }
    let receipt = Receipt {
        sequence: i64::from_be_bytes(bytes[..8].try_into().map_err(|_| Error::InvalidStore)?),
        expires_at: u64::from_be_bytes(bytes[8..].try_into().map_err(|_| Error::InvalidStore)?),
    };
    if receipt.sequence <= 0 || receipt.expires_at > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(receipt)
}
impl ClientStore {
    /// Accept only a receipt from the authenticated response to this exact request.
    /// Atomically stores server acceptance and clears the packet. This is not a peer read receipt.
    pub fn acknowledge_sent(
        &mut self,
        session: Id,
        id: Id,
        receipt: &Receipt,
    ) -> Result<(), Error> {
        if receipt.sequence <= 0 || receipt.expires_at > i64::MAX as u64 {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        type Row = (Vec<u8>, Option<Vec<u8>>, bool);
        let (metadata,prior,pending):Row=tx.query_row("SELECT d.metadata,d.receipt,o.packet IS NOT NULL FROM deliveries d JOIN outbox o ON o.session=d.session AND o.id=d.id WHERE d.id=?1 AND d.session=?2 AND length(d.metadata)=76 AND (d.receipt IS NULL OR length(d.receipt)=52)",(id.as_slice(),session.as_slice()),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(Error::NotFound)?;
        let fields = self.key.open(&metadata, &binding(7, &session, &id))?;
        if fields.len() != 40 || fields[32..] != receipt.expires_at.to_be_bytes() {
            return Err(Error::Conflict);
        }
        if let Some(prior) = prior {
            if decode_receipt(&self.key, &session, &id, &prior)? != *receipt || pending {
                return Err(Error::Conflict);
            }
            return Ok(());
        }
        if !pending
            && !expired(&tx, &self.key, &session, &id)?
            && !retry::cancelled(&tx, &self.key, &session, &id)?
        {
            return Err(Error::AlreadyDelivered);
        }
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&receipt.sequence.to_be_bytes());
        bytes[8..].copy_from_slice(&receipt.expires_at.to_be_bytes());
        let sealed = self.key.seal(&bytes, &binding(8, &session, &id))?;
        tx.execute(
            "UPDATE deliveries SET receipt=?1,expired=NULL WHERE id=?2 AND session=?3",
            (sealed, id.as_slice(), session.as_slice()),
        )?;
        tx.execute(
            "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
            (session.as_slice(), id.as_slice()),
        )?;
        retry::accepted(&tx, &self.key, &session, &id, receipt.expires_at)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delivery_receipt(&self, session: Id, id: Id) -> Result<Option<Receipt>, Error> {
        receipt(&self.db, &self.key, session, id)
    }
}
pub(super) fn receipt(
    db: &Connection,
    key: &StorageKey,
    session: Id,
    id: Id,
) -> Result<Option<Receipt>, Error> {
    let sealed:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN receipt IS NULL THEN NULL WHEN length(receipt)=52 THEN receipt ELSE X'' END FROM deliveries WHERE session=?1 AND id=?2",(session.as_slice(),id.as_slice()),|r|r.get(0)).optional()?.ok_or(Error::NotFound)?;
    sealed
        .map(|sealed| decode_receipt(key, &session, &id, &sealed))
        .transpose()
}

pub(super) fn prepare(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    id: Id,
    recipient: Id,
    expires_at: Option<u64>,
    now: u64,
) -> Result<Submit, Error> {
    let (_, state) = load(tx, key, &session)?;
    let prior: Option<(Vec<u8>, Vec<u8>)> = tx.query_row(
        "SELECT session,metadata FROM deliveries WHERE id=?1 AND length(session)=32 AND length(metadata)=76",
        [id.as_slice()], |r| Ok((r.get(0)?, r.get(1)?))
    ).optional()?;
    let sealed = if let Some((owner, sealed)) = prior {
        if owner != session {
            return Err(Error::Conflict);
        }
        let fields = key.open(&sealed, &binding(7, &session, &id))?;
        if fields.len() != 40
            || fields[..32] != recipient
            || expires_at.is_some_and(|v| fields[32..] != v.to_be_bytes())
        {
            return Err(Error::Conflict);
        }
        sealed
    } else {
        let expiry = if !state.peer_confirmed()
            && tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM initiations WHERE session=?1)",
                [session.as_slice()],
                |r| r.get::<_, bool>(0),
            )? {
            handshake::prepare_expiry(tx, key, &session, expires_at, now)?
        } else {
            expires_at.unwrap_or(now.saturating_add(604800))
        };
        if expiry <= now || expiry > i64::MAX as u64 || expiry > now.saturating_add(604800) {
            return Err(Error::Expired);
        }
        key.seal(&metadata(&recipient, expiry), &binding(7, &session, &id))?
    };
    let packet: Option<Vec<u8>> = tx.query_row(
        "SELECT packet FROM outbox WHERE session=?1 AND id=?2 AND (packet IS NULL OR length(packet)<=67266)",
        (session.as_slice(), id.as_slice()), |r| r.get(0)
    ).optional()?.ok_or(Error::NotFound)?;
    let packet = match packet {
        Some(packet) => packet,
        None if retry::cancelled(tx, key, &session, &id)? => return Err(Error::Cancelled),
        None if expired(tx, key, &session, &id)? => return Err(Error::Expired),
        None => return Err(Error::AlreadyDelivered),
    };
    let value = request(tx, key, &session, &id, &sealed, &packet, now)?;
    tx.execute(
        "INSERT INTO deliveries(id,session,metadata) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING",
        (id.as_slice(), session.as_slice(), sealed),
    )?;
    Ok(value)
}

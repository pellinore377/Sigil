//! Acknowledge failed transport only after authenticated logical recovery.
use super::*;
#[derive(Serialize, Deserialize)]
struct Resolution {
    peer: Id,
    failed: Id,
    digest: Id,
    expires: u64,
    session: Id,
    response: Id,
    acknowledged: bool,
}
fn load(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    sequence: i64,
) -> Result<Option<Resolution>, Error> {
    let row: Option<(bool, Vec<u8>)> = db.query_row("SELECT acknowledged,CASE WHEN length(state)<=2048 THEN state END FROM recovered_deliveries WHERE sequence=?1", [sequence], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((acknowledged, bytes)) = row else {
        return Ok(None);
    };
    let bytes = key.open(&bytes, &binding(27, own, &sequence.to_be_bytes()))?;
    let value: Resolution = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
    if value.acknowledged != acknowledged || value.expires == 0 || value.expires > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(Some(value))
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    sequence: i64,
    record: &Resolution,
) -> Result<(), Error> {
    let bytes = serde_json::to_vec(record).map_err(|_| Error::InvalidStore)?;
    if bytes.len() + 36 > 2048 {
        return Err(Error::Limit);
    }
    let sealed = key.seal(&bytes, &binding(27, own, &sequence.to_be_bytes()))?;
    tx.execute("INSERT INTO recovered_deliveries VALUES(?1,?2,?3) ON CONFLICT(sequence) DO UPDATE SET acknowledged=excluded.acknowledged,state=excluded.state", (sequence, record.acknowledged, sealed))?;
    Ok(())
}
pub(super) fn reclaimable(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    sequence: i64,
    now: u64,
) -> Result<bool, Error> {
    let record = load(db, key, own, sequence)?.ok_or(Error::InvalidStore)?;
    Ok(record.acknowledged && record.expires <= now)
}
fn proof(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    record: &Resolution,
) -> Result<(), Error> {
    let peer = peers::known(tx, key, &record.peer)?;
    let fingerprint = device_fingerprint(own)?;
    if chain(
        tx,
        key,
        "retry_outbox",
        (&peer.id, &fingerprint, &peer.fingerprint),
        record.response,
        record.failed,
    )? == 0
    {
        return Err(Error::Conflict);
    }
    let control = read(tx, key, "retry_outbox", &record.response)?.ok_or(Error::InvalidStore)?;
    let request = Request::from_bytes(&control.packet).map_err(|_| Error::InvalidStore)?;
    let sealed: Vec<u8> = tx.query_row(
        "SELECT content FROM inbox WHERE session=?1 AND id=?2 AND length(content)<=65572",
        (record.session.as_slice(), record.response.as_slice()),
        |r| r.get(0),
    )?;
    let plaintext = key.open(&sealed, &binding(2, &record.session, &record.response))?;
    event::validate(
        tx,
        key,
        own,
        &record.peer,
        &record.response,
        &plaintext,
        request.expires_at,
    )?;
    if !event::remember(tx, key, own, &record.peer, &record.session, &plaintext)? {
        return Err(Error::Unprepared);
    }
    Ok(())
}
impl ClientStore {
    /// Journal a failed delivery only if its authenticated replacement is already
    /// accepted locally. Does not acknowledge the server until the normal worker.
    pub fn resolve_failed_delivery(&mut self, failed: &Delivery) -> Result<bool, Error> {
        if failed.sequence <= 0
            || failed.expires_at == 0
            || failed.expires_at > i64::MAX as u64
            || !network::valid_hex(
                &failed.payload,
                32,
                sigil_protocol::mailbox::MAX_PAYLOAD_HEX,
            )
        {
            return Err(Error::Conflict);
        }
        let packet: Vec<u8> = failed
            .payload
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                u8::from_str_radix(std::str::from_utf8(p).map_err(|_| Error::Conflict)?, 16)
                    .map_err(|_| Error::Conflict)
            })
            .collect::<Result<_, _>>()?;
        let own = self.own_device_binding()?;
        let binding = SignedBinding::from_bytes(&own)
            .map_err(|_| Error::InvalidStore)?
            .binding;
        let fingerprint = device_fingerprint(&own)?;
        let peer = peers::reference(&binding.server, &decode_id(&failed.sender_device)?);
        let message = decode_id(&failed.message_id)?;
        let digest = Sha256::digest(packet).into();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM incoming WHERE sequence=?1) OR EXISTS(SELECT 1 FROM retry_incoming WHERE sequence=?1) OR EXISTS(SELECT 1 FROM group_incoming WHERE sequence=?1)", [failed.sequence], |r| r.get::<_, bool>(0))? { return Err(Error::Conflict) }
        if let Some(prior) = load(&tx, &self.key, &fingerprint, failed.sequence)? {
            if prior.peer != peer
                || prior.failed != message
                || prior.digest != digest
                || prior.expires != failed.expires_at
            {
                return Err(Error::Conflict);
            }
            proof(&tx, &self.key, &own, &prior)?;
            tx.commit()?;
            return Ok(true);
        }
        let known = peers::known(&tx, &self.key, &peer)?;
        if !known.verified {
            return Err(Error::Unprepared);
        }
        let mut parent = message;
        for _ in 0..3 {
            let id = super::id(&Request {
                message: parent,
                requester: fingerprint,
                target: known.fingerprint,
                expires_at: 1,
                signature: [0; 64],
            });
            let Some(control) = read(&tx, &self.key, "retry_outbox", &id)? else {
                break;
            };
            if control.peer != peer {
                return Err(Error::Conflict);
            }
            parent = id;
            let session: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT session FROM inbox WHERE id=?1 AND length(session)=32 LIMIT 1",
                    [id.as_slice()],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(session) = session else { continue };
            let record = Resolution {
                peer,
                failed: message,
                digest,
                expires: failed.expires_at,
                session: session.try_into().map_err(|_| Error::InvalidStore)?,
                response: id,
                acknowledged: false,
            };
            proof(&tx, &self.key, &own, &record)?;
            save(&tx, &self.key, &fingerprint, failed.sequence, &record)?;
            tx.commit()?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(crate) fn acknowledge_recovered_online(&mut self, sequence: i64) -> Result<(), Error> {
        let network = self.connected_client()?;
        let own = self.own_device_binding()?;
        let fingerprint = device_fingerprint(&own)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior = load(&tx, &self.key, &fingerprint, sequence)?.ok_or(Error::InvalidStore)?;
        if prior.acknowledged {
            return Ok(());
        }
        proof(&tx, &self.key, &own, &prior)?;
        tx.commit()?;
        network.acknowledge_delivery(sequence)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current =
            load(&tx, &self.key, &fingerprint, sequence)?.ok_or(Error::InvalidStore)?;
        if current.failed != prior.failed
            || current.digest != prior.digest
            || current.peer != prior.peer
            || current.response != prior.response
            || current.session != prior.session
            || current.expires != prior.expires
        {
            return Err(Error::Conflict);
        }
        current.acknowledged = true;
        save(&tx, &self.key, &fingerprint, sequence, &current)?;
        tx.commit()?;
        Ok(())
    }
}

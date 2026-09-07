use super::*;
use crate::{connection::decode_id, transport};
use sigil_protocol::mailbox::{Delivery, Receipt, Submit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupDeliveryStatus {
    Pending,
    Accepted,
    Expired,
    Cancelled,
}
impl GroupDeliveryStatus {
    fn tag(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Accepted => 1,
            Self::Expired => 2,
            Self::Cancelled => 3,
        }
    }
    fn parse(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Accepted),
            2 => Ok(Self::Expired),
            3 => Ok(Self::Cancelled),
            _ => Err(Error::InvalidStore),
        }
    }
}
#[derive(Debug)]
pub struct GroupSendAttempt {
    pub delivery: Id,
    pub result: Result<GroupDeliveryStatus, Error>,
}
pub(super) struct Target {
    pub fingerprint: Id,
    pub peer: Id,
    pub device: Id,
}
struct Job {
    id: Id,
    sequence: i64,
    group: Id,
    message: Id,
    target: Target,
    expires: u64,
    status: GroupDeliveryStatus,
    receipt: i64,
}
impl Job {
    fn seal(&self, key: &StorageKey, own: &Id) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::with_capacity(177);
        for id in [
            &self.group,
            &self.message,
            &self.target.fingerprint,
            &self.target.peer,
            &self.target.device,
        ] {
            bytes.extend_from_slice(id);
        }
        bytes.extend_from_slice(&self.expires.to_be_bytes());
        bytes.push(self.status.tag());
        bytes.extend_from_slice(&self.receipt.to_be_bytes());
        Ok(key.seal(
            &bytes,
            &crate::binding(
                52,
                own,
                &[self.id.as_slice(), &self.sequence.to_be_bytes()].concat(),
            ),
        )?)
    }
    fn save(&self, tx: &Transaction<'_>, key: &StorageKey, own: &Id) -> Result<(), Error> {
        if tx.execute(
            "UPDATE group_delivery SET state=?1,status=?2 WHERE sequence=?3 AND id=?4",
            (
                self.seal(key, own)?,
                self.status.tag(),
                self.sequence,
                self.id.as_slice(),
            ),
        )? != 1
        {
            return Err(Error::InvalidStore);
        }
        Ok(())
    }
}
fn read(db: &Connection, key: &StorageKey, own: &Id, id: &Id) -> Result<Job, Error> {
    type Row = (i64, Vec<u8>, Vec<u8>, Vec<u8>, u8);
    let (sequence, bytes, message, recipient, status): Row = db.query_row("SELECT sequence,CASE WHEN length(state)=213 THEN state END,message,recipient,status FROM group_delivery WHERE id=?1", [id.as_slice()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or(Error::NotFound)?;
    if sequence <= 0 {
        return Err(Error::InvalidStore);
    }
    let bytes = key.open(
        &bytes,
        &crate::binding(52, own, &[id.as_slice(), &sequence.to_be_bytes()].concat()),
    )?;
    if bytes.len() != 177 || bytes[168] != status {
        return Err(Error::InvalidStore);
    }
    let field = |n| <Id>::try_from(&bytes[n..n + 32]).map_err(|_| Error::InvalidStore);
    let job = Job {
        id: *id,
        sequence,
        group: field(0)?,
        message: field(32)?,
        target: Target {
            fingerprint: field(64)?,
            peer: field(96)?,
            device: field(128)?,
        },
        expires: u64::from_be_bytes(
            bytes[160..168]
                .try_into()
                .map_err(|_| Error::InvalidStore)?,
        ),
        status: GroupDeliveryStatus::parse(status)?,
        receipt: i64::from_be_bytes(
            bytes[169..177]
                .try_into()
                .map_err(|_| Error::InvalidStore)?,
        ),
    };
    if message != job.message
        || recipient != recipient_index(key, &job.group, &job.target.fingerprint)?
        || job.expires == 0
        || job.expires > i64::MAX as u64
        || (job.status == GroupDeliveryStatus::Accepted && job.receipt <= 0)
        || (job.status != GroupDeliveryStatus::Accepted && job.receipt != 0)
    {
        return Err(Error::InvalidStore);
    }
    Ok(job)
}
fn validate_job(job: &Job, stored: &Stored) -> Result<(), Error> {
    if !stored.message.outgoing
        || stored.message.context.group != job.group
        || transport_id(
            &stored.message.context,
            &stored.message.event()?.message,
            &job.target.fingerprint,
        ) != job.id
    {
        return Err(Error::InvalidStore);
    }
    Ok(())
}
pub(super) fn enqueue(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    id: &Id,
    content: &GroupMessage,
    target: Target,
    expires: u64,
) -> Result<(), Error> {
    let job_id = transport_id(
        &content.context,
        &content.event()?.message,
        &target.fingerprint,
    );
    tx.execute(
        "INSERT INTO group_delivery(id,message,recipient,state,status) VALUES(?1,?2,?3,x'',0)",
        (
            job_id.as_slice(),
            id.as_slice(),
            recipient_index(key, &content.context.group, &target.fingerprint)?.as_slice(),
        ),
    )?;
    Job {
        id: job_id,
        sequence: tx.last_insert_rowid(),
        group: content.context.group,
        message: *id,
        target,
        expires,
        status: GroupDeliveryStatus::Pending,
        receipt: 0,
    }
    .save(tx, key, own)
}
fn release_packet(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    message: &Id,
) -> Result<(), Error> {
    let ids = tx
        .prepare("SELECT id FROM group_delivery WHERE message=?1 LIMIT ?2")?
        .query_map((message.as_slice(), MAX_DEVICES as u32 + 1), |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty() || ids.len() > MAX_DEVICES {
        return Err(Error::InvalidStore);
    }
    for id in ids {
        let id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let job = read(tx, key, own, &id)?;
        if job.message != *message {
            return Err(Error::InvalidStore);
        }
        if job.status == GroupDeliveryStatus::Pending {
            return Ok(());
        }
    }
    tx.execute(
        "UPDATE group_messages SET packet=NULL WHERE id=?1",
        [message.as_slice()],
    )?;
    Ok(())
}
pub(in crate::groups) fn retire(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
) -> Result<(), Error> {
    let ids = tx
        .prepare("SELECT id FROM group_messages WHERE group_id=?1 AND packet IS NOT NULL LIMIT ?2")?
        .query_map((group.as_slice(), MAX_PENDING + 1), |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if ids.len() > MAX_PENDING as usize {
        return Err(Error::InvalidStore);
    }
    for id in ids {
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let stored = load(tx, key, own, &id)?.ok_or(Error::InvalidStore)?;
        if !stored.message.outgoing || stored.message.context.group != *group {
            return Err(Error::InvalidStore);
        }
        packet(tx, key, own, &id, &stored)?;
        let jobs = tx
            .prepare("SELECT id FROM group_delivery WHERE message=?1 LIMIT ?2")?
            .query_map((id.as_slice(), MAX_DEVICES as u32 + 1), |r| {
                r.get::<_, Vec<u8>>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if jobs.is_empty() || jobs.len() > MAX_DEVICES {
            return Err(Error::InvalidStore);
        }
        for job_id in jobs {
            let job_id = job_id.try_into().map_err(|_| Error::InvalidStore)?;
            let mut job = read(tx, key, own, &job_id)?;
            validate_job(&job, &stored)?;
            if job.message != id || job.group != *group {
                return Err(Error::InvalidStore);
            }
            if job.status == GroupDeliveryStatus::Pending {
                job.status = GroupDeliveryStatus::Cancelled;
                job.save(tx, key, own)?;
            }
        }
        release_packet(tx, key, own, &id)?;
    }
    Ok(())
}
fn cursor(db: &Connection, key: &StorageKey, own: &Id) -> Result<(i64, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(state)=44 THEN state END FROM group_delivery_cursor WHERE id=1", [], |r| r.get(0)).optional()?;
    let value = match &sealed {
        Some(bytes) => i64::from_be_bytes(
            key.open(bytes, &aad(54, own, &[0; 32]))?
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
impl ClientStore {
    pub fn group_delivery_status(&mut self, id: Id) -> Result<GroupDeliveryStatus, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        Ok(read(&tx, &self.key, &own, &id)?.status)
    }
    fn send_group_delivery(&mut self, id: Id, now: u64) -> Result<GroupDeliveryStatus, Error> {
        let network = self.connected_client()?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let before = read(&self.db, &self.key, &own, &id)?;
        if before.status == GroupDeliveryStatus::Pending && before.expires > now {
            self.refresh_group_authority_for_send(before.group, now)?;
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = read(&tx, &self.key, &own, &id)?;
        if job.status != GroupDeliveryStatus::Pending {
            return Ok(job.status);
        }
        let stored = load(&tx, &self.key, &own, &job.message)?.ok_or(Error::InvalidStore)?;
        validate_job(&job, &stored)?;
        let bytes = packet(&tx, &self.key, &own, &job.message, &stored)?;
        if job.expires <= now {
            job.status = GroupDeliveryStatus::Expired;
            job.save(&tx, &self.key, &own)?;
            release_packet(&tx, &self.key, &own, &job.message)?;
            tx.commit()?;
            return Ok(GroupDeliveryStatus::Expired);
        }
        let state = keys::current(&tx, &self.key, &own, &job.group)?;
        keys::matches_state(&state, &stored.message.context)?;
        state.device(job.target.fingerprint)?;
        let known = peers::verified(&tx, &self.key, &job.target.peer)?;
        if known.fingerprint != job.target.fingerprint || known.binding.device != job.target.device
        {
            return Err(Error::Conflict);
        }
        let distribution_id =
            super::super::control::message_id(&stored.message.context, &job.target.fingerprint);
        let (session, receipt) = keys::job(&tx, &self.key, &own, &job.group, &distribution_id)?
            .ok_or(Error::Unprepared)?;
        if receipt.context != stored.message.context || receipt.recipient != job.target.fingerprint
        {
            return Err(Error::InvalidStore);
        }
        if transport::receipt(&tx, &self.key, session, distribution_id)?.is_none() {
            return Err(Error::Unprepared);
        }
        let request = Submit {
            recipient_device: transport::hex(&job.target.device),
            message_id: transport::hex(&id),
            payload: transport::hex(&bytes),
            expires_at: job.expires,
        };
        tx.commit()?;
        let receipt = network.submit(&request)?;
        self.accept_group_receipt(id, &receipt)?;
        Ok(GroupDeliveryStatus::Accepted)
    }
    /// Accept only the authenticated server response to the exact frozen request.
    fn accept_group_receipt(&mut self, id: Id, receipt: &Receipt) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = read(&tx, &self.key, &own, &id)?;
        if receipt.sequence <= 0 || receipt.expires_at != job.expires {
            return Err(Error::Conflict);
        }
        if job.status == GroupDeliveryStatus::Accepted && job.receipt != receipt.sequence {
            return Err(Error::Conflict);
        }
        job.status = GroupDeliveryStatus::Accepted;
        job.receipt = receipt.sequence;
        job.save(&tx, &self.key, &own)?;
        release_packet(&tx, &self.key, &own, &job.message)?;
        tx.commit()?;
        Ok(())
    }
    /// One fair bounded pass. Each recipient's oldest pending packet comes first;
    /// network errors stop the pass, while local failures do not starve peers.
    pub fn resume_group_outbound_online(
        &mut self,
        now: u64,
    ) -> Result<Vec<GroupSendAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        self.connected_client()?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        let rows = self.db.prepare("SELECT sequence,id FROM group_delivery d WHERE status=0 AND sequence>?1 AND NOT EXISTS(SELECT 1 FROM group_delivery earlier WHERE earlier.recipient=d.recipient AND earlier.status=0 AND earlier.sequence<d.sequence) ORDER BY sequence LIMIT 16")?.query_map([after], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,Vec<u8>>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
        let mut attempts = Vec::new();
        let mut next = 0;
        for (sequence, id) in rows {
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            let result = self.send_group_delivery(id, now);
            let stop = matches!(result, Err(Error::Network(_)));
            attempts.push(GroupSendAttempt {
                delivery: id,
                result,
            });
            next = sequence;
            if stop {
                break;
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cursor(&tx, &self.key, &own)?.1 == expected {
            tx.execute("INSERT INTO group_delivery_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [self.key.seal(&next.to_be_bytes(), &aad(54, &own, &[0;32]))?])?;
        }
        tx.commit()?;
        Ok(attempts)
    }
}

struct Received {
    message: Id,
    group: Id,
    peer: Id,
    transport: Id,
    digest: Id,
    expires: u64,
    acknowledged: bool,
}
impl Received {
    fn seal(&self, key: &StorageKey, own: &Id, sequence: i64) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::with_capacity(169);
        for id in [
            &self.message,
            &self.group,
            &self.peer,
            &self.transport,
            &self.digest,
        ] {
            bytes.extend_from_slice(id);
        }
        bytes.extend_from_slice(&self.expires.to_be_bytes());
        bytes.push(self.acknowledged as u8);
        Ok(key.seal(&bytes, &crate::binding(53, own, &sequence.to_be_bytes()))?)
    }
}
fn received(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    sequence: i64,
) -> Result<Option<Received>, Error> {
    let row: Option<(Vec<u8>,bool)> = db.query_row("SELECT CASE WHEN length(state)=205 THEN state END,acknowledged FROM group_incoming WHERE sequence=?1", [sequence], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((sealed, ack)) = row else {
        return Ok(None);
    };
    let bytes = key.open(&sealed, &crate::binding(53, own, &sequence.to_be_bytes()))?;
    if bytes.len() != 169 || bytes[168] != ack as u8 {
        return Err(Error::InvalidStore);
    }
    let id = |n| <Id>::try_from(&bytes[n..n + 32]).map_err(|_| Error::InvalidStore);
    let record = Received {
        message: id(0)?,
        group: id(32)?,
        peer: id(64)?,
        transport: id(96)?,
        digest: id(128)?,
        expires: u64::from_be_bytes(
            bytes[160..168]
                .try_into()
                .map_err(|_| Error::InvalidStore)?,
        ),
        acknowledged: ack,
    };
    if record.expires == 0 || record.expires > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(Some(record))
}
fn committed(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    record: &Received,
) -> Result<GroupMessage, Error> {
    let stored = load(db, key, own, &record.message)?.ok_or(Error::InvalidStore)?;
    let known = peers::known(db, key, &record.peer)?;
    if stored.message.outgoing
        || stored.message.context.group != record.group
        || stored.message.context.sender != known.fingerprint
        || stored.digest != record.digest
        || transport_id(
            &stored.message.context,
            &stored.message.event()?.message,
            own,
        ) != record.transport
    {
        return Err(Error::InvalidStore);
    }
    Ok(stored.message)
}
impl ClientStore {
    /// Authenticate current membership, packet and logical content before
    /// committing receiver advancement, retained history and acknowledgement.
    pub fn accept_group_delivery(&mut self, delivery: &Delivery) -> Result<GroupMessage, Error> {
        if delivery.sequence <= 0
            || delivery.expires_at == 0
            || delivery.expires_at > i64::MAX as u64
            || !crate::network::valid_hex(
                &delivery.payload,
                32,
                sigil_protocol::mailbox::MAX_PAYLOAD_HEX,
            )
        {
            return Err(Error::Conflict);
        }
        let sender = decode_id(&delivery.sender_device)?;
        let transport_id_received = decode_id(&delivery.message_id)?;
        let own_binding = self.own_device_binding()?;
        let own = device_fingerprint(&own_binding)?;
        let own_fields = peers::parse(&own_binding)?.binding;
        let peer = peers::reference(&own_fields.server, &sender);
        let bytes = delivery
            .payload
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| Error::Conflict)?, 16)
                    .map_err(|_| Error::Conflict)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let packet = sk::Packet::from_bytes(&bytes)?;
        let context = packet.context();
        if transport_id(&context, &packet.message(), &own) != transport_id_received {
            return Err(Error::Conflict);
        }
        let digest: Id = Sha256::digest(&bytes).into();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM incoming WHERE sequence=?1) OR EXISTS(SELECT 1 FROM retry_incoming WHERE sequence=?1) OR EXISTS(SELECT 1 FROM recovered_deliveries WHERE sequence=?1)", [delivery.sequence], |r| r.get::<_,bool>(0))? { return Err(Error::Conflict); }
        if let Some(prior) = received(&tx, &self.key, &own, delivery.sequence)? {
            if prior.peer != peer
                || prior.transport != transport_id_received
                || prior.digest != digest
                || prior.expires != delivery.expires_at
            {
                return Err(Error::Conflict);
            }
            return committed(&tx, &self.key, &own, &prior);
        }
        let known = peers::verified(&tx, &self.key, &peer)?;
        if known.fingerprint != context.sender || known.binding.device != sender {
            return Err(Error::Conflict);
        }
        let state = keys::current(&tx, &self.key, &own, &context.group)?;
        keys::matches_state(&state, &context)?;
        let id = index(
            &self.key,
            &context.group,
            &context.sender,
            &packet.message(),
        )?;
        let content = if let Some(prior) = load(&tx, &self.key, &own, &id)? {
            if prior.message.outgoing || prior.message.context != context || prior.digest != digest
            {
                return Err(Error::Conflict);
            }
            prior.message
        } else {
            let (mut receiver, tag) =
                keys::receiver(&tx, &self.key, &own, &state, &context.sender)?
                    .ok_or(Error::Unprepared)?;
            let plaintext = Zeroizing::new(receiver.open(&packet)?);
            let text = Group::from_bytes(&plaintext).map_err(|_| Error::InvalidEvent)?;
            crate::event::validate_content(
                text.content,
                &own_fields.server,
                &text.message,
                &crate::event::account(&known.binding),
                text.timestamp,
            )?;
            if text.group != context.group
                || text.sender != context.sender
                || text.message != packet.message()
            {
                return Err(Error::InvalidEvent);
            }
            let outgoing = own_fields.account == known.binding.account
                && own_fields.server == known.binding.server;
            retain(
                &tx,
                &self.key,
                &own_binding,
                &text,
                outgoing,
                known.binding.identity,
            )?;
            let content = GroupMessage {
                context,
                plaintext,
                outgoing: false,
                duplicate: false,
            };
            save(&tx, &self.key, &own, &content, &bytes)?;
            keys::save_receiver(&tx, &self.key, &own, &receiver, &tag)?;
            content
        };
        let record = Received {
            message: id,
            group: context.group,
            peer,
            transport: transport_id_received,
            digest,
            expires: delivery.expires_at,
            acknowledged: false,
        };
        tx.execute(
            "INSERT INTO group_incoming VALUES(?1,0,?2)",
            (
                delivery.sequence,
                record.seal(&self.key, &own, delivery.sequence)?,
            ),
        )?;
        tx.commit()?;
        Ok(content)
    }
}
pub(crate) fn acknowledge(client: &mut ClientStore, sequence: i64) -> Result<(), Error> {
    let network = client.connected_client()?;
    let own = device_fingerprint(&client.own_device_binding()?)?;
    let tx = client
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let prior = received(&tx, &client.key, &own, sequence)?.ok_or(Error::InvalidStore)?;
    committed(&tx, &client.key, &own, &prior)?;
    tx.commit()?;
    network.acknowledge_delivery(sequence)?;
    let tx = client
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut prior = received(&tx, &client.key, &own, sequence)?.ok_or(Error::InvalidStore)?;
    committed(&tx, &client.key, &own, &prior)?;
    prior.acknowledged = true;
    tx.execute(
        "UPDATE group_incoming SET acknowledged=1,state=?1 WHERE sequence=?2",
        (prior.seal(&client.key, &own, sequence)?, sequence),
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "group_delivery_tests.rs"]
mod tests;

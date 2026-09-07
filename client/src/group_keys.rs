//! Durable sender-key setup over existing independently verified pairwise
//! sessions. Group-scoped contact trust and fresh-channel scheduling remain open.
use super::control::{context_bytes, context_from_bytes, message_id, parse_wire, wire};
use super::*;
use crate::{incoming::Incoming, selection, send_in, transport, ClientStore};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sigil_crypto::{sender_keys as sk, storage::StorageKey};
use zeroize::Zeroizing;

pub(crate) const MIGRATION: &str = "
CREATE TABLE group_senders(group_id BLOB PRIMARY KEY REFERENCES groups(id),state BLOB NOT NULL,seed BLOB);
CREATE TABLE group_receivers(group_id BLOB NOT NULL REFERENCES groups(id),id BLOB NOT NULL,state BLOB NOT NULL,PRIMARY KEY(group_id,id));
CREATE TABLE group_key_outbox(id BLOB PRIMARY KEY REFERENCES deliveries(id),group_id BLOB NOT NULL REFERENCES groups(id),state BLOB NOT NULL);
CREATE INDEX group_key_outbox_group ON group_key_outbox(group_id,id);
CREATE TABLE group_controls(id BLOB PRIMARY KEY,state BLOB NOT NULL);
PRAGMA user_version=44;
";
fn aad(kind: u8, own: &Id, group: &Id, record: &[u8]) -> Vec<u8> {
    crate::binding(kind, group, &[own.as_slice(), record].concat())
}
pub(super) fn current(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
) -> Result<State, Error> {
    let status = store::load(tx, key, own, group).map_err(|e| match e {
        Error::NotFound => Error::Unprepared,
        other => other,
    })?;
    if status.frozen {
        return Err(Error::Conflict);
    }
    if status.state.closed {
        return Err(Error::Obsolete);
    }
    status.state.device(*own)?;
    Ok(status.state)
}
pub(super) fn matches_state(state: &State, context: &sk::Context) -> Result<(), Error> {
    if context.group != state.group || context.state != state.head || context.epoch != state.epoch {
        return Err(Error::Unprepared);
    }
    state.device(context.sender)?;
    Ok(())
}
type StoredSender = (sk::Sender, bool, Option<Vec<u8>>);
pub(super) fn load_sender(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
) -> Result<Option<StoredSender>, Error> {
    type Row = (Vec<u8>, bool, Option<Vec<u8>>);
    let row: Option<Row> = tx.query_row("SELECT CASE WHEN length(state)<=8192 THEN state END,seed IS NULL,CASE WHEN length(seed)=244 THEN seed END FROM group_senders WHERE group_id=?1", [state.group.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
    let Some((sealed, seed_null, seed)) = row else {
        return Ok(None);
    };
    if seed_null != seed.is_none() {
        return Err(Error::InvalidStore);
    }
    let aad = aad(45, own, &state.group, b"sender");
    let bytes = key.open(&sealed, &aad)?;
    if bytes.len() < 173 || bytes[0] > 1 {
        return Err(Error::InvalidStore);
    }
    let context = context_from_bytes(&bytes[1..137])?;
    matches_state(state, &context)?;
    if context.sender != *own || (bytes[0] == 0) != seed.is_some() {
        return Err(Error::InvalidStore);
    }
    Ok(Some((
        sk::Sender::open_checkpoint(key, &bytes[137..], context, &aad)?,
        bytes[0] == 1,
        seed,
    )))
}
pub(super) fn save_sender(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    sender: &sk::Sender,
    ready: bool,
    seed: Option<&[u8]>,
) -> Result<(), Error> {
    if ready == seed.is_some() {
        return Err(Error::InvalidStore);
    }
    let context = sender.context();
    let sender_aad = aad(45, own, &context.group, b"sender");
    let mut bytes = Zeroizing::new(vec![ready as u8]);
    bytes.extend_from_slice(&context_bytes(&context));
    bytes.extend_from_slice(&sender.seal_checkpoint(key, &sender_aad)?);
    let sealed = key.seal(&bytes, &sender_aad)?;
    let seed = seed
        .map(|v| key.seal(v, &aad(46, own, &context.group, &context_bytes(&context))))
        .transpose()?;
    tx.execute("INSERT INTO group_senders VALUES(?1,?2,?3) ON CONFLICT(group_id) DO UPDATE SET state=excluded.state,seed=excluded.seed", (context.group.as_slice(), sealed, seed))?;
    Ok(())
}
fn receiver_id(key: &StorageKey, group: &Id, sender: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[group.as_slice(), sender].concat(),
        b"Sigil/group-receiver-index/v0",
    )?)
}
pub(super) fn receiver(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    fingerprint: &Id,
) -> Result<Option<(sk::Receiver, Id)>, Error> {
    let id = receiver_id(key, &state.group, fingerprint)?;
    let sealed: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)<=8192 THEN state END FROM group_receivers WHERE group_id=?1 AND id=?2", (state.group.as_slice(), id.as_slice()), |r| r.get(0)).optional()?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let aad = aad(47, own, &state.group, &id);
    let bytes = key.open(&sealed, &aad)?;
    if bytes.len() < 204 {
        return Err(Error::InvalidStore);
    }
    let context = context_from_bytes(&bytes[..136])?;
    matches_state(state, &context)?;
    if context.sender != *fingerprint {
        return Err(Error::InvalidStore);
    }
    let tag = bytes[136..168]
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    Ok(Some((
        sk::Receiver::open_checkpoint(key, &bytes[168..], context, &aad)?,
        tag,
    )))
}
pub(super) fn save_receiver(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    receiver: &sk::Receiver,
    tag: &Id,
) -> Result<(), Error> {
    let context = receiver.context();
    let id = receiver_id(key, &context.group, &context.sender)?;
    let aad = aad(47, own, &context.group, &id);
    let mut bytes = Zeroizing::new(context_bytes(&context));
    bytes.extend_from_slice(tag);
    bytes.extend_from_slice(&receiver.seal_checkpoint(key, &aad)?);
    tx.execute("INSERT INTO group_receivers VALUES(?1,?2,?3) ON CONFLICT(group_id,id) DO UPDATE SET state=excluded.state", (context.group.as_slice(), id.as_slice(), key.seal(&bytes, &aad)?))?;
    Ok(())
}
pub(super) fn job(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
    message: &Id,
) -> Result<Option<(Id, DistributionReceipt)>, Error> {
    let sealed: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)=308 THEN state END FROM group_key_outbox WHERE id=?1 AND group_id=?2", (message.as_slice(), group.as_slice()), |r| r.get(0)).optional()?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let bytes = key.open(&sealed, &aad(48, own, group, message))?;
    if bytes.len() != 272 {
        return Err(Error::InvalidStore);
    }
    let session: Id = bytes[..32].try_into().map_err(|_| Error::InvalidStore)?;
    let receipt = distribution_receipt(&bytes[32..])?.ok_or(Error::InvalidStore)?;
    if receipt.message != *message
        || receipt.context.group != *group
        || receipt.context.sender != *own
    {
        return Err(Error::InvalidStore);
    }
    let content: Vec<u8> = tx.query_row(
        "SELECT content FROM outbox WHERE session=?1 AND id=?2 AND length(content)=276",
        (session.as_slice(), message.as_slice()),
        |r| r.get(0),
    )?;
    if key
        .open(&content, &crate::binding(9, &session, message))?
        .as_slice()
        != &bytes[32..]
    {
        return Err(Error::InvalidStore);
    }
    Ok(Some((session, receipt)))
}
fn finish_setup(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    sender: &sk::Sender,
) -> Result<(), Error> {
    let targets: Vec<Id> = state
        .members
        .iter()
        .map(Member::device_fingerprints)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .filter(|v| v != own)
        .collect();
    let count: usize = tx.query_row(
        "SELECT count(*) FROM group_key_outbox WHERE group_id=?1",
        [state.group.as_slice()],
        |r| r.get::<_, u32>(0),
    )? as usize;
    if count != targets.len() {
        return Ok(());
    }
    for target in targets {
        let id = message_id(&sender.context(), &target);
        let (_, receipt) = job(tx, key, own, &state.group, &id)?.ok_or(Error::InvalidStore)?;
        if receipt.context != sender.context() || receipt.recipient != target {
            return Err(Error::InvalidStore);
        }
    }
    save_sender(tx, key, own, sender, true, None)
}

pub(crate) fn validate_distribution_receipt(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    message: &Id,
    plaintext: &[u8],
) -> Result<bool, Error> {
    let Some(receipt) = distribution_receipt(plaintext)? else {
        return Ok(false);
    };
    let own = device_fingerprint(own)?;
    let known = peers::known(db, key, peer)?;
    if receipt.message != *message
        || receipt.recipient != own
        || receipt.context.sender != known.fingerprint
    {
        return Err(Error::Conflict);
    }
    let sealed: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(state)=276 THEN state END FROM group_controls WHERE id=?1",
            [message.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::InvalidStore)?;
    if key
        .open(&sealed, &aad(49, &own, &receipt.context.group, message))?
        .as_slice()
        != plaintext
    {
        return Err(Error::InvalidStore);
    }
    Ok(true)
}
pub(crate) fn install_distribution(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own_binding: &[u8],
    peer: &Id,
    message: &Id,
    plaintext: &[u8],
) -> Result<Option<Vec<u8>>, Error> {
    if !is_wire_control(plaintext) {
        return Ok(None);
    }
    let own = device_fingerprint(own_binding)?;
    let known = peers::verified(tx, key, peer)?;
    let (id, recipient, distribution) = parse_wire(plaintext)?;
    let context = distribution.context();
    if id != *message || recipient != own || context.sender != known.fingerprint {
        return Err(Error::Conflict);
    }
    let marker = retained_payload(key, plaintext)?.into_owned();
    let receipt = distribution_receipt(&marker)?.ok_or(Error::InvalidStore)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_controls WHERE id=?1)",
        [id.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        validate_distribution_receipt(tx, key, own_binding, peer, message, &marker)?;
        return Ok(Some(marker));
    }
    let state = current(tx, key, &own, &context.group)?;
    matches_state(&state, &context)?;
    match receiver(tx, key, &own, &state, &context.sender)? {
        Some((existing, tag)) if existing.context() == context && tag == receipt.tag => {}
        Some(_) => return Err(Error::Conflict),
        None => save_receiver(
            tx,
            key,
            &own,
            &sk::Receiver::from_authenticated_distribution(distribution, context)?,
            &receipt.tag,
        )?,
    }
    let sealed = key.seal(&marker, &aad(49, &own, &context.group, &id))?;
    tx.execute(
        "INSERT INTO group_controls VALUES(?1,?2)",
        (id.as_slice(), sealed),
    )?;
    Ok(Some(marker))
}

/// Atomic local cutover: remove current live group keys and unsent distribution
/// packets. Already in-flight or server-accepted traffic cannot be recalled.
pub(super) fn retire(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
) -> Result<(), Error> {
    super::messages::retire(tx, key, own, group)?;
    let raw = tx
        .prepare("SELECT id FROM group_key_outbox WHERE group_id=?1 LIMIT ?2")?
        .query_map((group.as_slice(), MAX_DEVICES as u32 + 1), |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if raw.len() > MAX_DEVICES {
        return Err(Error::InvalidStore);
    }
    for raw in raw {
        let message: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
        let (session, _) = job(tx, key, own, group, &message)?.ok_or(Error::InvalidStore)?;
        tx.execute(
            "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
            (session.as_slice(), message.as_slice()),
        )?;
    }
    tx.execute(
        "DELETE FROM group_key_outbox WHERE group_id=?1",
        [group.as_slice()],
    )?;
    tx.execute(
        "DELETE FROM group_senders WHERE group_id=?1",
        [group.as_slice()],
    )?;
    tx.execute(
        "DELETE FROM group_receivers WHERE group_id=?1",
        [group.as_slice()],
    )?;
    Ok(())
}
impl Incoming {
    pub fn distribution(&self) -> Result<Option<DistributionReceipt>, Error> {
        distribution_receipt(&self.plaintext)
    }
}
impl ClientStore {
    pub(crate) fn check_group_distribution_send(
        &mut self,
        session: Id,
        message: Id,
        now: u64,
    ) -> Result<(), Error> {
        let group: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT group_id FROM group_key_outbox WHERE id=?1",
                [message.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(group) = group else {
            return Ok(());
        };
        let group: Id = group.try_into().map_err(|_| Error::InvalidStore)?;
        self.refresh_group_authority_for_send(group, now)?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let state = current(&tx, &self.key, &own, &group)?;
        let (expected, receipt) =
            job(&tx, &self.key, &own, &group, &message)?.ok_or(Error::Obsolete)?;
        if session != expected {
            return Err(Error::Conflict);
        }
        matches_state(&state, &receipt.context)
    }
    /// Prepare one exact encrypted distribution through an existing verified
    /// session. No live key is returned or retained in user-message history.
    pub fn prepare_group_distribution(
        &mut self,
        group: Id,
        peer: Id,
        now: u64,
    ) -> Result<Id, Error> {
        let own_binding = self.own_device_binding()?;
        let own = device_fingerprint(&own_binding)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = current(&tx, &self.key, &own, &group)?;
        let known = peers::verified(&tx, &self.key, &peer)?;
        if peers::parse(&own_binding)?.binding.server != known.binding.server {
            return Err(Error::Unprepared);
        }
        state.device(known.fingerprint)?;
        if known.fingerprint == own {
            return Err(Error::Unprepared);
        }
        let (sender, ready, seed) = match load_sender(&tx, &self.key, &own, &state)? {
            Some(v) => v,
            None => {
                let (sender, distribution) = sk::Sender::new(group, state.head, state.epoch, own)?;
                save_sender(
                    &tx,
                    &self.key,
                    &own,
                    &sender,
                    false,
                    Some(&distribution.to_bytes()?),
                )?;
                load_sender(&tx, &self.key, &own, &state)?.ok_or(Error::InvalidStore)?
            }
        };
        let id = message_id(&sender.context(), &known.fingerprint);
        if job(&tx, &self.key, &own, &group, &id)?.is_some() {
            return Ok(id);
        }
        if ready {
            return Err(Error::InvalidStore);
        }
        let seed = self.key.open(
            &seed.ok_or(Error::InvalidStore)?,
            &aad(46, &own, &group, &context_bytes(&sender.context())),
        )?;
        let distribution = sk::Distribution::from_bytes(&seed)?;
        if distribution.context() != sender.context() {
            return Err(Error::InvalidStore);
        }
        let control = wire(&distribution, &known.fingerprint)?;
        let session = selection::for_send(&tx, &self.key, &peer, now)?;
        if crate::session_peer(&tx, &session)? != Some(peer) {
            return Err(Error::Conflict);
        }
        send_in(&tx, &self.key, session, id, &control)?;
        transport::prepare(&tx, &self.key, session, id, known.binding.device, None, now)?;
        let mut record = session.to_vec();
        record.extend_from_slice(&retained_payload(&self.key, &control)?);
        let sealed = self.key.seal(&record, &aad(48, &own, &group, &id))?;
        tx.execute(
            "INSERT INTO group_key_outbox VALUES(?1,?2,?3)",
            (id.as_slice(), group.as_slice(), sealed),
        )?;
        finish_setup(&tx, &self.key, &own, &state, &sender)?;
        tx.commit()?;
        Ok(id)
    }
    /// Ready means every recipient's ciphertext is locally frozen, not read or
    /// installed remotely. Expired/lost distributions require recovery work.
    pub fn group_sender_ready(&mut self, group: Id) -> Result<bool, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let state = current(&tx, &self.key, &own, &group)?;
        Ok(load_sender(&tx, &self.key, &own, &state)?.is_some_and(|(_, ready, _)| ready))
    }
    pub fn group_receiver_ready(&mut self, group: Id, fingerprint: Id) -> Result<bool, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let state = current(&tx, &self.key, &own, &group)?;
        Ok(receiver(&tx, &self.key, &own, &state, &fingerprint)?.is_some())
    }
}

#[cfg(test)]
#[path = "group_key_tests.rs"]
mod tests;

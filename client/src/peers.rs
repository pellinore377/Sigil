//! Cached contact trust and independently checked device identities.
use super::*;
use crate::connection::decode_id as id;
use sigil_crypto::verify_signature;
use sigil_protocol::device::{Binding, SignedBinding, Statement};
#[path = "device_review.rs"]
mod device_review;
pub use device_review::{DeviceReview, DeviceReviewCursor, DeviceReviewPage};
pub(super) const MAX_SESSIONS: usize = 8;
pub(super) fn trusted(db: &Connection, key: &StorageKey, id: &Id) -> Result<Peer, Error> {
    let peer = known(db, key, id)?;
    if !peer.trusted {
        return Err(Error::Unprepared);
    }
    Ok(peer)
}
pub(super) fn known(db: &Connection, key: &StorageKey, id: &Id) -> Result<Peer, Error> {
    load(db, key, id)?.public()
}
pub(super) fn statement(db: &Connection, key: &StorageKey, id: &Id) -> Result<Vec<u8>, Error> {
    load(db, key, id)?
        .signed
        .to_bytes()
        .map_err(|_| Error::InvalidStore)
}

pub(crate) const MIGRATION: &str = "
CREATE TABLE own_device_binding(id INTEGER PRIMARY KEY CHECK(id=1), state BLOB NOT NULL);
CREATE TABLE peers(id BLOB PRIMARY KEY, state BLOB NOT NULL);
PRAGMA user_version=13;";
#[cfg(test)]
#[path = "peer_tests.rs"]
mod tests;

pub struct Peer {
    pub id: Id,
    pub binding: Binding,
    pub fingerprint: Id,
    pub active: bool,
    pub trusted: bool,
    pub verified: bool,
    pub blocked: bool,
    pub changed_fingerprint: Option<Id>,
    pub replaced_by: Option<Id>,
}
struct Record {
    signed: SignedBinding,
    candidate: Option<SignedBinding>,
    trusted: bool,
    verified: bool,
    suspended: bool,
    blocked: bool,
    replacement: Option<(Id, Id)>,
}
pub(super) fn parse(bytes: &[u8]) -> Result<SignedBinding, Error> {
    let signed = SignedBinding::from_bytes(bytes).map_err(|_| Error::InvalidStore)?;
    verify_signature(
        &signed.binding.identity,
        &signed
            .binding
            .signing_bytes()
            .map_err(|_| Error::InvalidStore)?,
        &signed.signature,
    )?;
    Ok(signed)
}
pub(super) fn fingerprint(binding: &Binding) -> Result<Id, Error> {
    Ok(Sha256::digest(binding.signing_bytes().map_err(|_| Error::InvalidStore)?).into())
}
pub fn device_fingerprint(bytes: &[u8]) -> Result<Id, Error> {
    fingerprint(&parse(bytes)?.binding)
}
fn peer_id(binding: &Binding) -> Id {
    reference(&binding.server, &binding.device)
}
pub(super) fn reference(server: &str, device: &Id) -> Id {
    Sha256::digest(
        [
            b"Sigil/peer-reference/v0".as_slice(),
            &(server.len() as u16).to_be_bytes(),
            server.as_bytes(),
            device,
        ]
        .concat(),
    )
    .into()
}
impl Record {
    fn public(&self) -> Result<Peer, Error> {
        Ok(Peer {
            id: peer_id(&self.signed.binding),
            binding: self.signed.binding.clone(),
            fingerprint: fingerprint(&self.signed.binding)?,
            active: !self.suspended && self.replacement.is_none(),
            trusted: self.trusted
                && !self.suspended
                && !self.blocked
                && self.candidate.is_none()
                && self.replacement.is_none(),
            verified: self.verified
                && self.trusted
                && !self.suspended
                && !self.blocked
                && self.candidate.is_none()
                && self.replacement.is_none(),
            blocked: self.blocked,
            changed_fingerprint: self
                .candidate
                .as_ref()
                .map(|v| fingerprint(&v.binding))
                .transpose()?,
            replaced_by: self.replacement.map(|(peer, _)| peer),
        })
    }
}
fn load(db: &Connection, key: &StorageKey, id: &Id) -> Result<Record, Error> {
    let record = decode_record(db, key, id)?;
    let obsolete: bool = db.query_row(
        "SELECT obsolete FROM peers WHERE id=?1",
        [id.as_slice()],
        |r| r.get(0),
    )?;
    if obsolete != record.replacement.is_some() {
        return Err(Error::InvalidStore);
    }
    Ok(record)
}
fn decode_record(db: &Connection, key: &StorageKey, id: &Id) -> Result<Record, Error> {
    let sealed: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=1128 THEN state END FROM peers WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let bytes = key.open(&sealed, &binding(15, id, b"peer"))?;
    if bytes.len() < 4 || bytes[0] > 15 || bytes[1] > 1 {
        return Err(Error::InvalidStore);
    }
    let size =
        u16::from_be_bytes(bytes[2..4].try_into().map_err(|_| Error::InvalidStore)?) as usize;
    let (payload, replacement) = if bytes[0] & 2 != 0 {
        let split = bytes
            .len()
            .checked_sub(64)
            .filter(|split| *split >= 4)
            .ok_or(Error::InvalidStore)?;
        (
            &bytes[4..split],
            Some((
                bytes[split..split + 32]
                    .try_into()
                    .map_err(|_| Error::InvalidStore)?,
                bytes[split + 32..]
                    .try_into()
                    .map_err(|_| Error::InvalidStore)?,
            )),
        )
    } else {
        (&bytes[4..], None)
    };
    let (current, candidate) = payload.split_at_checked(size).ok_or(Error::InvalidStore)?;
    let signed = parse(current)?;
    if peer_id(&signed.binding) != *id {
        return Err(Error::InvalidStore);
    }
    let candidate = if candidate.is_empty() {
        None
    } else {
        Some(parse(candidate)?)
    };
    if candidate
        .as_ref()
        .is_some_and(|v| peer_id(&v.binding) != *id || v.binding == signed.binding)
    {
        return Err(Error::InvalidStore);
    }
    Ok(Record {
        signed,
        candidate,
        trusted: bytes[0] & 5 != 0,
        verified: bytes[0] & 1 != 0,
        suspended: bytes[0] & 8 != 0,
        blocked: bytes[1] == 1,
        replacement,
    })
}
fn save(db: &Connection, key: &StorageKey, id: &Id, record: &Record) -> Result<(), Error> {
    let raw = record.signed.to_bytes().map_err(|_| Error::InvalidStore)?;
    let mut bytes = Zeroizing::new(vec![
        u8::from(record.verified)
            | if record.trusted { 4 } else { 0 }
            | if record.replacement.is_some() { 2 } else { 0 }
            | if record.suspended { 8 } else { 0 },
        u8::from(record.blocked),
    ]);
    bytes.extend_from_slice(&(raw.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&raw);
    if let Some(candidate) = &record.candidate {
        bytes.extend_from_slice(&candidate.to_bytes().map_err(|_| Error::InvalidStore)?);
    }
    if let Some((peer, fingerprint)) = record.replacement {
        bytes.extend_from_slice(&peer);
        bytes.extend_from_slice(&fingerprint);
    }
    let sealed = key.seal(&bytes, &binding(15, id, b"peer"))?;
    db.execute(
        "INSERT INTO peers(id,state,obsolete) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,obsolete=excluded.obsolete",
        (id.as_slice(), sealed, record.replacement.is_some()),
    )?;
    Ok(())
}
pub(super) fn migrate_lifetime(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    let mut query = tx.prepare("SELECT id FROM peers ORDER BY id")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let raw: Vec<u8> = row.get(0)?;
        let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
        let record = decode_record(tx, key, &id)?;
        tx.execute(
            "UPDATE peers SET obsolete=?1 WHERE id=?2",
            (record.replacement.is_some(), id.as_slice()),
        )?;
    }
    Ok(())
}
pub(super) fn require(
    db: &Connection,
    key: &StorageKey,
    peer: &Id,
    recipient: &Id,
    identity: &Id,
) -> Result<(), Error> {
    let record = load(db, key, peer)?;
    if !record.public()?.trusted {
        return Err(Error::Unprepared);
    }
    if record.signed.binding.device != *recipient || record.signed.binding.identity != *identity {
        return Err(Error::Conflict);
    }
    Ok(())
}
pub(super) fn own(tx: &Transaction<'_>, key: &StorageKey) -> Result<Vec<u8>, Error> {
    let session = connection::session_in(tx, key)?.ok_or(Error::InvalidStore)?;
    let device = id(&session.device_id)?;
    let identity = handshake::identity(tx, key)?.public_key();
    let sealed: Vec<u8> = tx.query_row(
        "SELECT state FROM own_device_binding WHERE id=1 AND length(state)<=548",
        [],
        |r| r.get(0),
    )?;
    let raw = key.open(&sealed, &binding(14, &device, &identity))?;
    let signed = parse(&raw)?;
    if signed.binding.device != device || signed.binding.identity != identity {
        return Err(Error::InvalidStore);
    }
    Ok(raw.to_vec())
}
impl ClientStore {
    /// Stable public QR material, signed by this installation's encryption key.
    pub fn own_device_binding(&mut self) -> Result<Vec<u8>, Error> {
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let (username, server) = session
            .address
            .strip_prefix('@')
            .and_then(|s| s.split_once(':'))
            .ok_or(Error::InvalidStore)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = handshake::identity(&tx, &self.key)?;
        let value = Binding {
            server: server.into(),
            username: username.into(),
            account: id(&session.account_id)?,
            device: id(&session.device_id)?,
            identity: identity.public_key(),
        };
        let aad = binding(14, &value.device, &value.identity);
        let prior: Option<Vec<u8>> = tx
            .query_row(
                "SELECT state FROM own_device_binding WHERE id=1 AND length(state)<=548",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            let raw = self.key.open(&prior, &aad)?;
            if parse(&raw)?.binding != value {
                return Err(Error::Conflict);
            }
            return Ok(raw.to_vec());
        }
        let signature = identity.sign(&value.signing_bytes().map_err(|_| Error::InvalidStore)?)?;
        let bytes = SignedBinding {
            binding: value,
            signature,
        }
        .to_bytes()
        .map_err(|_| Error::InvalidStore)?;
        tx.execute(
            "INSERT INTO own_device_binding VALUES(1,?1)",
            [self.key.seal(&bytes, &aad)?],
        )?;
        tx.commit()?;
        Ok(bytes)
    }
    pub fn publish_device_binding_online(&mut self) -> Result<(), Error> {
        let client = self.connected_client()?;
        let bytes = self.own_device_binding()?;
        Ok(client.publish_device_binding(&Statement {
            statement: transport::hex(&bytes),
        })?)
    }
    pub fn fetch_peer_online(&mut self, device: Id) -> Result<Peer, Error> {
        let statement = self
            .connected_client()?
            .device_binding(&transport::hex(&device))?;
        self.observe_peer_binding(&statement.bytes().map_err(|_| Error::InvalidStore)?)
    }
    /// Records a candidate only. Changed bindings for an existing device remain
    /// quarantined; replaying the original cannot clear the warning or transfer trust.
    pub fn observe_peer_binding(&mut self, bytes: &[u8]) -> Result<Peer, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = observe(&tx, &self.key, bytes)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn peer(&self, id: Id) -> Result<Peer, Error> {
        load(&self.db, &self.key, &id)?.public()
    }
}
pub(crate) fn observe(tx: &Transaction<'_>, key: &StorageKey, bytes: &[u8]) -> Result<Peer, Error> {
    let signed = parse(bytes)?;
    let id = peer_id(&signed.binding);
    let mut record = match load(tx, key, &id) {
        Ok(record) => record,
        Err(Error::NotFound) => {
            if tx.query_row("SELECT count(*) FROM peers WHERE obsolete=0", [], |r| {
                r.get::<_, i64>(0)
            })? >= 4096
            {
                return Err(Error::Limit);
            }
            Record {
                signed: signed.clone(),
                candidate: None,
                trusted: false,
                verified: false,
                suspended: false,
                blocked: false,
                replacement: None,
            }
        }
        Err(error) => return Err(error),
    };
    if record.signed.binding != signed.binding {
        record.candidate = Some(signed);
    }
    save(tx, key, &id, &record)?;
    record.public()
}
impl ClientStore {
    /// The full fingerprint must be independently compared or scanned from the
    /// intended peer. Network acquisition alone never authorizes this call.
    pub fn confirm_peer(&mut self, id: Id, expected_fingerprint: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = load(&tx, &self.key, &id)?;
        if record.candidate.is_some()
            || record.replacement.is_some()
            || fingerprint(&record.signed.binding)? != expected_fingerprint
        {
            return Err(Error::Conflict);
        }
        record.trusted = true;
        record.verified = true;
        save(&tx, &self.key, &id, &record)?;
        tx.commit()?;
        Ok(())
    }
    pub fn block_peer(&mut self, id: Id, blocked: bool) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        block(&tx, &self.key, &id, blocked)?;
        tx.commit()?;
        Ok(())
    }
    /// Independently compare the complete new fingerprint before approval.
    /// Replacement is irreversible locally: the old reference can never resume
    /// encryption trust through unblock, confirmation or directory replay.
    pub fn approve_peer_replacement(
        &mut self,
        old: Id,
        new: Id,
        expected_old: Id,
        expected_new: Id,
    ) -> Result<(), Error> {
        if old == new {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut previous = load(&tx, &self.key, &old)?;
        let mut next = load(&tx, &self.key, &new)?;
        let a = &previous.signed.binding;
        let b = &next.signed.binding;
        if fingerprint(a)? != expected_old
            || fingerprint(b)? != expected_new
            || a.server != b.server
            || a.account != b.account
            || a.username != b.username
            || a.identity == b.identity
            || next.blocked
            || next.candidate.is_some()
            || next.replacement.is_some()
        {
            return Err(Error::Conflict);
        }
        if let Some(replacement) = previous.replacement {
            return if replacement == (new, expected_new) {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        if let Some((session, _)) = selection::record(&tx, &self.key, &old)? {
            selection::retire(&tx, &self.key, &old, &session)?;
        }
        previous.replacement = Some((new, expected_new));
        next.trusted = true;
        next.verified = true;
        save(&tx, &self.key, &old, &previous)?;
        save(&tx, &self.key, &new, &next)?;
        tx.commit()?;
        Ok(())
    }
    pub fn prepare_peer_claim(&mut self, request: Id, peer: Id) -> Result<(), Error> {
        self.connected_client()?;
        let identity = self.identity()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, &self.key, &peer)?;
        if !record.public()?.trusted {
            return Err(Error::Unprepared);
        }
        claims::prepare(
            &tx,
            &self.key,
            &identity,
            request,
            record.signed.binding.device,
            record.signed.binding.identity,
            (Some(peer), None),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn accept_peer_initial(
        &mut self,
        peer: Id,
        session: Id,
        message: Id,
        packet: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let slot = self.initial_prekey_slot(packet)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, &self.key, &peer)?;
        if !record.public()?.trusted {
            return Err(Error::Unprepared);
        }
        let plaintext = handshake::accept(
            &tx,
            &self.key,
            slot,
            session,
            message,
            record.signed.binding.identity,
            packet,
        )?;
        bind_session(&tx, &self.key, &session, &peer)?;
        if groups::is_wire_control(&plaintext) {
            return Err(Error::Unprepared);
        }
        tx.commit()?;
        Ok(plaintext)
    }
}

pub(super) fn destination(db: &Connection, key: &StorageKey, peer: &Id) -> Result<Id, Error> {
    Ok(trusted(db, key, peer)?.binding.device)
}
pub(super) fn block(
    tx: &Transaction<'_>,
    key: &StorageKey,
    id: &Id,
    blocked: bool,
) -> Result<(), Error> {
    let mut record = load(tx, key, id)?;
    record.blocked = blocked;
    save(tx, key, id, &record)
}
pub(super) fn bind_session(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    peer: &Id,
) -> Result<(), Error> {
    destination(tx, key, peer)?;
    if let Some(current) = session_peer(tx, session)? {
        if current != *peer {
            return Err(Error::Conflict);
        }
        super::load(tx, key, session)?;
        return Ok(());
    }
    if tx.query_row(
        "SELECT count(*) FROM sessions WHERE peer=?1 AND retired=0",
        [peer.as_slice()],
        |r| r.get::<_, i64>(0),
    )? >= MAX_SESSIONS as i64
    {
        return Err(Error::Limit);
    }
    let (revision, state) = super::load(tx, key, session)?;
    let sealed = state.seal_checkpoint(key, &state_binding(session, revision, Some(*peer)))?;
    if tx.execute(
        "UPDATE sessions SET peer=?1,state=?2 WHERE id=?3 AND revision=?4 AND peer IS NULL",
        (peer.as_slice(), sealed, session.as_slice(), revision),
    )? != 1
    {
        return Err(Error::Conflict);
    }
    selection::activate(tx, key, peer, session)?;
    Ok(())
}

pub(super) fn link_trust(
    tx: &Transaction<'_>,
    key: &StorageKey,
    signed: SignedBinding,
    expected: Id,
) -> Result<Id, Error> {
    if fingerprint(&signed.binding)? != expected {
        return Err(Error::Conflict);
    }
    let id = peer_id(&signed.binding);
    let mut record = match load(tx, key, &id) {
        Ok(record) => record,
        Err(Error::NotFound) => {
            if tx.query_row("SELECT count(*) FROM peers WHERE obsolete=0", [], |r| {
                r.get::<_, i64>(0)
            })? >= 4096
            {
                return Err(Error::Limit);
            }
            Record {
                signed: signed.clone(),
                candidate: None,
                trusted: false,
                verified: false,
                suspended: false,
                blocked: false,
                replacement: None,
            }
        }
        Err(error) => return Err(error),
    };
    if record.signed.binding != signed.binding
        || record.candidate.is_some()
        || record.blocked
        || record.replacement.is_some()
    {
        return Err(Error::Conflict);
    }
    record.trusted = true;
    record.verified = true;
    save(tx, key, &id, &record)?;
    Ok(id)
}
impl ClientStore {
    /// Apply an endorsement from an already trusted sponsor, never
    /// from an account directory alone. Only the exact approved child gains trust.
    pub fn accept_linked_peer(
        &mut self,
        proof: &sigil_protocol::link::Proof,
        sponsor: Id,
        now: u64,
    ) -> Result<Id, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let trusted = trusted(&tx, &self.key, &sponsor)?;
        if trusted.binding != proof.sponsor.binding {
            return Err(Error::Conflict);
        }
        sigil_crypto::link::verify(proof, trusted.fingerprint, now)?;
        let id = link_trust(
            &tx,
            &self.key,
            proof.joining.clone(),
            proof.transcript.joining,
        )?;
        let mut child = load(&tx, &self.key, &id)?;
        child.verified = trusted.verified;
        save(&tx, &self.key, &id, &child)?;
        tx.commit()?;
        Ok(id)
    }
}
impl ClientStore {
    pub(super) fn reconcile_contact_trust(
        &mut self,
        directory: &sigil_protocol::admin::ContactDirectory,
        address: (&str, &str, Id),
        accepted: bool,
        approval: Option<Id>,
        now: u64,
    ) -> Result<Option<Id>, Error> {
        use std::collections::BTreeMap;
        let (server, username, account) = address;
        if !directory.account.valid_for(username, server)
            || directory.account.account != transport::hex(&account)
            || directory.bindings.len() > 64
            || directory.links.len() > 128
        {
            return Err(Error::Conflict);
        }
        let mut current = BTreeMap::new();
        for raw in &directory.bindings {
            let bytes = Statement {
                statement: raw.clone(),
            }
            .bytes()
            .map_err(|_| Error::InvalidStore)?;
            let signed = parse(&bytes)?;
            let b = &signed.binding;
            if b.server != server
                || b.username != username
                || b.account != account
                || !directory
                    .account
                    .devices
                    .contains(&transport::hex(&b.device))
                || current.insert(peer_id(b), signed.clone()).is_some()
            {
                return Err(Error::Conflict);
            }
        }
        let mut proofs = Vec::new();
        for raw in &directory.links {
            let proof = sigil_protocol::link::Authorization { proof: raw.clone() }
                .parse()
                .map_err(|_| Error::InvalidStore)?;
            let b = &proof.sponsor.binding;
            if b.server != server
                || b.username != username
                || b.account != account
                || proof.transcript.created_at > now
            {
                return Err(Error::Conflict);
            }
            // Enrollment expiry limits consent use, not the lifetime of its endorsement.
            sigil_crypto::link::verify(&proof, fingerprint(b)?, proof.transcript.created_at)?;
            proofs.push(proof);
        }
        let digest: Id = Sha256::digest(
            [
                b"Sigil/contact-identity-review/v1\0".as_slice(),
                server.as_bytes(),
                &account,
                &current
                    .values()
                    .map(|s| fingerprint(&s.binding))
                    .collect::<Result<Vec<_>, _>>()?
                    .concat(),
            ]
            .concat(),
        )
        .into();
        if approval.is_some_and(|expected| expected != digest)
            || (approval.is_some() && (current.is_empty() || !accepted))
        {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = tx
            .prepare("SELECT id FROM peers WHERE obsolete=0")?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut records = BTreeMap::new();
        let mut former_account = Vec::new();
        for raw in ids {
            let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
            let record = load(&tx, &self.key, &id)?;
            let b = &record.signed.binding;
            if b.server == server && b.account == account {
                records.insert(id, record);
            } else if b.server == server && b.username == username && record.trusted {
                former_account.push((id, record));
            }
        }
        let mut anchors = BTreeMap::new();
        for record in records
            .values()
            .filter(|r| r.trusted && !r.blocked && r.candidate.is_none())
        {
            anchors.insert(fingerprint(&record.signed.binding)?, record.verified);
        }
        let established = !former_account.is_empty()
            || !anchors.is_empty()
            || records.values().any(|r| r.trusted);
        for _ in 0..proofs.len() {
            let mut progress = false;
            for proof in &proofs {
                if let Some(verified) = anchors.get(&proof.transcript.sponsor).copied() {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        anchors.entry(proof.transcript.joining)
                    {
                        entry.insert(verified);
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
        }
        let changed = established
            && current
                .values()
                .any(|s| fingerprint(&s.binding).is_ok_and(|fp| !anchors.contains_key(&fp)));
        let paused = changed && approval.is_none();
        for (id, record) in &mut former_account {
            record.suspended = true;
            if approval.is_some() {
                if let Some((session, _)) = selection::record(&tx, &self.key, id)? {
                    selection::retire(&tx, &self.key, id, &session)?;
                }
                let (next, signed) = current.first_key_value().ok_or(Error::Conflict)?;
                record.replacement = Some((*next, fingerprint(&signed.binding)?));
            }
            save(&tx, &self.key, id, record)?;
        }
        for (id, signed) in &current {
            let raw = signed.to_bytes().map_err(|_| Error::InvalidStore)?;
            observe(&tx, &self.key, &raw)?;
            let mut record = load(&tx, &self.key, id)?;
            if record.replacement.is_some() {
                return Err(Error::Conflict);
            }
            if record.candidate.is_some() && approval.is_some() {
                if let Some((session, _)) = selection::record(&tx, &self.key, id)? {
                    selection::retire(&tx, &self.key, id, &session)?;
                }
                record.signed = signed.clone();
                record.candidate = None;
                record.verified = false;
            }
            if accepted
                && (!established
                    || approval.is_some()
                    || anchors.contains_key(&fingerprint(&signed.binding)?))
            {
                record.trusted = true;
                record.verified |= anchors
                    .get(&fingerprint(&signed.binding)?)
                    .copied()
                    .unwrap_or(false);
            }
            records.insert(*id, record);
        }
        for (id, record) in &mut records {
            record.suspended = paused || !current.contains_key(id);
            if approval.is_some() && !current.contains_key(id) {
                if let Some((session, _)) = selection::record(&tx, &self.key, id)? {
                    selection::retire(&tx, &self.key, id, &session)?;
                }
                let (next, signed) = current.first_key_value().ok_or(Error::Conflict)?;
                record.replacement = Some((*next, fingerprint(&signed.binding)?));
            }
            save(&tx, &self.key, id, record)?;
        }
        tx.commit()?;
        Ok(paused.then_some(digest))
    }
}

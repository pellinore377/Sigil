use super::*;
use sha2::{Digest, Sha256};
use sigil_crypto::{
    handshake::{initiate_session, Bundle, InitialMessage, Receiver},
    IdentityKey,
};
use sigil_protocol::initial as envelope;

pub(super) fn create_prekey_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    id: Id,
    include_ec: bool,
) -> Result<Vec<u8>, Error> {
    let identity = identity(tx, key)?;
    let prior: Option<Option<Vec<u8>>> = tx
        .query_row(
            "SELECT state FROM prekeys WHERE id=?1 AND (state IS NULL OR length(state)<4096)",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let aad = binding(4, &id, &identity.public_key());
    if let Some(prior) = prior {
        let sealed = prior.ok_or(Error::AlreadyDelivered)?;
        let receiver = Receiver::open_checkpoint(key, &sealed, &aad, &identity.public_key())?;
        let bytes = receiver.bundle()?.to_bytes();
        if (bytes.len() == 1805) != include_ec {
            return Err(Error::Conflict);
        }
        return Ok(bytes);
    }
    let live: i64 = tx.query_row(
        "SELECT count(*) FROM prekeys WHERE state IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    if live >= 64 {
        return Err(Error::Limit);
    }
    let receiver = Receiver::generate(&identity, include_ec)?;
    let sealed = receiver.seal_checkpoint(key, &aad)?;
    tx.execute("INSERT INTO prekeys VALUES(?1,?2)", (id.as_slice(), sealed))?;
    let bundle = receiver.bundle()?.to_bytes();
    Ok(bundle)
}

pub(super) fn identity(tx: &Transaction<'_>, key: &StorageKey) -> Result<IdentityKey, Error> {
    let sealed: Option<Vec<u8>> = tx
        .query_row(
            "SELECT state FROM identity WHERE id=1 AND length(state)<128",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(sealed) = sealed {
        return Ok(IdentityKey::open_checkpoint(
            key,
            &sealed,
            b"Sigil/client/identity/v0",
        )?);
    }
    let identity = IdentityKey::generate()?;
    let sealed = identity.seal_checkpoint(key, b"Sigil/client/identity/v0")?;
    tx.execute("INSERT INTO identity VALUES(1,?1)", [sealed])?;
    Ok(identity)
}

impl ClientStore {
    /// Authenticated peer traffic, not server acceptance, establishes confirmation.
    pub fn session_peer_confirmed(&self, session: Id) -> Result<bool, Error> {
        Ok(load(&self.db, &self.key, &session)?.1.peer_confirmed())
    }
    /// Commit the initial packet and new session before exposing either to transport.
    /// Expected recipient identity must be independently verified by the caller.
    pub fn start_initial(
        &mut self,
        session: Id,
        message: Id,
        expected_recipient: Id,
        bundle: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let packet = start(
            &tx,
            &self.key,
            session,
            message,
            expected_recipient,
            bundle,
            plaintext,
        )?;
        tx.commit()?;
        Ok(packet)
    }
    /// Creates the installation's immutable local encryption identity if absent.
    pub fn identity(&mut self) -> Result<Id, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let public = identity(&tx, &self.key)?.public_key();
        tx.commit()?;
        Ok(public)
    }

    /// Commit private prekeys before returning a publishable public bundle.
    /// The caller retains this local slot ID; it is distinct from the server upload ID.
    pub fn create_prekey(&mut self, id: Id, include_ec: bool) -> Result<Vec<u8>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let bundle = create_prekey_in(&tx, &self.key, id, include_ec)?;
        tx.commit()?;
        Ok(bundle)
    }

    /// Caller independently verifies expected_sender. Only success permits server acknowledgement.
    /// The one-time slot, new session and accepted initial plaintext commit together.
    pub fn accept_initial(
        &mut self,
        slot: Id,
        session: Id,
        message: Id,
        expected_sender: Id,
        packet: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let plaintext = accept(
            &tx,
            &self.key,
            slot,
            session,
            message,
            expected_sender,
            packet,
        )?;
        if groups::is_wire_control(&plaintext) {
            return Err(Error::Unprepared);
        }
        tx.commit()?;
        Ok(plaintext)
    }
}

pub(super) fn start(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    message: Id,
    expected_recipient: Id,
    bundle: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    if plaintext.len() > envelope::MAX_PLAINTEXT {
        return Err(Error::Limit);
    }
    let parsed = Bundle::from_bytes(bundle, &expected_recipient)?;
    let identity = identity(tx, key)?;
    let mut aad = binding(6, &session, &message);
    aad.extend_from_slice(&expected_recipient);
    aad.extend_from_slice(&identity.public_key());
    aad.extend_from_slice(&Sha256::digest(bundle));
    let tag = key.commitment(plaintext, &aad)?;
    if let Some(packet) = retry_outgoing(tx, key, &session, &message, &tag)? {
        return Ok(packet);
    }
    let prekey = parsed.prekey_id();
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM initiations WHERE prekey=?1)",
        [prekey.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Conflict);
    }
    let (mut state, initial) = initiate_session(&identity, &expected_recipient, &parsed, b"")?;
    let bootstrap = state.send(plaintext)?.to_bytes();
    let packet = envelope::encode(&initial.to_bytes(), &bootstrap).map_err(|_| Error::Limit)?;
    insert(tx, key, &session, &state)?;
    save_header(tx, key, &session, &initial.to_bytes(), &[])?;
    queue(tx, key, &session, &message, &tag, &packet, plaintext)?;
    tx.execute(
        "INSERT INTO initiations VALUES(?1,?2)",
        (prekey.as_slice(), session.as_slice()),
    )?;
    Ok(packet)
}

pub(super) fn accept(
    tx: &Transaction<'_>,
    key: &StorageKey,
    slot: Id,
    session: Id,
    message: Id,
    expected_sender: Id,
    packet: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let (initial, bootstrap) = envelope::decode(packet).map_err(|_| Error::UnsupportedSession)?;
    let initial = InitialMessage::from_bytes(initial)?;
    let bootstrap = Packet::from_bytes(bootstrap)?;
    if !initial.is_triple_ratchet() {
        return Err(Error::UnsupportedSession);
    }
    let identity = identity(tx, key)?;
    let extra = [slot.as_slice(), &expected_sender].concat();
    if let Some(header) = header(tx, key, &session)? {
        if header.as_slice() != [initial.to_bytes().as_slice(), &extra].concat() {
            return Err(Error::Conflict);
        }
        let plaintext = receive_in(tx, key, session, message, packet, &bootstrap)?;
        if plaintext.len() > envelope::MAX_PLAINTEXT {
            return Err(Error::Limit);
        }
        return Ok(plaintext);
    }
    let aad = binding(2, &session, &message);
    let tag = key.commitment(&Sha256::digest(packet), &aad)?;
    let sealed: Option<Vec<u8>> = tx
        .query_row(
            "SELECT state FROM prekeys WHERE id=?1 AND (state IS NULL OR length(state)<4096)",
            [slot.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let sealed = sealed.ok_or(Error::AlreadyDelivered)?;
    let mut receiver = Receiver::open_checkpoint(
        key,
        &sealed,
        &binding(4, &slot, &identity.public_key()),
        &identity.public_key(),
    )?;
    let (mut state, empty) = receiver.accept_session(&identity, &expected_sender, &initial)?;
    if !Zeroizing::new(empty).is_empty() {
        return Err(Error::Conflict);
    }
    let plaintext = Zeroizing::new(state.receive(&bootstrap)?);
    if plaintext.len() > envelope::MAX_PLAINTEXT {
        return Err(Error::Conflict);
    }
    insert(tx, key, &session, &state)?;
    save_header(tx, key, &session, &initial.to_bytes(), &extra)?;
    let content = key.seal(&groups::retained_payload(key, &plaintext)?, &aad)?;
    tx.execute(
        "INSERT INTO inbox VALUES(?1,?2,?3,?4)",
        (
            session.as_slice(),
            message.as_slice(),
            tag.as_slice(),
            content,
        ),
    )?;
    tx.execute(
        "UPDATE prekeys SET state=NULL WHERE id=?1",
        [slot.as_slice()],
    )?;
    Ok(plaintext)
}

fn header(
    db: &Connection,
    key: &StorageKey,
    session: &Id,
) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    let sealed: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(data) IN (1730,1738,1794) THEN data END FROM initial_headers WHERE session=?1", [session.as_slice()], |r| r.get(0)).optional()?;
    sealed
        .map(|sealed| {
            key.open(&sealed, &binding(20, session, b"initial header"))
                .map_err(Error::from)
        })
        .transpose()
}
fn save_header(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    initial: &[u8],
    extra: &[u8],
) -> Result<(), Error> {
    let sealed = key.seal(
        &[initial, extra].concat(),
        &binding(20, session, b"initial header"),
    )?;
    tx.execute(
        "INSERT INTO initial_headers VALUES(?1,?2)",
        (session.as_slice(), sealed),
    )?;
    Ok(())
}
pub(super) fn wrap(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    state: &Session,
    packet: Vec<u8>,
    plaintext_len: usize,
) -> Result<Vec<u8>, Error> {
    if state.peer_confirmed() {
        return Ok(packet);
    }
    if let Some(header) = header(tx, key, session)? {
        if plaintext_len > envelope::MAX_PLAINTEXT {
            return Err(Error::Limit);
        }
        if !matches!(header.len(), 1694 | 1702) {
            return Err(Error::InvalidStore);
        }
        return envelope::encode(&header[..1694], &packet).map_err(|_| Error::Limit);
    }
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM initiations WHERE session=?1)",
        [session.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::UnsupportedSession);
    }
    Ok(packet)
}

pub(super) fn accept_repeated(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    message: Id,
    packet: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let header = header(tx, key, &session)?.ok_or(Error::Unprepared)?;
    if header.len() != 1758 {
        return Err(Error::Conflict);
    }
    let slot = header[1694..1726]
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    let sender = header[1726..].try_into().map_err(|_| Error::InvalidStore)?;
    accept(tx, key, slot, session, message, sender, packet)
}

// Freeze the initiating lifetime with its header, not mutable queue ordering.
pub(super) fn prepare_expiry(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    requested: Option<u64>,
    now: u64,
) -> Result<u64, Error> {
    let mut header = header(tx, key, session)?.ok_or(Error::UnsupportedSession)?;
    let cap = match header.len() {
        1694 => {
            if tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM deliveries WHERE session=?1)",
                [session.as_slice()],
                |r| r.get::<_, bool>(0),
            )? {
                // Earlier records have no authenticated frozen lifetime.
                return Err(Error::UnsupportedSession);
            }
            None
        }
        1702 => {
            let cap =
                u64::from_be_bytes(header[1694..].try_into().map_err(|_| Error::InvalidStore)?);
            if cap == 0 || cap > i64::MAX as u64 {
                return Err(Error::InvalidStore);
            }
            Some(cap)
        }
        _ => return Err(Error::InvalidStore),
    };
    let expiry =
        requested.unwrap_or_else(|| now.saturating_add(604800).min(cap.unwrap_or(u64::MAX)));
    if expiry <= now
        || expiry > i64::MAX as u64
        || expiry > now.saturating_add(604800)
        || cap.is_some_and(|cap| expiry > cap)
    {
        return Err(Error::Expired);
    }
    if cap.is_none() {
        header.extend_from_slice(&expiry.to_be_bytes());
        let sealed = key.seal(&header, &binding(20, session, b"initial header"))?;
        tx.execute(
            "UPDATE initial_headers SET data=?1 WHERE session=?2",
            (sealed, session.as_slice()),
        )?;
    }
    Ok(expiry)
}

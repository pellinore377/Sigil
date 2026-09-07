//! Exact linking retries. Retained records prevent challenge/binding rebinding.
use super::*;
use crate::{binding, handshake};
use rusqlite::{Connection, OptionalExtension, Transaction};
use sigil_crypto::{storage::StorageKey, IdentityKey};
use zeroize::Zeroizing;

pub(super) fn reference(own: &Id, kind: u8, handle: &Id) -> Id {
    Sha256::digest(
        [
            b"Sigil/device-link-record/v0".as_slice(),
            own,
            &[kind],
            handle,
        ]
        .concat(),
    )
    .into()
}
pub(super) fn read(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    id: &Id,
) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    let sealed: Option<Vec<u8>> = db.query_row(
        "SELECT CASE WHEN length(state)<=32768 THEN state END FROM device_link_records WHERE id=?1",
        [id.as_slice()], |r| r.get(0)).optional()?;
    sealed
        .map(|bytes| key.open(&bytes, &binding(28, own, id)).map_err(Error::from))
        .transpose()
}
pub(super) fn save(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    id: &Id,
    bytes: &[u8],
    existing: bool,
) -> Result<(), Error> {
    if bytes.len() > 32732 {
        return Err(Error::Limit);
    }
    let sealed = key.seal(bytes, &binding(28, own, id))?;
    if existing {
        if db.execute(
            "UPDATE device_link_records SET state=?1 WHERE id=?2",
            (&sealed, id.as_slice()),
        )? != 1
        {
            return Err(Error::Conflict);
        }
    } else {
        db.execute(
            "INSERT INTO device_link_records VALUES(?1,?2)",
            (id.as_slice(), sealed),
        )?;
    }
    Ok(())
}
pub(super) fn joining_binding(
    tx: &Transaction<'_>,
    key: &StorageKey,
    identity: &IdentityKey,
    sponsor: Id,
    value: sigil_protocol::device::Binding,
) -> Result<Vec<u8>, Error> {
    let own = identity.public_key();
    let id = reference(&own, 2, &value.device);
    if let Some(bytes) = read(tx, key, &own, &id)? {
        if bytes.len() < 32 || bytes[..32] != sponsor {
            return Err(Error::Conflict);
        }
        let signed = peers::parse(&bytes[32..])?;
        if signed.binding != value {
            return Err(Error::Conflict);
        }
        return Ok(bytes[32..].to_vec());
    }
    let signature = identity.sign(&value.signing_bytes().map_err(|_| Error::InvalidEvent)?)?;
    let raw = SignedBinding {
        binding: value,
        signature,
    }
    .to_bytes()
    .map_err(|_| Error::InvalidEvent)?;
    save(
        tx,
        key,
        &own,
        &id,
        &[sponsor.as_slice(), &raw].concat(),
        false,
    )?;
    Ok(raw)
}
fn consent_id(own: &Id, transcript: &Transcript, sponsor: bool) -> Id {
    reference(
        own,
        u8::from(sponsor),
        if sponsor {
            &transcript.sponsor_challenge
        } else {
            &transcript.joining_challenge
        },
    )
}
fn decode(
    bytes: &[u8],
    own: &Id,
    transcript: &Transcript,
    sponsor: bool,
) -> Result<([u8; 64], bool), Error> {
    if bytes.len() != 281 || bytes[280] > 1 {
        return Err(Error::InvalidStore);
    }
    let saved = Transcript::from_bytes(&bytes[..216]).map_err(|_| Error::InvalidStore)?;
    let signature: [u8; 64] = bytes[216..280]
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    verify_signature(own, &signing_bytes(&saved, sponsor)?, &signature)?;
    if saved != *transcript {
        return Err(Error::Conflict);
    }
    Ok((signature, bytes[280] == 1))
}
pub(super) fn consent(
    tx: &Transaction<'_>,
    key: &StorageKey,
    identity: &IdentityKey,
    transcript: &Transcript,
    sponsor: bool,
) -> Result<[u8; 64], Error> {
    let own = identity.public_key();
    let id = consent_id(&own, transcript, sponsor);
    if let Some(bytes) = read(tx, key, &own, &id)? {
        let (signature, cancelled) = decode(&bytes, &own, transcript, sponsor)?;
        return if cancelled {
            Err(Error::Cancelled)
        } else {
            Ok(signature)
        };
    }
    let signature = identity.sign(&signing_bytes(transcript, sponsor)?)?;
    let mut bytes = transcript
        .to_bytes()
        .map_err(|_| Error::InvalidEvent)?
        .to_vec();
    bytes.extend_from_slice(&signature);
    bytes.push(0);
    save(tx, key, &own, &id, &bytes, false)?;
    Ok(signature)
}
pub(super) fn cancel_record(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    sponsor: bool,
    challenge: &Id,
    expected: Option<&Transcript>,
) -> Result<Option<bool>, Error> {
    let id = reference(own, u8::from(sponsor), challenge);
    let Some(mut bytes) = read(tx, key, own, &id)? else {
        return Ok(None);
    };
    let saved = Transcript::from_bytes(bytes.get(..216).ok_or(Error::InvalidStore)?)
        .map_err(|_| Error::InvalidStore)?;
    if consent_id(own, &saved, sponsor) != id || expected.is_some_and(|value| value != &saved) {
        return Err(Error::Conflict);
    }
    let (_, cancelled) = decode(&bytes, own, &saved, sponsor)?;
    if !cancelled {
        bytes[280] = 1;
        save(tx, key, own, &id, &bytes, true)?;
    }
    Ok(Some(!cancelled))
}
impl ClientStore {
    /// Stop local consent retries. This cannot retract an already transmitted
    /// signature or revoke a server grant; those require the provisioning protocol.
    pub fn cancel_device_link_consent(&mut self, transcript: &Transcript) -> Result<bool, Error> {
        transcript.to_bytes().map_err(|_| Error::InvalidEvent)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let own = handshake::identity(&tx, &self.key)?.public_key();
        for sponsor in [true, false] {
            let challenge = if sponsor {
                &transcript.sponsor_challenge
            } else {
                &transcript.joining_challenge
            };
            if let Some(changed) =
                cancel_record(&tx, &self.key, &own, sponsor, challenge, Some(transcript))?
            {
                tx.commit()?;
                return Ok(changed);
            }
        }
        Err(Error::NotFound)
    }
}

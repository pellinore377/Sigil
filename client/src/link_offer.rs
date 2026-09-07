//! Sealed pending joining offers; no provisioning messages are transmitted.
use super::*;
use crate::{binding, handshake};
use sigil_crypto::IdentityKey;
use sigil_protocol::{
    accounts::valid_credential,
    link::{Offer, MAX_LIFETIME},
};
use zeroize::Zeroizing;

pub(super) struct Pending {
    pub(super) offer: Offer,
    // None is an authenticated cancellation tombstone, not an absent offer.
    pub(super) secrets: Option<(IdentityKey, Zeroizing<String>)>,
}
fn random_id() -> Result<Id, Error> {
    let mut value = [0; 32];
    getrandom::fill(&mut value).map_err(|_| sigil_crypto::Error::Entropy)?;
    Ok(value)
}
fn decode(
    key: &sigil_crypto::storage::StorageKey,
    own: &Id,
    attempt: &Id,
    bytes: &[u8],
) -> Result<Pending, Error> {
    if bytes.len() != 185 && bytes.len() != 325 {
        return Err(Error::InvalidStore);
    }
    let offer = Offer::from_bytes(&bytes[..184]).map_err(|_| Error::InvalidStore)?;
    if offer.identity != *own {
        return Err(Error::Conflict);
    }
    let secrets = match (bytes[184], bytes.len()) {
        (1, 185) => None,
        (0, 325) => {
            let key =
                IdentityKey::open_checkpoint(key, &bytes[185..261], &binding(29, own, attempt))?;
            let credential = Zeroizing::new(
                std::str::from_utf8(&bytes[261..])
                    .map_err(|_| Error::InvalidStore)?
                    .to_owned(),
            );
            if !valid_credential(&credential)
                || key.public_key() != offer.provisioning_key
                || <Id>::from(Sha256::digest(credential.as_bytes())) != offer.credential_commitment
            {
                return Err(Error::InvalidStore);
            }
            Some((key, credential))
        }
        _ => return Err(Error::InvalidStore),
    };
    Ok(Pending { offer, secrets })
}
pub(super) fn live(pending: &Pending, now: u64) -> Result<(), Error> {
    if pending.secrets.is_none() {
        return Err(Error::Cancelled);
    }
    if now < pending.offer.created_at || now >= pending.offer.expires_at {
        return Err(Error::Expired);
    }
    Ok(())
}
impl ClientStore {
    /// Persist secrets before exposing the public offer. Retain the caller's fresh
    /// attempt ID across restart; retries cannot refresh its deadline or keys.
    pub fn prepare_device_link_offer(&mut self, attempt: Id, now: u64) -> Result<Offer, Error> {
        if attempt == [0; 32] || now == 0 {
            return Err(Error::InvalidEvent);
        }
        let own = self.identity()?;
        let id = journal::reference(&own, 3, &attempt);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_unconfigured(&tx)?;
        if let Some(bytes) = journal::read(&tx, &self.key, &own, &id)? {
            let pending = decode(&self.key, &own, &attempt, &bytes)?;
            live(&pending, now)?;
            tx.commit()?;
            return Ok(pending.offer);
        }
        let provisioning = IdentityKey::generate()?;
        let mut random = Zeroizing::new([0; 32]);
        getrandom::fill(random.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
        let credential = Zeroizing::new(crate::transport::hex(random.as_ref()));
        let offer = Offer {
            device: random_id()?,
            identity: own,
            challenge: random_id()?,
            provisioning_key: provisioning.public_key(),
            credential_commitment: Sha256::digest(credential.as_bytes()).into(),
            created_at: now,
            expires_at: now
                .checked_add(MAX_LIFETIME)
                .filter(|time| *time <= i64::MAX as u64)
                .ok_or(Error::InvalidEvent)?,
        };
        let mut bytes = Zeroizing::new(offer.to_bytes().map_err(|_| Error::InvalidEvent)?.to_vec());
        bytes.push(0);
        bytes.extend_from_slice(
            &provisioning.seal_checkpoint(&self.key, &binding(29, &own, &attempt))?,
        );
        bytes.extend_from_slice(credential.as_bytes());
        journal::save(&tx, &self.key, &own, &id, &bytes, false)?;
        tx.commit()?;
        Ok(offer)
    }
    /// Cancel local preparation and logically remove its provisioning key and
    /// credential. This does not retract a transmitted approval or server action.
    pub fn cancel_device_link_offer(&mut self, attempt: Id) -> Result<bool, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 3, &attempt);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let bytes = journal::read(&tx, &self.key, &own, &id)?.ok_or(Error::NotFound)?;
        let pending = decode(&self.key, &own, &attempt, &bytes)?;
        let active = pending.secrets.is_some();
        journal::cancel_record(&tx, &self.key, &own, false, &pending.offer.challenge, None)?;
        if active {
            let mut bytes = pending
                .offer
                .to_bytes()
                .map_err(|_| Error::InvalidStore)?
                .to_vec();
            bytes.push(1);
            journal::save(&tx, &self.key, &own, &id, &bytes, true)?;
        }
        tx.commit()?;
        Ok(active)
    }
    /// Consent must match this installation's saved offer. The caller must still
    /// independently verify the sponsor and full confirmation digest.
    pub fn approve_device_link_offer(
        &mut self,
        attempt: Id,
        transcript: &Transcript,
        sponsor: &[u8],
        expected_confirmation: Id,
        now: u64,
    ) -> Result<(Vec<u8>, [u8; 64]), Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 3, &attempt);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_unconfigured(&tx)?;
        let bytes = journal::read(&tx, &self.key, &own, &id)?.ok_or(Error::NotFound)?;
        let pending = decode(&self.key, &own, &attempt, &bytes)?;
        live(&pending, now)?;
        let offer = pending.offer;
        if transcript.joining_challenge != offer.challenge
            || transcript.provisioning_key != offer.provisioning_key
            || transcript.credential_commitment != offer.credential_commitment
            || transcript.created_at < offer.created_at
            || transcript.expires_at > offer.expires_at
        {
            return Err(Error::Conflict);
        }
        let mut value = peers::parse(sponsor)?.binding;
        let sponsor_fingerprint = peers::fingerprint(&value)?;
        value.device = offer.device;
        value.identity = own;
        let identity = handshake::identity(&tx, &self.key)?;
        let joining =
            journal::joining_binding(&tx, &self.key, &identity, sponsor_fingerprint, value)?;
        check(transcript, sponsor, &joining, now)?;
        if confirmation(transcript)? != expected_confirmation {
            return Err(Error::Conflict);
        }
        let signature = journal::consent(&tx, &self.key, &identity, transcript, false)?;
        tx.commit()?;
        Ok((joining, signature))
    }
}

#[cfg(test)]
#[path = "link_offer_tests.rs"]
mod tests;

pub(super) fn pending(
    db: &rusqlite::Connection,
    key: &sigil_crypto::storage::StorageKey,
    own: &Id,
    attempt: &Id,
) -> Result<Pending, Error> {
    let id = journal::reference(own, 3, attempt);
    let bytes = journal::read(db, key, own, &id)?.ok_or(Error::NotFound)?;
    decode(key, own, attempt, &bytes)
}
pub(super) fn matches(offer: &Offer, transcript: &Transcript) -> Result<(), Error> {
    if transcript.joining_challenge != offer.challenge
        || transcript.provisioning_key != offer.provisioning_key
        || transcript.credential_commitment != offer.credential_commitment
        || transcript.created_at < offer.created_at
        || transcript.expires_at > offer.expires_at
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub(super) fn erase(
    tx: &rusqlite::Transaction<'_>,
    key: &sigil_crypto::storage::StorageKey,
    own: &Id,
    attempt: &Id,
) -> Result<(), Error> {
    let pending = pending(tx, key, own, attempt)?;
    journal::cancel_record(tx, key, own, false, &pending.offer.challenge, None)?;
    let mut bytes = pending
        .offer
        .to_bytes()
        .map_err(|_| Error::InvalidStore)?
        .to_vec();
    bytes.push(1);
    journal::save(
        tx,
        key,
        own,
        &journal::reference(own, 3, attempt),
        &bytes,
        true,
    )
}

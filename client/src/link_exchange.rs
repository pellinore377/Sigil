//! Direct QR trust bootstrap. Only scan codes directly from the intended device.
use super::*;
use serde::{Deserialize, Serialize};
use sigil_crypto::IdentityKey;
use sigil_protocol::link::{Offer, Proof};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sponsor {
    #[serde(default)]
    cancelled: bool,
    offer: Vec<u8>,
    transcript: Vec<u8>,
    binding: Vec<u8>,
    key: Option<Vec<u8>>,
    proposal: Vec<u8>,
    proof: Option<Vec<u8>>,
    #[serde(default)]
    receipt: Option<sigil_protocol::accounts::Session>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Joining {
    proposal: Vec<u8>,
    transcript: Vec<u8>,
    sponsor: Vec<u8>,
    response: Option<Vec<u8>>,
    #[serde(default)]
    completed: bool,
}
fn encode(kind: &str, bytes: &[u8]) -> String {
    format!("sigil:link:v1:{kind}:{}", crate::transport::hex(bytes))
}
fn decode(kind: &str, text: &str, limit: usize) -> Result<Vec<u8>, Error> {
    let value = text
        .strip_prefix(&format!("sigil:link:v1:{kind}:"))
        .ok_or(Error::InvalidEvent)?;
    if value.len() > limit * 2 || !value.len().is_multiple_of(2) {
        return Err(Error::Limit);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        if !pair
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(Error::InvalidEvent);
        }
        bytes.push(
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| Error::InvalidEvent)?,
                16,
            )
            .map_err(|_| Error::InvalidEvent)?,
        );
    }
    Ok(bytes)
}
fn frame(
    key: &IdentityKey,
    peer: &Id,
    digest: &Id,
    role: u8,
    body: &[u8],
) -> Result<Vec<u8>, Error> {
    Ok([
        b"SGLF\0\x01\0\0".as_slice(),
        &key.public_key(),
        digest,
        &sigil_crypto::link::seal(key, peer, digest, role, body)?,
    ]
    .concat())
}
fn unframe(
    key: &IdentityKey,
    bytes: &[u8],
    role: u8,
) -> Result<(Id, Id, Zeroizing<Vec<u8>>), Error> {
    if !(108..=2156).contains(&bytes.len()) || &bytes[..8] != b"SGLF\0\x01\0\0" {
        return Err(Error::InvalidEvent);
    }
    let peer = bytes[8..40].try_into().map_err(|_| Error::InvalidEvent)?;
    let digest = bytes[40..72].try_into().map_err(|_| Error::InvalidEvent)?;
    Ok((
        peer,
        digest,
        sigil_crypto::link::open(key, &peer, &digest, role, &bytes[72..])?,
    ))
}
fn load<T: serde::de::DeserializeOwned>(
    db: &rusqlite::Connection,
    key: &sigil_crypto::storage::StorageKey,
    own: &Id,
    id: &Id,
) -> Result<Option<T>, Error> {
    journal::read(db, key, own, id)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore))
        .transpose()
}
fn save<T: Serialize>(
    db: &rusqlite::Connection,
    key: &sigil_crypto::storage::StorageKey,
    own: &Id,
    id: &Id,
    value: &T,
    existing: bool,
) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
    journal::save(db, key, own, id, &bytes, existing)
}
fn live(transcript: &Transcript, now: u64) -> Result<(), Error> {
    if now < transcript.created_at || now >= transcript.expires_at {
        return Err(Error::Expired);
    }
    Ok(())
}
/// Additional visual consistency check. The directly scanned QR frame is the
/// authentication channel; this short string alone must never establish trust.
pub fn emoji_confirmation(digest: Id) -> [&'static str; 8] {
    const ICONS: [&str; 64] = [
        "🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘", "🐰", "🐼", "🐓", "🐧", "🐢", "🐟", "🐙", "🦋",
        "🌷", "🌳", "🌵", "🍄", "🌏", "🌙", "☁️", "🔥", "🍌", "🍎", "🍓", "🌽", "🍕", "🎂", "❤️",
        "😀", "🤖", "🎩", "👓", "🔧", "🎅", "👍", "☂️", "⌛", "⏰", "🎁", "💡", "📕", "✏️", "📎",
        "✂️", "🔒", "🔑", "🔨", "☎️", "🏁", "🚂", "🚲", "✈️", "🚀", "🏆", "⚽", "🎸", "🎺", "🔔",
        "⚓", "🎧", "📁", "📌",
    ];
    let mut bits = 0_u64;
    for byte in &digest[..6] {
        bits = (bits << 8) | u64::from(*byte);
    }
    std::array::from_fn(|index| ICONS[((bits >> (42 - index * 6)) & 63) as usize])
}
pub fn offer_qr(offer: &Offer) -> Result<String, Error> {
    Ok(encode(
        "offer",
        &offer.to_bytes().map_err(|_| Error::InvalidEvent)?,
    ))
}

impl ClientStore {
    pub(crate) fn joining_link_approved(&mut self, attempt: Id) -> Result<bool, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 5, &attempt);
        Ok(load::<Joining>(&self.db, &self.key, &own, &id)?
            .is_some_and(|value| value.response.is_some()))
    }
    pub(crate) fn joining_link_address(&mut self, attempt: Id) -> Result<String, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 5, &attempt);
        let saved = load::<Joining>(&self.db, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        let sponsor = peers::parse(&saved.sponsor)?;
        Ok(format!(
            "@{}:{}",
            sponsor.binding.username, sponsor.binding.server
        ))
    }
    /// Scan only the QR displayed by the intended joining device.
    pub fn prepare_sponsored_link(
        &mut self,
        attempt: Id,
        offer_qr: &str,
        now: u64,
    ) -> Result<(String, Id), Error> {
        if attempt == [0; 32] {
            return Err(Error::InvalidEvent);
        }
        self.connected_client()?;
        let raw_offer = decode("offer", offer_qr, Offer::BYTES)?;
        let offer = Offer::from_bytes(&raw_offer).map_err(|_| Error::InvalidEvent)?;
        if now < offer.created_at || now >= offer.expires_at {
            return Err(Error::Expired);
        }
        let own_binding = self.own_device_binding()?;
        let sponsor = peers::parse(&own_binding)?;
        let own = sponsor.binding.identity;
        if offer.identity == own {
            return Err(Error::Conflict);
        }
        let id = journal::reference(&own, 4, &attempt);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(saved) = load::<Sponsor>(&tx, &self.key, &own, &id)? {
            if saved.key.is_none() {
                return Err(Error::Cancelled);
            }
            if saved.offer != raw_offer || saved.binding != own_binding {
                return Err(Error::Conflict);
            }
            let transcript =
                Transcript::from_bytes(&saved.transcript).map_err(|_| Error::InvalidStore)?;
            live(&transcript, now)?;
            return Ok((
                encode("proposal", &saved.proposal),
                confirmation(&transcript)?,
            ));
        }
        let provisioning = IdentityKey::generate()?;
        let mut challenge = [0; 32];
        getrandom::fill(&mut challenge).map_err(|_| sigil_crypto::Error::Entropy)?;
        let mut joining = sponsor.binding.clone();
        joining.device = offer.device;
        joining.identity = offer.identity;
        let transcript = Transcript {
            sponsor: peers::fingerprint(&sponsor.binding)?,
            joining: peers::fingerprint(&joining)?,
            sponsor_challenge: challenge,
            joining_challenge: offer.challenge,
            provisioning_key: offer.provisioning_key,
            credential_commitment: offer.credential_commitment,
            created_at: now,
            expires_at: offer.expires_at,
        };
        let digest = confirmation(&transcript)?;
        let proposal = frame(
            &provisioning,
            &offer.provisioning_key,
            &digest,
            0,
            &[
                transcript
                    .to_bytes()
                    .map_err(|_| Error::InvalidEvent)?
                    .as_slice(),
                &own_binding,
            ]
            .concat(),
        )?;
        let saved = Sponsor {
            cancelled: false,
            offer: raw_offer,
            transcript: transcript
                .to_bytes()
                .map_err(|_| Error::InvalidEvent)?
                .to_vec(),
            binding: own_binding,
            key: Some(
                provisioning.seal_checkpoint(&self.key, &crate::binding(32, &own, &attempt))?,
            ),
            proposal: proposal.clone(),
            proof: None,
            receipt: None,
        };
        save(&tx, &self.key, &own, &id, &saved, false)?;
        tx.commit()?;
        Ok((encode("proposal", &proposal), digest))
    }
    /// The proposal QR must be scanned directly from the intended sponsor. Do not
    /// call this on an unsolicited network message or link sent by another person.
    pub fn accept_link_proposal(&mut self, attempt: Id, qr: &str, now: u64) -> Result<Id, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 5, &attempt);
        let raw = decode("proposal", qr, 2156)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_unconfigured(&tx)?;
        let pending = offer::pending(&tx, &self.key, &own, &attempt)?;
        offer::live(&pending, now)?;
        if let Some(saved) = load::<Joining>(&tx, &self.key, &own, &id)? {
            if saved.proposal != raw {
                return Err(Error::Conflict);
            }
            return confirmation(
                &Transcript::from_bytes(&saved.transcript).map_err(|_| Error::InvalidStore)?,
            );
        }
        let key = &pending.secrets.as_ref().ok_or(Error::Cancelled)?.0;
        let (_, digest, body) = unframe(key, &raw, 0)?;
        let transcript = Transcript::from_bytes(body.get(..216).ok_or(Error::InvalidEvent)?)
            .map_err(|_| Error::InvalidEvent)?;
        live(&transcript, now)?;
        if confirmation(&transcript)? != digest {
            return Err(Error::Conflict);
        }
        let sponsor = body.get(216..).ok_or(Error::InvalidEvent)?;
        let parsed = peers::parse(sponsor)?;
        let mut value = parsed.binding.clone();
        value.device = pending.offer.device;
        value.identity = own;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        let joining = journal::joining_binding(
            &tx,
            &self.key,
            &identity,
            peers::fingerprint(&parsed.binding)?,
            value,
        )?;
        check(&transcript, sponsor, &joining, now)?;
        offer::matches(&pending.offer, &transcript)?;
        let saved = Joining {
            proposal: raw,
            transcript: transcript
                .to_bytes()
                .map_err(|_| Error::InvalidEvent)?
                .to_vec(),
            sponsor: sponsor.to_vec(),
            response: None,
            completed: false,
        };
        save(&tx, &self.key, &own, &id, &saved, false)?;
        tx.commit()?;
        Ok(digest)
    }
    pub fn confirm_link_proposal(
        &mut self,
        attempt: Id,
        expected: Id,
        now: u64,
    ) -> Result<String, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 5, &attempt);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_unconfigured(&tx)?;
        let pending = offer::pending(&tx, &self.key, &own, &attempt)?;
        offer::live(&pending, now)?;
        let mut saved = load::<Joining>(&tx, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        let transcript =
            Transcript::from_bytes(&saved.transcript).map_err(|_| Error::InvalidStore)?;
        live(&transcript, now)?;
        if confirmation(&transcript)? != expected {
            return Err(Error::Conflict);
        }
        if let Some(response) = saved.response {
            return Ok(encode("response", &response));
        }
        let sponsor = peers::parse(&saved.sponsor)?;
        let mut value = sponsor.binding.clone();
        value.device = pending.offer.device;
        value.identity = own;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        let joining = journal::joining_binding(
            &tx,
            &self.key,
            &identity,
            peers::fingerprint(&sponsor.binding)?,
            value,
        )?;
        check(&transcript, &saved.sponsor, &joining, now)?;
        offer::matches(&pending.offer, &transcript)?;
        let signature = journal::consent(&tx, &self.key, &identity, &transcript, false)?;
        let key = &pending.secrets.as_ref().ok_or(Error::Cancelled)?.0;
        let (peer, _, _) = unframe(key, &saved.proposal, 0)?;
        let response = frame(
            key,
            &peer,
            &expected,
            1,
            &[joining.as_slice(), &signature].concat(),
        )?;
        saved.response = Some(response.clone());
        save(&tx, &self.key, &own, &id, &saved, true)?;
        tx.commit()?;
        Ok(encode("response", &response))
    }
    pub fn confirm_sponsored_link(
        &mut self,
        attempt: Id,
        qr: &str,
        expected: Id,
        now: u64,
    ) -> Result<Proof, Error> {
        self.connected_client()?;
        let own_binding = self.own_device_binding()?;
        let own = peers::parse(&own_binding)?.binding.identity;
        let id = journal::reference(&own, 4, &attempt);
        let raw = decode("response", qr, 2156)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut saved = load::<Sponsor>(&tx, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        let transcript =
            Transcript::from_bytes(&saved.transcript).map_err(|_| Error::InvalidStore)?;
        live(&transcript, now)?;
        if saved.binding != own_binding || confirmation(&transcript)? != expected {
            return Err(Error::Conflict);
        }
        let key = IdentityKey::open_checkpoint(
            &self.key,
            saved.key.as_ref().ok_or(Error::Cancelled)?,
            &crate::binding(32, &own, &attempt),
        )?;
        let (peer, digest, body) = unframe(&key, &raw, 1)?;
        if peer != transcript.provisioning_key || digest != expected || body.len() < 64 {
            return Err(Error::Conflict);
        }
        let (joining, signature) = body.split_at(body.len() - 64);
        let (sponsor, joining) = check(&transcript, &saved.binding, joining, now)?;
        let joining_signature = signature.try_into().map_err(|_| Error::InvalidEvent)?;
        verify_signature(
            &joining.binding.identity,
            &signing_bytes(&transcript, false)?,
            signature,
        )?;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        let sponsor_signature = journal::consent(&tx, &self.key, &identity, &transcript, true)?;
        let proof = Proof {
            transcript,
            sponsor,
            joining,
            sponsor_signature,
            joining_signature,
        };
        sigil_crypto::link::verify(&proof, expected_sponsor(&proof)?, now)?;
        let bytes = proof.to_bytes().map_err(|_| Error::InvalidEvent)?;
        if saved.proof.as_ref().is_some_and(|old| old != &bytes) {
            return Err(Error::Conflict);
        }
        saved.proof = Some(bytes);
        save(&tx, &self.key, &own, &id, &saved, true)?;
        tx.commit()?;
        Ok(proof)
    }
}
/// An unused installation may discard its offer before signing consent. Once
/// consent exists, retain the credential to resolve possible server acceptance.
pub(crate) fn discard_unapproved_offer(
    tx: &rusqlite::Transaction<'_>,
    key: &sigil_crypto::storage::StorageKey,
    attempt: Id,
) -> Result<(), Error> {
    require_unconfigured(tx)?;
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions) OR EXISTS(SELECT 1 FROM prekeys) OR EXISTS(SELECT 1 FROM peers) OR EXISTS(SELECT 1 FROM own_device_binding)", [], |r| r.get::<_, bool>(0))? { return Err(Error::Conflict); }
    let own = crate::handshake::identity(tx, key)?.public_key();
    let offer_id = journal::reference(&own, 3, &attempt);
    let joining_id = journal::reference(&own, 5, &attempt);
    if load::<Joining>(tx, key, &own, &joining_id)?.is_some_and(|value| value.response.is_some()) {
        return Err(Error::Conflict);
    }
    let pending = match offer::pending(tx, key, &own, &attempt) {
        Ok(value) => Some(value),
        Err(Error::NotFound) => None,
        Err(error) => return Err(error),
    };
    let binding_id = pending
        .as_ref()
        .map(|value| journal::reference(&own, 2, &value.offer.device));
    let ids = tx
        .prepare("SELECT id FROM device_link_records LIMIT 4")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if ids.iter().any(|id| {
        id.as_slice() != offer_id
            && id.as_slice() != joining_id
            && binding_id
                .as_ref()
                .is_none_or(|binding| id.as_slice() != binding)
    }) {
        return Err(Error::Conflict);
    }
    tx.execute("DELETE FROM device_link_records", [])?;
    tx.execute("DELETE FROM identity", [])?;
    Ok(())
}
fn expected_sponsor(proof: &Proof) -> Result<Id, Error> {
    Ok(sigil_crypto::link::fingerprint(&proof.sponsor.binding)?)
}

impl ClientStore {
    pub fn authorize_sponsored_link_online(
        &mut self,
        attempt: Id,
    ) -> Result<sigil_protocol::accounts::Session, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 4, &attempt);
        let saved = load::<Sponsor>(&self.db, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        if saved.cancelled {
            return Err(Error::Cancelled);
        }
        if let Some(receipt) = saved.receipt {
            return Ok(receipt);
        }
        if saved.key.is_none() {
            return Err(Error::Cancelled);
        }
        let proof = Proof::from_bytes(saved.proof.as_ref().ok_or(Error::Unprepared)?)
            .map_err(|_| Error::InvalidStore)?;
        self.publish_device_binding_online()?;
        let receipt = self.connected_client()?.authorize_device_link(&proof)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut current = load::<Sponsor>(&tx, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        if current.proof != saved.proof || current.key.is_none() || current.cancelled {
            return Err(Error::Conflict);
        }
        peers::link_trust(&tx, &self.key, proof.joining, proof.transcript.joining)?;
        current.receipt = Some(receipt.clone());
        current.key = None;
        save(&tx, &self.key, &own, &id, &current, true)?;
        tx.commit()?;
        Ok(receipt)
    }
    /// Recover the already authorized server session with the exact locally
    /// prepared credential. No sponsor or ratchet private key is transferred.
    pub fn finish_device_link_online(
        &mut self,
        attempt: Id,
        port: u16,
        roots: &[Vec<u8>],
    ) -> Result<sigil_protocol::accounts::Session, Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 5, &attempt);
        let saved = load::<Joining>(&self.db, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        if saved.completed {
            return self.connection_session()?.ok_or(Error::InvalidStore);
        }
        if saved.response.is_none() {
            return Err(Error::Unprepared);
        }
        let pending = offer::pending(&self.db, &self.key, &own, &attempt)?;
        let credential = &pending.secrets.as_ref().ok_or(Error::Cancelled)?.1;
        let sponsor = peers::parse(&saved.sponsor)?;
        let client = crate::network::HttpsClient::discover(
            &sponsor.binding.server,
            port,
            credential,
            roots,
        )?;
        let session = client.session()?;
        let proof = client.own_device_link()?;
        if proof
            .transcript
            .to_bytes()
            .map_err(|_| Error::InvalidStore)?
            .as_slice()
            != saved.transcript
            || proof.sponsor != sponsor
            || proof.joining.binding.identity != own
            || proof.joining.binding.device != pending.offer.device
            || session.device_id != crate::transport::hex(&pending.offer.device)
            || session.account_id != crate::transport::hex(&proof.joining.binding.account)
            || session.address
                != format!(
                    "@{}:{}",
                    proof.joining.binding.username, proof.joining.binding.server
                )
            || session.device_label != "Linked device"
        {
            return Err(Error::Conflict);
        }
        sigil_crypto::link::verify(
            &proof,
            peers::fingerprint(&sponsor.binding)?,
            proof.transcript.created_at,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut current = load::<Joining>(&tx, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        if current.proposal != saved.proposal
            || current.response != saved.response
            || current.completed
        {
            return Err(Error::Conflict);
        }
        let pending = offer::pending(&tx, &self.key, &own, &attempt)?;
        let (_, credential) = pending.secrets.ok_or(Error::Cancelled)?;
        crate::connection::install_linked_connection(
            &tx,
            &self.key,
            session.clone(),
            port,
            roots,
            credential,
        )?;
        let binding = crate::binding(14, &proof.joining.binding.device, &own);
        tx.execute(
            "INSERT INTO own_device_binding VALUES(1,?1)",
            [self.key.seal(
                &proof.joining.to_bytes().map_err(|_| Error::InvalidStore)?,
                &binding,
            )?],
        )?;
        peers::link_trust(&tx, &self.key, proof.sponsor, proof.transcript.sponsor)?;
        offer::erase(&tx, &self.key, &own, &attempt)?;
        current.completed = true;
        save(&tx, &self.key, &own, &id, &current, true)?;
        tx.commit()?;
        Ok(session)
    }
}

#[cfg(test)]
#[path = "link_exchange_tests.rs"]
mod tests;

impl ClientStore {
    /// Durable server cancellation closes the race with authorization and revokes
    /// a child already created by that challenge. Retry after ambiguous failures.
    pub fn cancel_sponsored_link_online(&mut self, attempt: Id) -> Result<(), Error> {
        let own = self.identity()?;
        let id = journal::reference(&own, 4, &attempt);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut saved = load::<Sponsor>(&tx, &self.key, &own, &id)?.ok_or(Error::Unprepared)?;
        let transcript =
            Transcript::from_bytes(&saved.transcript).map_err(|_| Error::InvalidStore)?;
        journal::cancel_record(
            &tx,
            &self.key,
            &own,
            true,
            &transcript.sponsor_challenge,
            Some(&transcript),
        )?;
        saved.cancelled = true;
        saved.key = None;
        save(&tx, &self.key, &own, &id, &saved, true)?;
        if let Some(bytes) = saved.proof {
            let proof = Proof::from_bytes(&bytes).map_err(|_| Error::InvalidStore)?;
            let peer =
                peers::reference(&proof.joining.binding.server, &proof.joining.binding.device);
            match peers::block(&tx, &self.key, &peer, true) {
                Ok(()) | Err(Error::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        tx.commit()?;
        self.connected_client()?
            .cancel_device_link(&transcript.sponsor_challenge)?;
        Ok(())
    }
}

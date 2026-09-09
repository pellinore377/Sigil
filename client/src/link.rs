//! Durable direct-QR linking, consent and one-use server authorization.
use crate::{peers, ClientStore, Error, Id};
use sha2::{Digest, Sha256};
use sigil_crypto::verify_signature;
use sigil_protocol::{device::SignedBinding, link::Transcript};
#[path = "link_exchange.rs"]
mod exchange;
#[path = "link_journal.rs"]
mod journal;
#[path = "link_offer.rs"]
mod offer;
pub(crate) use exchange::discard_unapproved_offer;
pub use exchange::{emoji_confirmation, offer_qr};

/// Full confirmation digest. The emoji string is supplementary; authentication
/// requires the complete direct QR exchange, not a truncated digest alone.
pub fn confirmation(transcript: &Transcript) -> Result<Id, Error> {
    Ok(sigil_crypto::link::confirmation(transcript)?)
}
fn signing_bytes(transcript: &Transcript, sponsor: bool) -> Result<Vec<u8>, Error> {
    Ok(sigil_crypto::link::signing_bytes(transcript, sponsor)?)
}
fn check(
    transcript: &Transcript,
    sponsor: &[u8],
    joining: &[u8],
    now: u64,
) -> Result<(SignedBinding, SignedBinding), Error> {
    transcript.to_bytes().map_err(|_| Error::InvalidEvent)?;
    if now < transcript.created_at || now >= transcript.expires_at {
        return Err(Error::Expired);
    }
    Ok(sigil_crypto::link::context(transcript, sponsor, joining)?)
}

/// Check both role-separated consents against an independently trusted sponsor.
/// This does not check server authorization, spent challenges or device revocation.
pub fn verify_consents(
    transcript: &Transcript,
    sponsor: &[u8],
    joining: &[u8],
    sponsor_signature: &[u8; 64],
    joining_signature: &[u8; 64],
    trusted_sponsor: Id,
    now: u64,
) -> Result<Id, Error> {
    if transcript.sponsor != trusted_sponsor {
        return Err(Error::Conflict);
    }
    let (sponsor, joining) = check(transcript, sponsor, joining, now)?;
    verify_signature(
        &sponsor.binding.identity,
        &signing_bytes(transcript, true)?,
        sponsor_signature,
    )?;
    verify_signature(
        &joining.binding.identity,
        &signing_bytes(transcript, false)?,
        joining_signature,
    )?;
    confirmation(transcript)
}
impl ClientStore {
    /// Sign the prospective joining binding with this unconfigured installation's
    /// own key. Persists exact bytes before returning; retries must reuse the
    /// chosen device ID. This does not enroll or trust either device.
    pub fn sign_joining_device_binding(
        &mut self,
        sponsor: &[u8],
        device: Id,
    ) -> Result<Vec<u8>, Error> {
        let mut binding = peers::parse(sponsor)?.binding;
        let sponsor_fingerprint = peers::fingerprint(&binding)?;
        if device == [0; 32] || device == binding.device {
            return Err(Error::Conflict);
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_unconfigured(&tx)?;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        if binding.identity == identity.public_key() {
            return Err(Error::Conflict);
        }
        binding.device = device;
        binding.identity = identity.public_key();
        let bytes =
            journal::joining_binding(&tx, &self.key, &identity, sponsor_fingerprint, binding)?;
        tx.commit()?;
        Ok(bytes)
    }
    /// Explicit consent after independently checking the complete transcript.
    /// The joining binding may be prospective (not yet enrolled). This signs only
    /// with this installation's independent identity; it grants no server access.
    pub fn sign_device_link_consent(
        &mut self,
        transcript: &Transcript,
        sponsor: &[u8],
        joining: &[u8],
        expected_confirmation: Id,
        now: u64,
    ) -> Result<[u8; 64], Error> {
        let (sponsor, joining) = check(transcript, sponsor, joining, now)?;
        if confirmation(transcript)? != expected_confirmation {
            return Err(Error::Conflict);
        }
        let public_key = self.identity()?;
        let is_sponsor = public_key == sponsor.binding.identity;
        if is_sponsor {
            self.connected_client()?;
            if self
                .connection_session()?
                .ok_or(Error::Unprepared)?
                .expires_at
                <= now
            {
                return Err(Error::Expired);
            }
            let own = peers::parse(&self.own_device_binding()?)?;
            if own.binding != sponsor.binding {
                return Err(Error::Conflict);
            }
        } else if public_key != joining.binding.identity {
            return Err(Error::Conflict);
        } else {
            require_unconfigured(&self.db)?;
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        if !is_sponsor {
            require_unconfigured(&tx)?;
        }
        if identity.public_key() != public_key {
            return Err(Error::Conflict);
        }
        let signature = journal::consent(&tx, &self.key, &identity, transcript, is_sponsor)?;
        tx.commit()?;
        Ok(signature)
    }
}

fn require_unconfigured(db: &rusqlite::Connection) -> Result<(), Error> {
    if db.query_row("SELECT EXISTS(SELECT 1 FROM connection)", [], |row| {
        row.get::<_, bool>(0)
    })? {
        return Err(Error::Conflict);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::tests::{prepare, setup};
    use sigil_crypto::{storage::StorageKey, DhKey, Secret32};
    fn open(path: &std::path::Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn linking_journal_migration_failed_commits_substitution_and_capacity_fail_closed() {
        let (dir, fixture, invitation, now) = setup();
        let mut sponsor = open(&dir.path().join("sponsor.db"));
        prepare(&mut sponsor, &fixture, &invitation.secret);
        sponsor.enroll_online().unwrap();
        let sponsor_bytes = sponsor.own_device_binding().unwrap();
        let path = dir.path().join("joining.db");
        let mut joining = open(&path);
        let identity = joining.identity().unwrap();
        crate::test_schema::rewind(&joining.db, 34);
        drop(joining);
        let mut joining = open(&path);
        assert_eq!(joining.identity().unwrap(), identity);
        let count = |store: &ClientStore| {
            store
                .db
                .query_row("SELECT count(*) FROM device_link_records", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        };
        assert_eq!(count(&joining), 0);
        joining.db.execute_batch("CREATE TRIGGER fail_link BEFORE INSERT ON device_link_records BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
        assert!(joining
            .sign_joining_device_binding(&sponsor_bytes, [3; 32])
            .is_err());
        assert_eq!(count(&joining), 0);
        joining.db.execute_batch("DROP TRIGGER fail_link;").unwrap();
        let joining_bytes = joining
            .sign_joining_device_binding(&sponsor_bytes, [3; 32])
            .unwrap();
        let binding_state: Vec<u8> = joining
            .db
            .query_row("SELECT state FROM device_link_records", [], |r| r.get(0))
            .unwrap();
        let transcript = Transcript {
            sponsor: peers::device_fingerprint(&sponsor_bytes).unwrap(),
            joining: peers::device_fingerprint(&joining_bytes).unwrap(),
            sponsor_challenge: [4; 32],
            joining_challenge: [5; 32],
            provisioning_key: DhKey::generate().unwrap().public_key(),
            credential_commitment: [6; 32],
            created_at: now,
            expires_at: now + 600,
        };
        let digest = confirmation(&transcript).unwrap();
        joining.db.execute_batch("CREATE TRIGGER fail_link BEFORE INSERT ON device_link_records BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
        assert!(joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .is_err());
        assert_eq!(count(&joining), 1);
        joining.db.execute_batch("DROP TRIGGER fail_link;").unwrap();
        let signature = joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .unwrap();
        let (id, state): (Vec<u8>, Vec<u8>) = joining
            .db
            .query_row(
                "SELECT id,state FROM device_link_records WHERE length(state)=317",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        joining
            .db
            .execute(
                "UPDATE device_link_records SET state=?1 WHERE id=?2",
                (&binding_state, &id),
            )
            .unwrap();
        assert!(joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .is_err());
        assert!(joining.cancel_device_link_consent(&transcript).is_err());
        joining
            .db
            .execute(
                "UPDATE device_link_records SET state=?1 WHERE id=?2",
                (&state, &id),
            )
            .unwrap();
        // Synthetic occupied slots cross the previous lifetime bound. They are
        // never interpreted as valid consent records or used to grant authority.
        for n in 0..254_u32 {
            joining
                .db
                .execute(
                    "INSERT INTO device_link_records VALUES(?1,zeroblob(1))",
                    [n.to_be_bytes().as_slice()],
                )
                .unwrap();
        }
        assert_eq!(count(&joining), 256);
        assert_eq!(
            joining
                .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
                .unwrap(),
            signature
        );
        let mut other = transcript.clone();
        other.joining_challenge = [7; 32];
        joining
            .sign_device_link_consent(
                &other,
                &sponsor_bytes,
                &joining_bytes,
                confirmation(&other).unwrap(),
                now,
            )
            .unwrap();
        assert!(joining.cancel_device_link_consent(&transcript).unwrap());
        assert_eq!(count(&joining), 257);
    }
    #[test]
    fn both_devices_consent_to_the_same_context_and_every_transcript_byte_is_bound() {
        let (dir, fixture, invitation, now) = setup();
        let mut sponsor = open(&dir.path().join("sponsor.db"));
        prepare(&mut sponsor, &fixture, &invitation.secret);
        sponsor.enroll_online().unwrap();
        let sponsor_bytes = sponsor.own_device_binding().unwrap();
        let mut joining = open(&dir.path().join("joining.db"));
        let joining_bytes = joining
            .sign_joining_device_binding(&sponsor_bytes, [3; 32])
            .unwrap();
        let tx = joining.db.transaction().unwrap();
        let key = crate::handshake::identity(&tx, &joining.key).unwrap();
        tx.commit().unwrap();
        assert!(sponsor
            .sign_joining_device_binding(&sponsor_bytes, [7; 32])
            .is_err());
        let transcript = Transcript {
            sponsor: peers::device_fingerprint(&sponsor_bytes).unwrap(),
            joining: peers::device_fingerprint(&joining_bytes).unwrap(),
            sponsor_challenge: [4; 32],
            joining_challenge: [5; 32],
            provisioning_key: DhKey::generate().unwrap().public_key(),
            credential_commitment: [6; 32],
            created_at: now,
            expires_at: now + 600,
        };
        let digest = confirmation(&transcript).unwrap();
        let a = sponsor
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .unwrap();
        let b = joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .unwrap();
        assert_eq!(
            verify_consents(
                &transcript,
                &sponsor_bytes,
                &joining_bytes,
                &a,
                &b,
                transcript.sponsor,
                now
            )
            .unwrap(),
            digest
        );
        assert!(verify_consents(
            &transcript,
            &sponsor_bytes,
            &joining_bytes,
            &b,
            &a,
            transcript.sponsor,
            now
        )
        .is_err());
        assert!(verify_consents(
            &transcript,
            &sponsor_bytes,
            &joining_bytes,
            &a,
            &b,
            [9; 32],
            now
        )
        .is_err());
        for n in 0..sigil_protocol::link::TRANSCRIPT_BYTES {
            let mut bytes = transcript.to_bytes().unwrap();
            bytes[n] ^= 1;
            if let Ok(changed) = Transcript::from_bytes(&bytes) {
                assert!(
                    verify_consents(
                        &changed,
                        &sponsor_bytes,
                        &joining_bytes,
                        &a,
                        &b,
                        transcript.sponsor,
                        now
                    )
                    .is_err(),
                    "byte {n}"
                );
            }
        }
        assert!(sponsor
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, [0; 32], now)
            .is_err());
        for clock in [now - 1, transcript.expires_at] {
            assert!(verify_consents(
                &transcript,
                &sponsor_bytes,
                &joining_bytes,
                &a,
                &b,
                transcript.sponsor,
                clock
            )
            .is_err());
            assert!(joining
                .sign_device_link_consent(
                    &transcript,
                    &sponsor_bytes,
                    &joining_bytes,
                    digest,
                    clock
                )
                .is_err());
        }
        // No local peer trust is created by consent, and independent installation
        // keys survive restart without copying a sponsor identity or ratchet.
        assert_eq!(
            sponsor
                .db
                .query_row("SELECT count(*) FROM peers", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(joining);
        let mut joining = open(&dir.path().join("joining.db"));
        let again = joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .unwrap();
        assert_eq!(again, b);
        assert_eq!(
            joining
                .sign_joining_device_binding(&sponsor_bytes, [3; 32])
                .unwrap(),
            joining_bytes
        );
        let mut rebound = transcript.clone();
        rebound.expires_at -= 1;
        assert!(matches!(
            joining.sign_device_link_consent(
                &rebound,
                &sponsor_bytes,
                &joining_bytes,
                confirmation(&rebound).unwrap(),
                now
            ),
            Err(Error::Conflict)
        ));
        joining.db.execute_batch("CREATE TRIGGER fail_cancel BEFORE UPDATE ON device_link_records BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
        assert!(joining.cancel_device_link_consent(&transcript).is_err());
        joining
            .db
            .execute_batch("DROP TRIGGER fail_cancel;")
            .unwrap();
        assert_eq!(
            joining
                .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
                .unwrap(),
            b
        );
        assert!(joining.cancel_device_link_consent(&transcript).unwrap());
        assert!(!joining.cancel_device_link_consent(&transcript).unwrap());
        drop(joining);
        let mut joining = open(&dir.path().join("joining.db"));
        assert!(matches!(
            joining.sign_device_link_consent(
                &transcript,
                &sponsor_bytes,
                &joining_bytes,
                digest,
                now
            ),
            Err(Error::Cancelled)
        ));
        verify_consents(
            &transcript,
            &sponsor_bytes,
            &joining_bytes,
            &a,
            &again,
            transcript.sponsor,
            now,
        )
        .unwrap();
        assert!(joining.connection_session().is_err());
        joining
            .db
            .execute("INSERT INTO connection VALUES(1,zeroblob(5000))", [])
            .unwrap();
        assert!(joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .is_err());
        assert!(joining
            .sign_joining_device_binding(&sponsor_bytes, [7; 32])
            .is_err());
        joining.db.execute("DELETE FROM connection", []).unwrap();
        // A freshly signed binding for another account cannot join this sponsor.
        let mut other = peers::parse(&joining_bytes).unwrap();
        other.binding.account = [99; 32];
        other.signature = key.sign(&other.binding.signing_bytes().unwrap()).unwrap();
        let other_bytes = other.to_bytes().unwrap();
        let mut altered = transcript;
        altered.joining = peers::device_fingerprint(&other_bytes).unwrap();
        assert!(joining
            .sign_device_link_consent(
                &altered,
                &sponsor_bytes,
                &other_bytes,
                confirmation(&altered).unwrap(),
                now
            )
            .is_err());
    }
}

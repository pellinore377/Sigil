//! Issuing keys rotate weekly; the current and previous week are accepted.
//! Two purposes: `credential` (one per name per week) and `token` (daily
//! batches against a credential). Spent nonces are remembered per key id.

use crate::store::{key2, today, Store, CREDS, KEYS, SPENT};
use redb::ReadableTable;
use sigil_protocol::token::{Issuer, Token, Verifier};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct TokenService {
    issuers: Mutex<HashMap<(String, u32), std::sync::Arc<Issuer>>>,
}

fn week() -> u32 {
    today() / 7
}

impl TokenService {
    pub fn new() -> Self {
        TokenService {
            issuers: Mutex::new(HashMap::new()),
        }
    }

    /// The issuer for `purpose` in `week`, created and persisted on first use.
    pub fn issuer(
        &self,
        store: &Store,
        purpose: &str,
        week: u32,
    ) -> anyhow::Result<std::sync::Arc<Issuer>> {
        if let Some(i) = self
            .issuers
            .lock()
            .unwrap()
            .get(&(purpose.to_string(), week))
        {
            return Ok(i.clone());
        }
        let key = key2(purpose.as_bytes(), &week.to_be_bytes());
        let existing = {
            let r = store.db.begin_read()?;
            r.open_table(KEYS)?
                .get(key.as_slice())?
                .map(|v| v.value().to_vec())
        };
        let issuer = match existing {
            Some(der) => Issuer::from_der(&der).map_err(|e| anyhow::anyhow!("{e:?}"))?,
            None => {
                let mut rng = rand::rngs::OsRng;
                let i = Issuer::generate(&mut RngAdapter(&mut rng));
                let w = store.db.begin_write()?;
                w.open_table(KEYS)?
                    .insert(key.as_slice(), i.to_der().as_slice())?;
                w.commit()?;
                i
            }
        };
        let arc = std::sync::Arc::new(issuer);
        self.issuers
            .lock()
            .unwrap()
            .insert((purpose.to_string(), week), arc.clone());
        Ok(arc)
    }

    pub fn current(&self, store: &Store, purpose: &str) -> anyhow::Result<std::sync::Arc<Issuer>> {
        self.issuer(store, purpose, week())
    }

    /// Find the verifier for a token's key id among the accepted weeks.
    fn verifier_for(
        &self,
        store: &Store,
        purpose: &str,
        key_id: &[u8; 32],
    ) -> anyhow::Result<Option<Verifier>> {
        for wk in [week(), week().saturating_sub(1)] {
            let i = self.issuer(store, purpose, wk)?;
            if &i.key_id == key_id {
                return Ok(Some(
                    Verifier::from_spki(&i.spki).map_err(|e| anyhow::anyhow!("{e:?}"))?,
                ));
            }
        }
        Ok(None)
    }

    /// Verify and spend. Returns the status to answer with on failure.
    pub fn spend(
        &self,
        store: &Store,
        purpose: &str,
        token_bytes: &[u8],
    ) -> anyhow::Result<Result<(), sigil_protocol::wire::Status>> {
        use sigil_protocol::wire::Status;
        let tok = match Token::decode(token_bytes) {
            Ok(t) => t,
            Err(_) => return Ok(Err(Status::TokenInvalid)),
        };
        let Some(v) = self.verifier_for(store, purpose, &tok.key_id)? else {
            return Ok(Err(Status::TokenInvalid));
        };
        if v.verify(&tok).is_err() {
            return Ok(Err(Status::TokenInvalid));
        }
        let key = key2(&tok.key_id, &tok.spend_id());
        let w = store.db.begin_write()?;
        {
            let mut t = w.open_table(SPENT)?;
            if t.get(key.as_slice())?.is_some() {
                return Ok(Err(Status::TokenSpent));
            }
            t.insert(key.as_slice(), ())?;
        }
        w.commit()?;
        Ok(Ok(()))
    }

    /// One credential per identity per week.
    pub fn credential_once(&self, store: &Store, identity_pub: &[u8; 32]) -> anyhow::Result<bool> {
        let key = key2(identity_pub, &week().to_be_bytes());
        let w = store.db.begin_write()?;
        {
            let mut t = w.open_table(CREDS)?;
            if t.get(key.as_slice())?.is_some() {
                return Ok(false);
            }
            t.insert(key.as_slice(), ())?;
        }
        w.commit()?;
        Ok(true)
    }

    /// Daily quota per credential: counted under the credential's spend id
    /// and today's day number. Returns false when exhausted.
    pub fn quota(
        &self,
        store: &Store,
        credential: &Token,
        n: u32,
        per_day: u32,
    ) -> anyhow::Result<bool> {
        let key = key2(
            b"quota",
            &key2(&credential.spend_id(), &today().to_be_bytes()),
        );
        let w = store.db.begin_write()?;
        let ok = {
            let mut t = w.open_table(KEYS)?;
            let used = t
                .get(key.as_slice())?
                .map(|v| u32::from_le_bytes(v.value().try_into().unwrap_or([0; 4])))
                .unwrap_or(0);
            if used + n > per_day {
                false
            } else {
                t.insert(key.as_slice(), (used + n).to_le_bytes().as_slice())?;
                true
            }
        };
        w.commit()?;
        Ok(ok)
    }
}

/// rand 0.8's OsRng behind the rand_core 0.10 traits blind-rsa wants.
pub struct RngAdapter<'a>(pub &'a mut rand::rngs::OsRng);
impl rand_core10::TryRng for RngAdapter<'_> {
    type Error = core::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(rand::RngCore::next_u32(self.0))
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(rand::RngCore::next_u64(self.0))
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        rand::RngCore::fill_bytes(self.0, dst);
        Ok(())
    }
}
impl rand_core10::TryCryptoRng for RngAdapter<'_> {}

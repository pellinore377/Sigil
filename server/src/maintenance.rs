use crate::{
    enrollment::now,
    store::{Store, StoreError},
    with_store, AppState,
};
use rusqlite::TransactionBehavior;
use std::time::Duration;

pub(crate) const MIGRATION: &str = "
CREATE INDEX prekeys_expiry ON prekeys(expires_at,id) WHERE bundle IS NOT NULL;
CREATE INDEX invitations_expiry ON invitations(expires_at,id);
";
const BATCH: usize = 64;

impl Store {
    /// Clears at most 64 expired payloads/reservations per table in one transaction.
    /// Retry identifiers, assignments and contact-invitation tombstones are retained.
    pub fn expire_batch(&mut self, now: u64) -> Result<usize, StoreError> {
        if now > i64::MAX as u64 {
            return Err(StoreError::InvalidData);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expired:Vec<i64>=tx.prepare("SELECT sequence FROM mailbox WHERE payload IS NOT NULL AND expires_at<=?1 ORDER BY expires_at,sequence LIMIT ?2")?.query_map((now as i64,BATCH as i64),|r|r.get(0))?.collect::<Result<_,_>>()?;
        for sequence in &expired {
            crate::federation_mailbox::release_payload(&tx, *sequence)?;
        }
        let mut changed = expired.len();
        changed += tx.execute("UPDATE recovery_objects SET data=NULL WHERE rowid IN (SELECT r.rowid FROM recovery_objects r JOIN deleted_accounts d ON d.account=r.account_id WHERE r.data IS NOT NULL LIMIT 64)", [])?;
        changed += tx.execute(
            "DELETE FROM web_sessions WHERE expires<=?1 OR touched<=?2",
            (now as i64, now.saturating_sub(1800) as i64),
        )?;
        changed += tx.execute("DELETE FROM web_oidc WHERE expires<=?1", [now as i64])?;
        // Release only the bytes physically removed in this same bounded batch.
        let released: Vec<(String, i64)> = tx.prepare("SELECT d.account_id,sum(length(p.bundle)) FROM prekeys p JOIN devices d ON d.id=p.device_id WHERE p.id IN (SELECT id FROM prekeys WHERE bundle IS NOT NULL AND expires_at<=?1 ORDER BY expires_at,id LIMIT ?2) GROUP BY d.account_id")?.query_map((now as i64,BATCH as i64), |r|Ok((r.get(0)?,r.get(1)?)))?.collect::<Result<_,_>>()?;
        for (account, bytes) in released {
            if tx.execute(
                "UPDATE retained_storage SET bytes=bytes-?2 WHERE account_id=?1 AND bytes>=?2",
                (&account, bytes),
            )? != 1
            {
                return Err(StoreError::InvalidData);
            }
        }
        changed+=tx.execute("UPDATE prekeys SET bundle=NULL WHERE id IN (SELECT id FROM prekeys WHERE bundle IS NOT NULL AND expires_at<=?1 ORDER BY expires_at,id LIMIT ?2)",(now as i64,BATCH as i64))?;
        changed+=tx.execute("DELETE FROM invitations WHERE id IN (SELECT id FROM invitations WHERE expires_at<=?1 ORDER BY expires_at,id LIMIT ?2)",(now as i64,BATCH as i64))?;
        changed+=tx.execute("DELETE FROM oidc_flows WHERE id IN (SELECT id FROM oidc_flows WHERE expires<=?1 LIMIT 64)",[now as i64])?;
        changed+=tx.execute("DELETE FROM oidc_grants WHERE token_hash IN (SELECT token_hash FROM oidc_grants WHERE expires<=?1 LIMIT 64)",[now as i64])?;
        changed += crate::attachments::cleanup(&tx, now)?;
        changed += crate::push_delivery::cleanup(&tx, now)?;
        changed += crate::federation_admission::cleanup(&tx, now)?;
        changed += crate::federation_mailbox::cleanup(&tx)?;
        changed += crate::federation_outbox::cleanup(&tx, now)?;
        tx.commit()?;
        Ok(changed)
    }
}

pub(crate) async fn run(state: AppState) {
    let period = Duration::from_secs(1);
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut failed = false;
    loop {
        timer.tick().await;
        match with_store(state.clone(), |store| store.expire_batch(now()?)).await {
            Ok(_) => failed = false,
            Err(StoreError::Busy) => {}
            Err(_) => {
                if !failed {
                    eprintln!("Storage maintenance failed; retrying in background.");
                }
                failed = true;
            }
        }
    }
}

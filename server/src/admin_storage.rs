use crate::{
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
pub(crate) const MIGRATION: &str = "CREATE TABLE deleted_accounts(account TEXT PRIMARY KEY REFERENCES accounts(id)); ALTER TABLE private_groups ADD COLUMN blocked INTEGER NOT NULL DEFAULT 0 CHECK(blocked IN(0,1));";
#[derive(Serialize)]
pub(crate) struct Group {
    id: String,
    revision: u64,
    blocked: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Delete {
    pub expected_revision: u64,
    pub confirm: bool,
}
impl Store {
    pub(crate) fn admin_groups(&self, after: &str) -> Result<Vec<Group>, StoreError> {
        if !after.is_empty() && crate::federation_auth::bytes32(after).is_err() {
            return Err(StoreError::Invalid("Invalid group cursor"));
        }
        Ok(self.0.prepare("SELECT lower(hex(id)),revision,blocked FROM private_groups WHERE lower(hex(id))>?1 ORDER BY id LIMIT 100")?.query_map([after], |r|Ok(Group{id:r.get(0)?,revision:unsigned(r,1)?,blocked:r.get(2)?}))?.collect::<Result<_,_>>()?)
    }
    pub(crate) fn admin_delete_group(
        &mut self,
        id: &str,
        request: Delete,
    ) -> Result<(), StoreError> {
        if !request.confirm {
            return Err(StoreError::Invalid(
                "Confirm deletion of the encrypted group record",
            ));
        }
        let group = crate::federation_auth::bytes32(id)
            .map_err(|_| StoreError::Invalid("Invalid group reference"))?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, blocked): (u64, bool) = tx.query_row(
            "SELECT revision,blocked FROM private_groups WHERE id=?1",
            [group.as_slice()],
            |r| Ok((unsigned(r, 0)?, r.get(1)?)),
        )?;
        if revision != request.expected_revision {
            return Err(StoreError::Conflict);
        }
        if blocked {
            return Ok(());
        }
        let mut released = 0i64;
        for query in ["SELECT count(*)*512 FROM private_group_members WHERE group_id=?1", "SELECT coalesce(sum(512+length(control)),0) FROM private_group_commits WHERE group_id=?1", "SELECT coalesce(sum(512+length(control)),0) FROM private_group_proposals WHERE group_id=?1", "SELECT count(*)*384 FROM private_group_invitations WHERE group_id=?1"] { released += tx.query_row(query,[group.as_slice()],|r|r.get::<_,i64>(0))?; }
        if tx.execute(
            "UPDATE group_authority SET used=used-?1 WHERE used>=?1",
            [released],
        )? != 1
        {
            return Err(StoreError::InvalidData);
        }
        for table in [
            "private_group_members",
            "private_group_commits",
            "private_group_proposals",
            "private_group_invitations",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE group_id=?1"),
                [group.as_slice()],
            )?;
        }
        tx.execute(
            "UPDATE private_groups SET blocked=1 WHERE id=?1",
            [group.as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn admin_delete_account(
        &mut self,
        id: &str,
        request: Delete,
        now: u64,
    ) -> Result<(), StoreError> {
        if !request.confirm || !sigil_protocol::accounts::valid_credential(id) {
            return Err(StoreError::Invalid("Confirm account deletion"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM web_owner WHERE account=?1)",
            [id],
            |r| r.get(0),
        )?;
        if owner {
            return Err(StoreError::Forbidden);
        }
        crate::admin::retain_administrator(&tx, id)?;
        let revision: u64 = tx.query_row(
            "SELECT coalesce((SELECT revision FROM account_policy WHERE account=?1),0)",
            [id],
            |r| unsigned(r, 0),
        )?;
        if revision != request.expected_revision {
            return Err(StoreError::Conflict);
        }
        if tx.execute("UPDATE accounts SET disabled=1 WHERE id=?1", [id])? != 1 {
            return Err(StoreError::NotFound);
        }
        tx.execute("INSERT OR IGNORE INTO deleted_accounts VALUES(?1)", [id])?;
        tx.execute(
            "UPDATE devices SET revoked=1,token_hash=NULL WHERE account_id=?1",
            [id],
        )?;
        tx.execute("UPDATE prekeys SET expires_at=?2 WHERE device_id IN (SELECT id FROM devices WHERE account_id=?1)",(id,sql(now)?))?;
        tx.execute("UPDATE mailbox SET expires_at=?2 WHERE sender IN (SELECT id FROM devices WHERE account_id=?1) OR recipient IN (SELECT id FROM devices WHERE account_id=?1)",(id,sql(now)?))?;
        tx.execute("UPDATE attachments SET state=2,reserved_bytes=0,restored_checkpoint=0 WHERE account_id=?1",[id])?;
        tx.execute("DELETE FROM oidc_bindings WHERE account=?1", [id])?;
        tx.execute("DELETE FROM account_profiles WHERE account=?1", [id])?;
        tx.execute("DELETE FROM account_passwords WHERE account=?1", [id])?;
        tx.execute("DELETE FROM oidc_fallback_ack WHERE account=?1", [id])?;
        tx.execute(
            "DELETE FROM invitations WHERE username=(SELECT username FROM accounts WHERE id=?1)",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

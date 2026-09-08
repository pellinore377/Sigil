use crate::{
    prekeys::authorize,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sigil_protocol::{accounts::valid_credential, admin::*};

pub(crate) const MIGRATION: &str = "
CREATE TABLE admin_policy(id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);
INSERT INTO admin_policy VALUES(1,'{\"revision\":0,\"registration\":\"invitations\",\"max_accounts\":10000,\"registrations_per_day\":100,\"public_origin\":null}');
CREATE TABLE account_policy(account TEXT PRIMARY KEY REFERENCES accounts(id),revision INTEGER NOT NULL DEFAULT 0,role TEXT NOT NULL DEFAULT '\"member\"',discoverable INTEGER NOT NULL DEFAULT 1,quota INTEGER);
CREATE TABLE registration_usage(day INTEGER PRIMARY KEY,count INTEGER NOT NULL CHECK(count>=0));
";
pub(crate) fn policy(db: &Connection) -> Result<Policy, StoreError> {
    let value: String = db.query_row("SELECT value FROM admin_policy WHERE id=1", [], |r| {
        r.get(0)
    })?;
    serde_json::from_str(&value).map_err(|_| StoreError::InvalidData)
}
pub(crate) fn quota(db: &Connection, account: &str) -> Result<u64, StoreError> {
    let value: Option<i64> = db
        .query_row(
            "SELECT quota FROM account_policy WHERE account=?1",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    match value {
        Some(n) => u64::try_from(n).map_err(|_| StoreError::InvalidData),
        None => Ok(crate::store::read_configuration(db)?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .default_quota_bytes),
    }
}
pub(crate) fn role(db: &Connection, account: &str) -> Result<Role, StoreError> {
    let value: Option<String> = db
        .query_row(
            "SELECT role FROM account_policy WHERE account=?1",
            [account],
            |r| r.get(0),
        )
        .optional()?;
    value
        .map(|v| serde_json::from_str(&v).map_err(|_| StoreError::InvalidData))
        .unwrap_or(Ok(Role::Member))
}
pub(crate) fn retain_administrator(db: &Connection, id: &str) -> Result<(), StoreError> {
    if role(db, id)? == Role::Administrator
        && db
            .query_row("SELECT disabled=0 FROM accounts WHERE id=?1", [id], |r| {
                r.get::<_, bool>(0)
            })
            .optional()?
            .unwrap_or(false)
    {
        let others:u32=db.query_row("SELECT count(*) FROM accounts a JOIN account_policy p ON p.account=a.id WHERE a.disabled=0 AND p.role='\"administrator\"' AND a.id!=?1",[id],|r|r.get(0))?;
        if others == 0 {
            return Err(StoreError::Invalid(
                "retain an active administrator account",
            ));
        }
    }
    Ok(())
}
pub(crate) fn register(db: &Connection, now: u64, oidc: bool) -> Result<(), StoreError> {
    let policy = policy(db)?;
    if policy.registration == Registration::Closed
        || (oidc && policy.registration != Registration::Oidc)
    {
        return Err(StoreError::Forbidden);
    }
    let total: u32 = db.query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))?;
    if total >= policy.max_accounts {
        return Err(StoreError::Busy);
    }
    let day = sql(now / 86400)?;
    if db.query_row(
        "SELECT coalesce(max(day),0) FROM registration_usage",
        [],
        |r| r.get::<_, i64>(0),
    )? > day
    {
        return Err(StoreError::Busy);
    }
    db.execute("DELETE FROM registration_usage WHERE day<?1", [day])?;
    let count: u32 = db.query_row(
        "SELECT coalesce(max(count),0) FROM registration_usage WHERE day=?1",
        [day],
        |r| r.get(0),
    )?;
    if count >= policy.registrations_per_day {
        return Err(StoreError::Busy);
    }
    db.execute(
        "INSERT INTO registration_usage VALUES(?1,1) ON CONFLICT(day) DO UPDATE SET count=count+1",
        [day],
    )?;
    Ok(())
}
pub(crate) fn allowed(role: Role, method: &axum::http::Method, path: &str) -> bool {
    if role == Role::Administrator {
        return true;
    }
    if role == Role::Member {
        return false;
    }
    if *method == axum::http::Method::GET {
        return !path.starts_with("/admin/v0/maintenance/files/");
    }
    role == Role::Operator
        && (path == "/admin/v0/invitations" || path.starts_with("/admin/v0/invitations/"))
}
impl Store {
    pub fn admin_invitations(
        &self,
        after: Option<&str>,
        now: u64,
    ) -> Result<serde_json::Value, StoreError> {
        if after.is_some_and(|v| !valid_credential(v)) {
            return Err(StoreError::Invalid("invalid cursor"));
        }
        let mut rows=self.0.prepare("SELECT id,username,expires_at FROM invitations WHERE id>?1 AND expires_at>?2 ORDER BY id LIMIT 65")?.query_map((after.unwrap_or(""),sql(now)?),|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"expires_at":unsigned(r,2)?})))?.collect::<Result<Vec<_>,_>>()?;
        let after = if rows.len() > 64 {
            rows.pop();
            rows.last().map(|r| r["id"].clone())
        } else {
            None
        };
        Ok(serde_json::json!({"invitations":rows,"next_after":after}))
    }
    pub fn admin_devices(
        &self,
        id: &str,
        after: Option<&str>,
    ) -> Result<serde_json::Value, StoreError> {
        if !valid_credential(id) || after.is_some_and(|v| !valid_credential(v)) {
            return Err(StoreError::Invalid("invalid account or cursor"));
        }
        let mut rows=self.0.prepare("SELECT id,label,expires_at,revoked FROM devices WHERE account_id=?1 AND id>?2 ORDER BY id LIMIT 65")?.query_map((id,after.unwrap_or("")),|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"label":r.get::<_,String>(1)?,"expires_at":unsigned(r,2)?,"revoked":r.get::<_,bool>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
        let after = if rows.len() > 64 {
            rows.pop();
            rows.last().map(|r| r["id"].clone())
        } else {
            None
        };
        Ok(serde_json::json!({"account":id,"devices":rows,"next_after":after}))
    }
    pub fn admin_revoke_device(&mut self, id: &str, device: &str) -> Result<(), StoreError> {
        if self.0.execute(
            "UPDATE devices SET revoked=1,token_hash=NULL WHERE id=?1 AND account_id=?2",
            (device, id),
        )? != 1
        {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
    pub fn admin_diagnostics(&mut self, now: u64) -> Result<serde_json::Value, StoreError> {
        let maintenance = self.operation_diagnostics()?;
        let tx = self.0.transaction()?;
        let scalar = |query: &str| -> Result<u64, StoreError> {
            Ok(tx.query_row(query, [], |r| unsigned(r, 0))?)
        };
        let pages = scalar("PRAGMA page_count")?;
        let page_size = scalar("PRAGMA page_size")?;
        Ok(serde_json::json!({
            "as_of":now, "version":env!("CARGO_PKG_VERSION"), "schema":crate::store::SCHEMA_VERSION,
            "database_bytes":pages.saturating_mul(page_size),
            "accounts":scalar("SELECT count(*) FROM accounts WHERE disabled=0")?,
            "devices":scalar("SELECT count(*) FROM devices WHERE revoked=0")?,
            "retained_bytes":scalar("SELECT coalesce(sum(bytes),0) FROM retained_storage")?,
            "mailbox_pending":scalar("SELECT count(*) FROM mailbox WHERE payload IS NOT NULL")?,
            "push_pending":scalar("SELECT count(*) FROM push_jobs")?,
            "push_retries":scalar("SELECT coalesce(sum(attempts),0) FROM push_jobs")?,
            "push_invalid":scalar("SELECT count(*) FROM push_channels WHERE state=3")?,
            "federation_pending":scalar("SELECT count(*) FROM federation_outbox WHERE body IS NOT NULL")?,
            "federation_failed_peers":scalar("SELECT count(*) FROM federation_peers WHERE error IS NOT NULL")?,
            "attachment_bytes":scalar("SELECT coalesce(sum(length(data)),0) FROM attachment_chunks")?,
            "recovery_bytes":scalar("SELECT coalesce(sum(length(data)),0) FROM recovery_objects")?,
            "maintenance":maintenance, "redacted":true
        }))
    }
    pub fn admin_role(&self, credential: &str, now: u64) -> Result<Role, StoreError> {
        let session = self.session(credential, now)?;
        role(&self.0, &session.account_id)
    }
    pub fn administration_policy(&self) -> Result<Policy, StoreError> {
        policy(&self.0)
    }
    pub fn configure_administration(&mut self, mut update: Policy) -> Result<Policy, StoreError> {
        if !(1..=1000000).contains(&update.max_accounts)
            || !(1..=10000).contains(&update.registrations_per_day)
        {
            return Err(StoreError::Invalid("invalid registration limits"));
        }
        if let Some(origin) = &mut update.public_origin {
            origin_url(origin)?;
            *origin = openidconnect::url::Url::parse(origin)
                .map_err(|_| StoreError::Invalid("invalid HTTPS origin"))?
                .origin()
                .ascii_serialization();
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = policy(&tx)?;
        if old.revision != update.revision {
            return Err(StoreError::Conflict);
        }
        if old.public_origin != update.public_origin
            && tx.query_row(
                "SELECT value IS NOT NULL FROM oidc_configuration WHERE id=1",
                [],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Err(StoreError::Invalid(
                "disable OIDC before changing its redirect origin",
            ));
        }
        update.revision = crate::push_config::next(old.revision)?;
        tx.execute(
            "UPDATE admin_policy SET value=?1 WHERE id=1",
            [serde_json::to_string(&update).map_err(|_| StoreError::InvalidData)?],
        )?;
        tx.commit()?;
        Ok(update)
    }
    pub fn admin_accounts(
        &mut self,
        after: Option<&str>,
        now: u64,
    ) -> Result<AccountPage, StoreError> {
        if after.is_some_and(|id| !valid_credential(id)) {
            return Err(StoreError::Invalid("invalid account cursor"));
        }
        let tx = self.0.transaction()?;
        let ids: Vec<String> = tx
            .prepare("SELECT id FROM accounts WHERE id>?1 ORDER BY id LIMIT 65")?
            .query_map([after.unwrap_or("")], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let next_after = (ids.len() > 64).then(|| ids[63].clone());
        let accounts = ids
            .iter()
            .take(64)
            .map(|id| account(&tx, id, now))
            .collect::<Result<_, _>>()?;
        Ok(AccountPage {
            accounts,
            next_after,
        })
    }
    pub fn admin_update_account(
        &mut self,
        id: &str,
        update: AccountUpdate,
        now: u64,
    ) -> Result<Account, StoreError> {
        if !update.confirm
            || !valid_credential(id)
            || update
                .quota_bytes
                .is_some_and(|q| !(1024 * 1024..=1024u64.pow(5)).contains(&q))
        {
            return Err(StoreError::Invalid(
                "confirm the account access and quota change",
            ));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = account(&tx, id, now)?;
        if old.revision != update.expected_revision {
            return Err(StoreError::Conflict);
        }
        if old.role == Role::Administrator
            && !old.disabled
            && (update.disabled || update.role != Role::Administrator)
        {
            retain_administrator(&tx, id)?;
        }
        let revision = crate::push_config::next(old.revision)?;
        tx.execute("INSERT INTO account_policy(account,revision,role,quota) VALUES(?1,?2,?3,?4) ON CONFLICT(account) DO UPDATE SET revision=excluded.revision,role=excluded.role,quota=excluded.quota", (id,sql(revision)?,serde_json::to_string(&update.role).map_err(|_| StoreError::InvalidData)?,update.quota_bytes.map(sql).transpose()?))?;
        tx.execute(
            "UPDATE accounts SET disabled=?2 WHERE id=?1",
            (id, update.disabled),
        )?;
        if update.disabled {
            tx.execute(
                "UPDATE devices SET revoked=1,token_hash=NULL WHERE account_id=?1",
                [id],
            )?;
        }
        let result = account(&tx, id, now)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn discovery_preference(
        &mut self,
        credential: &str,
        update: Option<DiscoveryPreference>,
        now: u64,
    ) -> Result<DiscoveryPreference, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let account: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [device],
            |r| r.get(0),
        )?;
        let mut value = preference(&tx, &account)?;
        if let Some(update) = update {
            if update.revision != value.revision {
                return Err(StoreError::Conflict);
            }
            value = DiscoveryPreference {
                revision: crate::push_config::next(value.revision)?,
                discoverable: update.discoverable,
            };
            tx.execute("INSERT INTO account_policy(account,revision,discoverable) VALUES(?1,?2,?3) ON CONFLICT(account) DO UPDATE SET revision=excluded.revision,discoverable=excluded.discoverable", (&account,sql(value.revision)?,value.discoverable))?;
        }
        tx.commit()?;
        Ok(value)
    }
    pub fn discover_account(
        &mut self,
        credential: &str,
        username: &str,
        now: u64,
    ) -> Result<FoundAccount, StoreError> {
        let tx = self.0.transaction()?;
        authorize(&tx, credential, now)?;
        discover(&tx, username, now)
    }
}
fn preference(db: &Connection, id: &str) -> Result<DiscoveryPreference, StoreError> {
    Ok(db
        .query_row(
            "SELECT revision,discoverable FROM account_policy WHERE account=?1",
            [id],
            |r| {
                Ok(DiscoveryPreference {
                    revision: unsigned(r, 0)?,
                    discoverable: r.get(1)?,
                })
            },
        )
        .optional()?
        .unwrap_or(DiscoveryPreference {
            revision: 0,
            discoverable: true,
        }))
}
fn account(db: &Connection, id: &str, now: u64) -> Result<Account, StoreError> {
    let (username, disabled): (String, bool) = db
        .query_row(
            "SELECT username,disabled FROM accounts WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    let preference = preference(db, id)?;
    Ok(Account {
        id: id.into(),
        username,
        disabled,
        revision: preference.revision,
        discoverable: preference.discoverable,
        role: role(db, id)?,
        quota_bytes: quota(db, id)?,
        used_bytes: crate::recovery::used(db, id, now)?,
    })
}
pub(crate) fn discover(
    db: &Connection,
    username: &str,
    now: u64,
) -> Result<FoundAccount, StoreError> {
    if !sigil_protocol::accounts::valid_username(username) {
        return Err(StoreError::Invalid("provide an exact username"));
    }
    let account: String = db.query_row("SELECT a.id FROM accounts a LEFT JOIN account_policy p ON p.account=a.id WHERE a.username=?1 AND a.disabled=0 AND coalesce(p.discoverable,1)=1", [username], |r| r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
    let devices: Vec<String> = db.prepare("SELECT id FROM devices WHERE account_id=?1 AND revoked=0 AND expires_at>?2 ORDER BY id LIMIT 65")?.query_map((&account,sql(now)?), |r| r.get(0))?.collect::<Result<_,_>>()?;
    if devices.len() > 64 {
        return Err(StoreError::Busy);
    }
    let server = crate::store::read_configuration(db)?
        .settings
        .ok_or(StoreError::Unauthorized)?
        .server_name;
    Ok(FoundAccount {
        address: format!("@{username}:{server}"),
        account,
        devices,
    })
}
pub(crate) fn origin_url(origin: &str) -> Result<(), StoreError> {
    let uri =
        crate::egress::endpoint(origin).map_err(|_| StoreError::Invalid("invalid HTTPS origin"))?;
    if uri.path_and_query().is_some_and(|p| p.as_str() != "/") || origin.ends_with('/') {
        return Err(StoreError::Invalid(
            "origin must not have a path or trailing slash",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "admin_tests.rs"]
pub(crate) mod tests;

use crate::{
    oidc,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_protocol::oidc::{Access, AcknowledgeFallback};

pub(crate) const MIGRATION: &str = "
CREATE TABLE oidc_transition(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,retiring INTEGER NOT NULL);
INSERT INTO oidc_transition VALUES(1,0,0);
CREATE TABLE oidc_fallback_ack(account TEXT PRIMARY KEY REFERENCES accounts(id),revision INTEGER NOT NULL);
";
#[derive(Serialize)]
pub struct PendingAccount {
    pub id: String,
    pub username: String,
    pub active_devices: u32,
}
#[derive(Serialize)]
pub struct Transition {
    pub configuration_revision: u64,
    pub revision: u64,
    pub retiring: bool,
    pub linked_accounts: u64,
    pub awaiting_acknowledgement: u64,
    pub pending: Vec<PendingAccount>,
    pub next_after: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub configuration_revision: u64,
    pub revision: u64,
    pub retiring: bool,
    pub confirm: bool,
}
pub(crate) fn state(db: &Connection) -> Result<(u64, bool), StoreError> {
    Ok(
        db.query_row("SELECT revision,retiring FROM oidc_transition", [], |r| {
            Ok((unsigned(r, 0)?, r.get(1)?))
        })?,
    )
}
pub(crate) fn reset(db: &Connection) -> Result<(), StoreError> {
    let revision = crate::push_config::next(state(db)?.0)?;
    db.execute(
        "UPDATE oidc_transition SET revision=?1,retiring=0",
        [sql(revision)?],
    )?;
    db.execute("DELETE FROM oidc_fallback_ack", [])?;
    Ok(())
}
fn counts(db: &Connection, issuer: &str, revision: u64) -> Result<(u64, u64), StoreError> {
    Ok(db.query_row("SELECT count(*),coalesce(sum(CASE WHEN f.revision=?2 THEN 0 ELSE 1 END),0) FROM oidc_bindings b JOIN accounts a ON a.id=b.account LEFT JOIN oidc_fallback_ack f ON f.account=a.id WHERE b.issuer=?1 AND a.disabled=0 AND a.id NOT IN (SELECT account FROM web_owner WHERE account IS NOT NULL)",
        (issuer,sql(revision)?), |r| Ok((unsigned(r,0)?,unsigned(r,1)?)))?)
}
pub(crate) fn guard_retirement(db: &Connection, issuer: &str) -> Result<(), StoreError> {
    let (revision, retiring) = state(db)?;
    let (linked, pending) = counts(db, issuer, revision)?;
    if linked != 0 && (!retiring || pending != 0) {
        return Err(StoreError::Invalid("Prepare OIDC retirement and have affected users acknowledge administrator-assisted account access first"));
    }
    Ok(())
}
fn access(db: &Connection, account: &str) -> Result<Access, StoreError> {
    let (configuration_revision, configured) = oidc::read(db)?;
    let (transition_revision, retiring) = state(db)?;
    let issuer = configured.map(|v| v.provider.issuer);
    let linked: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM oidc_bindings WHERE account=?1 AND issuer=?2)",
        (account, issuer.as_deref()),
        |r| r.get(0),
    )?;
    let acknowledged: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM oidc_fallback_ack WHERE account=?1 AND revision=?2)",
        (account, sql(transition_revision)?),
        |r| r.get(0),
    )?;
    Ok(Access {
        configuration_revision,
        transition_revision,
        issuer,
        linked,
        retiring,
        invitation_fallback_acknowledged: linked && retiring && acknowledged,
    })
}
impl Store {
    pub fn oidc_access(&self, token: &str, now: u64) -> Result<Access, StoreError> {
        access(&self.0, &self.session(token, now)?.account_id)
    }
    pub fn acknowledge_oidc_fallback(
        &mut self,
        token: &str,
        request: AcknowledgeFallback,
        now: u64,
    ) -> Result<Access, StoreError> {
        let account = self.session(token, now)?.account_id;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = access(&tx, &account)?;
        if !request.confirm_invitation_fallback || !current.linked || !current.retiring {
            return Err(StoreError::Forbidden);
        }
        if request.configuration_revision != current.configuration_revision
            || request.transition_revision != current.transition_revision
        {
            return Err(StoreError::Conflict);
        }
        tx.execute("INSERT INTO oidc_fallback_ack VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET revision=excluded.revision", (&account,sql(current.transition_revision)?))?;
        tx.commit()?;
        self.oidc_access(token, now)
    }
    pub fn oidc_transition(&self, after: Option<&str>, now: u64) -> Result<Transition, StoreError> {
        if after.is_some_and(|v| !sigil_protocol::accounts::valid_credential(v)) {
            return Err(StoreError::InvalidData);
        }
        let (configuration_revision, configured) = oidc::read(&self.0)?;
        let issuer = configured.map(|v| v.provider.issuer).unwrap_or_default();
        let (revision, retiring) = state(&self.0)?;
        let (linked_accounts, awaiting_acknowledgement) = counts(&self.0, &issuer, revision)?;
        let mut pending = self.0.prepare("SELECT a.id,a.username,(SELECT count(*) FROM devices d WHERE d.account_id=a.id AND d.revoked=0 AND d.expires_at>?4) FROM oidc_bindings b JOIN accounts a ON a.id=b.account LEFT JOIN oidc_fallback_ack f ON f.account=a.id WHERE b.issuer=?1 AND a.disabled=0 AND a.id NOT IN (SELECT account FROM web_owner WHERE account IS NOT NULL) AND (f.revision IS NULL OR f.revision!=?2) AND a.id>?3 ORDER BY a.id LIMIT 51")?
            .query_map((&issuer,sql(revision)?,after.unwrap_or(""),sql(now)?), |r|Ok(PendingAccount { id:r.get(0)?,username:r.get(1)?,active_devices:r.get(2)? }))?.collect::<Result<Vec<_>,_>>()?;
        let next_after = if pending.len() > 50 {
            pending.pop();
            pending.last().map(|a| a.id.clone())
        } else {
            None
        };
        Ok(Transition {
            configuration_revision,
            revision,
            retiring,
            linked_accounts,
            awaiting_acknowledgement,
            pending,
            next_after,
        })
    }
    pub fn prepare_oidc_retirement(
        &mut self,
        request: Prepare,
        now: u64,
    ) -> Result<Transition, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (config, stored) = oidc::read(&tx)?;
        if !request.confirm || stored.is_none() {
            return Err(StoreError::Forbidden);
        }
        if config != request.configuration_revision || state(&tx)?.0 != request.revision {
            return Err(StoreError::Conflict);
        }
        let password: bool =
            tx.query_row("SELECT password_login FROM web_owner", [], |r| r.get(0))?;
        if !password {
            return Err(StoreError::Invalid(
                "Enable administrator password login before preparing OIDC retirement",
            ));
        }
        reset(&tx)?;
        tx.execute("UPDATE oidc_transition SET retiring=?1", [request.retiring])?;
        tx.execute("DELETE FROM web_oidc", [])?;
        tx.execute("DELETE FROM oidc_flows", [])?;
        tx.execute("DELETE FROM oidc_grants", [])?;
        tx.commit()?;
        self.oidc_transition(None, now)
    }
}

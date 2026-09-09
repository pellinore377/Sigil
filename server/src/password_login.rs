use crate::{
    auth::{digest, random_secret},
    push_config::sql,
    store::{Store, StoreError},
    web_admin::{hash_password, verify_password},
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use sigil_protocol::{
    accounts::{valid_credential, valid_username, Session},
    login::{Methods, PasswordPolicy},
};
use zeroize::Zeroizing;

pub(crate) const MIGRATION: &str = "
CREATE TABLE password_policy(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,enabled INTEGER NOT NULL,login_after INTEGER NOT NULL);
INSERT INTO password_policy VALUES(1,0,0,0);
CREATE TABLE account_passwords(account TEXT PRIMARY KEY REFERENCES accounts(id),hash TEXT NOT NULL);
ALTER TABLE oidc_grants ADD COLUMN replace_devices INTEGER NOT NULL DEFAULT 1;
";
#[cfg(test)]
#[path = "password_login_tests.rs"]
mod tests;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub username: String,
    pub password: Zeroizing<String>,
    pub device_credential: String,
    pub device_label: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPassword {
    pub password: Zeroizing<String>,
}

impl Store {
    pub fn login_methods(&self) -> Result<Methods, StoreError> {
        Ok(Methods {
            server_name: self
                .configuration()?
                .settings
                .ok_or(StoreError::NotFound)?
                .server_name,
            sso: self.oidc_configuration()?.enabled,
            password: self.user_password_policy()?.enabled,
            invitation: self.administration_policy()?.registration
                == sigil_protocol::admin::Registration::Invitations,
        })
    }
    pub fn user_password_policy(&self) -> Result<PasswordPolicy, StoreError> {
        Ok(self
            .0
            .query_row("SELECT revision,enabled FROM password_policy", [], |r| {
                Ok(PasswordPolicy {
                    revision: crate::push_config::unsigned(r, 0)?,
                    enabled: r.get(1)?,
                })
            })?)
    }
    pub fn configure_user_passwords(
        &mut self,
        value: PasswordPolicy,
    ) -> Result<PasswordPolicy, StoreError> {
        if self.0.execute(
            "UPDATE password_policy SET revision=revision+1,enabled=?1 WHERE revision=?2",
            (value.enabled, sql(value.revision)?),
        )? != 1
        {
            return Err(StoreError::Conflict);
        }
        self.user_password_policy()
    }
    pub fn set_user_password(
        &mut self,
        account: &str,
        value: SetPassword,
    ) -> Result<(), StoreError> {
        let exists: bool = self.0.query_row("SELECT EXISTS(SELECT 1 FROM accounts a WHERE a.id=?1 AND a.disabled=0 AND NOT EXISTS(SELECT 1 FROM deleted_accounts d WHERE d.account=a.id))", [account], |r|r.get(0))?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        let hash = hash_password(&value.password)?;
        self.0.execute("INSERT INTO account_passwords VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET hash=excluded.hash", (account, hash))?;
        Ok(())
    }
    pub fn password_sign_in(&mut self, value: Login, now: u64) -> Result<Session, StoreError> {
        if !valid_username(&value.username)
            || value.password.len() > 1024
            || !valid_credential(&value.device_credential)
            || value.device_label.is_empty()
            || value.device_label.len() > 80
            || value.device_label.chars().any(char::is_control)
        {
            return Err(StoreError::Unauthorized);
        }
        // Bound total password hashing work, including unknown account attempts.
        if self.0.execute(
            "UPDATE password_policy SET login_after=?1 WHERE enabled=1 AND login_after<=?2",
            (
                sql(now.checked_add(1).ok_or(StoreError::InvalidData)?)?,
                sql(now)?,
            ),
        )? != 1
        {
            return Err(StoreError::Unauthorized);
        }
        let row: Option<(String,Option<String>,bool)> = self.0.query_row(
            "SELECT a.id,coalesce(p.hash,(SELECT password FROM web_owner WHERE account=a.id AND password_login=1)),a.disabled FROM accounts a LEFT JOIN account_passwords p ON p.account=a.id WHERE a.username=?1", [&value.username], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let hash = row.as_ref().and_then(|r| r.1.as_deref());
        let matches = match hash {
            Some(hash) => verify_password(hash, &value.password)?,
            None => {
                hash_password("synthetic unknown account password")?;
                false
            }
        };
        let (account, _, disabled) = row.ok_or(StoreError::Unauthorized)?;
        if !matches || disabled {
            return Err(StoreError::Unauthorized);
        }
        if let Ok(session) = self.session(&value.device_credential, now) {
            return if session.account_id == account {
                Ok(session)
            } else {
                Err(StoreError::Unauthorized)
            };
        }
        let server = self
            .configuration()?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        let expires = now
            .checked_add(30 * 24 * 3600)
            .ok_or(StoreError::InvalidData)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE account_id=?1 AND revoked=0 AND expires_at>?2)", (&account,sql(now)?), |r|r.get(0))?;
        if active {
            return Err(StoreError::DeviceLinkRequired);
        }
        let device = random_secret().map_err(|_| StoreError::InvalidData)?;
        crate::storage_budget::reserve(&tx, &account, crate::storage_budget::DEVICE, now)?;
        tx.execute(
            "INSERT INTO devices(id,account_id,label,token_hash,expires_at) VALUES(?1,?2,?3,?4,?5)",
            (
                &device,
                &account,
                &value.device_label,
                digest(&value.device_credential).as_slice(),
                sql(expires)?,
            ),
        )?;
        tx.commit()?;
        Ok(Session {
            account_id: account,
            address: format!("@{}:{server}", value.username),
            device_id: device,
            device_label: value.device_label,
            expires_at: expires,
        })
    }
}

use crate::{
    auth::{digest, random_secret},
    store::{Store, StoreError},
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sigil_protocol::accounts::{
    valid_credential, valid_username, Enrollment, Invitation, InviteRequest, Session,
};

pub(crate) const MIGRATION: &str = "
CREATE TABLE invitations (
 id TEXT PRIMARY KEY, token_hash BLOB NOT NULL UNIQUE, username TEXT NOT NULL UNIQUE,
 expires_at INTEGER NOT NULL
);
CREATE TABLE accounts (
 id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, disabled INTEGER NOT NULL DEFAULT 0 CHECK(disabled IN (0,1))
);
CREATE TABLE devices (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id), label TEXT NOT NULL,
 token_hash BLOB UNIQUE, expires_at INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1))
);
";
const DEVICE_LIFETIME: u64 = 30 * 24 * 60 * 60;

impl Store {
    /// Authorization inventory only; device membership does not establish E2EE trust.
    pub fn list_devices(
        &mut self,
        credential: &str,
        after: Option<&str>,
        now: u64,
    ) -> Result<sigil_protocol::accounts::DevicePage, StoreError> {
        use sigil_protocol::accounts::{DevicePage, DeviceSummary, DEVICE_PAGE_SIZE};
        if after.is_some_and(|id| !valid_credential(id)) {
            return Err(StoreError::Invalid("invalid device cursor"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let caller = crate::prekeys::authorize(&tx, credential, now)?;
        let account_id: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [&caller],
            |r| r.get(0),
        )?;
        let mut devices: Vec<DeviceSummary> = tx
            .prepare("SELECT id,label,expires_at,revoked FROM devices WHERE account_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?
            .query_map(
                (&account_id, after.unwrap_or(""), (DEVICE_PAGE_SIZE + 1) as i64),
                |r| Ok(DeviceSummary {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    expires_at: r.get::<_, i64>(2)? as u64,
                    revoked: r.get(3)?,
                }),
            )?
            .collect::<Result<_, _>>()?;
        if devices.iter().any(|d| {
            !valid_credential(&d.id)
                || d.label.is_empty()
                || d.label.len() > 80
                || d.label.chars().any(char::is_control)
                || d.expires_at == 0
                || d.expires_at > i64::MAX as u64
        }) {
            return Err(StoreError::InvalidData);
        }
        let next_after = if devices.len() > DEVICE_PAGE_SIZE {
            devices.pop();
            devices.last().map(|d| d.id.clone())
        } else {
            None
        };
        tx.commit()?;
        Ok(DevicePage {
            account_id,
            devices,
            next_after,
        })
    }
    pub fn invite(&mut self, request: InviteRequest, now: u64) -> Result<Invitation, StoreError> {
        self.issue_invitation(request, None, now)
    }

    pub fn invite_reauthorization(
        &mut self,
        account_id: &str,
        expires_in_seconds: u32,
        now: u64,
    ) -> Result<Invitation, StoreError> {
        let username = self
            .0
            .query_row(
                "SELECT username FROM accounts WHERE id=?1 AND disabled=0",
                [account_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        self.issue_invitation(
            InviteRequest {
                username,
                expires_in_seconds,
            },
            Some(account_id),
            now,
        )
    }

    fn issue_invitation(
        &mut self,
        request: InviteRequest,
        account_id: Option<&str>,
        now: u64,
    ) -> Result<Invitation, StoreError> {
        if !valid_username(&request.username)
            || !(60..=604800).contains(&request.expires_in_seconds)
        {
            return Err(StoreError::Invalid(
                "invalid username or invitation lifetime (60 seconds to 7 days)",
            ));
        }
        if self.configuration()?.settings.is_none() {
            return Err(StoreError::Invalid(
                "configure the server before inviting users",
            ));
        }
        let secret = random_secret().map_err(|_| StoreError::InvalidData)?;
        let id = random_secret().map_err(|_| StoreError::InvalidData)?;
        let expires_at = now
            .checked_add(request.expires_in_seconds.into())
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(StoreError::InvalidData)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM invitations WHERE expires_at <= ?1",
            [now as i64],
        )?;
        let account: Option<(String, bool)> = tx
            .query_row(
                "SELECT id,disabled FROM accounts WHERE username=?1",
                [&request.username],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match (account_id, account) {
            (None, None) => {}
            (Some(expected), Some((id, false))) if id == expected => {}
            (Some(_), _) => return Err(StoreError::NotFound),
            _ => return Err(StoreError::AlreadyExists),
        }
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM invitations WHERE username=?1)",
            [&request.username],
            |r| r.get(0),
        )?;
        if pending {
            return Err(StoreError::AlreadyExists);
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM invitations", [], |r| r.get(0))?;
        if count >= 1024 {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "INSERT INTO invitations VALUES(?1, ?2, ?3, ?4)",
            (
                &id,
                digest(&secret).as_slice(),
                request.username,
                expires_at as i64,
            ),
        )?;
        tx.commit()?;
        Ok(Invitation {
            id,
            secret,
            expires_at,
        })
    }

    pub fn revoke_invitation(&mut self, id: &str) -> Result<(), StoreError> {
        self.0
            .execute("DELETE FROM invitations WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn enroll(&mut self, request: Enrollment, now: u64) -> Result<Session, StoreError> {
        self.accept_invitation(request, now, false)
    }

    pub fn reauthorize(&mut self, request: Enrollment, now: u64) -> Result<Session, StoreError> {
        self.accept_invitation(request, now, true)
    }

    fn accept_invitation(
        &mut self,
        request: Enrollment,
        now: u64,
        reauthorize: bool,
    ) -> Result<Session, StoreError> {
        if !valid_credential(&request.invitation) || !valid_credential(&request.device_credential) {
            return Err(StoreError::Unauthorized);
        }
        if request.device_label.is_empty()
            || request.device_label.len() > 80
            || request.device_label.chars().any(char::is_control)
        {
            return Err(StoreError::Invalid(
                "device_label must contain 1–80 UTF-8 bytes without control characters",
            ));
        }
        let server_name = self
            .configuration()?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        let expires_at = now
            .checked_add(DEVICE_LIFETIME)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(StoreError::InvalidData)?;
        let mut account_id = random_secret().map_err(|_| StoreError::InvalidData)?;
        let device_id = random_secret().map_err(|_| StoreError::InvalidData)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let username: String = tx
            .query_row(
                "SELECT username FROM invitations WHERE token_hash = ?1 AND expires_at > ?2",
                (digest(&request.invitation).as_slice(), now as i64),
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::Unauthorized)?;
        let duplicate: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE token_hash = ?1)",
            [digest(&request.device_credential).as_slice()],
            |r| r.get(0),
        )?;
        if duplicate {
            return Err(StoreError::AlreadyExists);
        }
        if reauthorize {
            account_id = tx
                .query_row(
                    "SELECT id FROM accounts WHERE username=?1 AND disabled=0",
                    [&username],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(StoreError::Unauthorized)?;
            tx.execute(
                "UPDATE devices SET revoked=1,token_hash=NULL WHERE account_id=?1",
                [&account_id],
            )?;
        } else {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE username=?1)",
                [&username],
                |r| r.get(0),
            )?;
            if exists {
                return Err(StoreError::Unauthorized);
            }
            tx.execute(
                "INSERT INTO accounts(id, username) VALUES(?1, ?2)",
                (&account_id, &username),
            )?;
        }
        crate::storage_budget::reserve(&tx, &account_id, crate::storage_budget::DEVICE, now)?;
        tx.execute("INSERT INTO devices(id, account_id, label, token_hash, expires_at) VALUES(?1, ?2, ?3, ?4, ?5)", (&device_id, &account_id, &request.device_label, digest(&request.device_credential).as_slice(), expires_at as i64))?;
        tx.execute(
            "DELETE FROM invitations WHERE token_hash = ?1",
            [digest(&request.invitation).as_slice()],
        )?;
        tx.commit()?;
        Ok(Session {
            account_id,
            address: format!("@{username}:{server_name}"),
            device_id,
            device_label: request.device_label,
            expires_at,
        })
    }

    pub fn session(&self, credential: &str, now: u64) -> Result<Session, StoreError> {
        if !valid_credential(credential) || now > i64::MAX as u64 {
            return Err(StoreError::Unauthorized);
        }
        let server_name = self
            .configuration()?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        self.0.query_row("SELECT a.id, a.username, d.id, d.label, d.expires_at FROM devices d JOIN accounts a ON a.id = d.account_id WHERE d.token_hash = ?1 AND d.revoked = 0 AND a.disabled = 0 AND d.expires_at > ?2", (digest(credential).as_slice(), now as i64), |r| {
            let username: String = r.get(1)?;
            Ok(Session { account_id: r.get(0)?, address: format!("@{username}:{server_name}"), device_id: r.get(2)?, device_label: r.get(3)?, expires_at: r.get::<_, i64>(4)? as u64 })
        }).optional()?.ok_or(StoreError::Unauthorized)
    }

    pub fn rotate_device(
        &mut self,
        current: &str,
        replacement: &str,
        now: u64,
    ) -> Result<Session, StoreError> {
        if !valid_credential(replacement) || replacement == current {
            return Err(StoreError::Invalid(
                "provide a new random 256-bit device credential",
            ));
        }
        let mut session = self.session(current, now)?;
        let expires_at = now
            .checked_add(DEVICE_LIFETIME)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(StoreError::InvalidData)?;
        let count = self.0.execute("UPDATE devices SET token_hash = ?1, expires_at = ?2 WHERE id = ?3 AND token_hash = ?4 AND revoked = 0 AND EXISTS(SELECT 1 FROM accounts WHERE id = devices.account_id AND disabled = 0) AND NOT EXISTS(SELECT 1 FROM devices WHERE token_hash = ?1)", (digest(replacement).as_slice(), expires_at as i64, &session.device_id, digest(current).as_slice()))?;
        if count == 0 {
            return Err(StoreError::AlreadyExists);
        }
        session.expires_at = expires_at;
        Ok(session)
    }

    pub fn revoke_device(
        &mut self,
        credential: &str,
        device_id: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let caller = crate::prekeys::authorize(&tx, credential, now)?;
        let count = tx.execute(
            "UPDATE devices SET revoked = 1, token_hash = NULL WHERE id = ?1 AND account_id = (SELECT account_id FROM devices WHERE id = ?2)",
            (device_id, caller),
        )?;
        if count == 0 {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        Ok(())
    }

    pub fn disable_account(&mut self, account_id: &str) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE accounts SET disabled = 1 WHERE id = ?1",
            [account_id],
        )? == 0
        {
            return Err(StoreError::NotFound);
        }
        tx.execute(
            "UPDATE devices SET revoked = 1, token_hash = NULL WHERE account_id = ?1",
            [account_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

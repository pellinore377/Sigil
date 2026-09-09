use crate::push_config::{sql, unsigned};
use crate::{
    auth::{digest, random_secret},
    store::{Store, StoreError},
};
use argon2::{
    password_hash::{PasswordHash, SaltString},
    Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version,
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_protocol::{accounts::valid_username, admin::Role};
use zeroize::Zeroizing;

pub(crate) const MIGRATION: &str = "
CREATE TABLE web_owner(id INTEGER PRIMARY KEY CHECK(id=1),setup_hash BLOB,password TEXT,account TEXT REFERENCES accounts(id),issuer TEXT,subject TEXT,picture TEXT,suggested TEXT,oidc_revision INTEGER,password_login INTEGER NOT NULL DEFAULT 1,login_after INTEGER NOT NULL DEFAULT 0);
INSERT INTO web_owner(id) VALUES(1);
CREATE TABLE web_sessions(hash BLOB PRIMARY KEY,expires INTEGER NOT NULL,touched INTEGER NOT NULL);
CREATE TABLE web_oidc(flow TEXT PRIMARY KEY,browser BLOB NOT NULL,link INTEGER NOT NULL,expires INTEGER NOT NULL);
";
const SESSION_SECONDS: u64 = 8 * 3600;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub code: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub server_name: String,
    pub public_origin: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub username: String,
    pub password: Zeroizing<String>,
}
#[derive(Serialize)]
pub struct Status {
    pub claimed: bool,
    pub authenticated: bool,
    pub complete: bool,
    pub username: Option<String>,
    pub server_name: Option<String>,
    pub public_origin: Option<String>,
    pub oidc_enabled: bool,
    pub oidc_login: bool,
    pub oidc_linked: bool,
    pub password_login: bool,
    pub suggested_username: Option<String>,
    pub display_name: Option<String>,
    pub suggested_display_name: Option<String>,
}
fn random() -> Result<String, StoreError> {
    random_secret().map_err(|_| StoreError::InvalidData)
}
fn argon() -> Result<Argon2<'static>, StoreError> {
    Ok(Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 1, Some(32)).map_err(|_| StoreError::InvalidData)?,
    ))
}
fn hash_password(password: &str) -> Result<String, StoreError> {
    if password.chars().count() < 15
        || password.len() > 1024
        || password.chars().any(char::is_control)
    {
        return Err(StoreError::Invalid(
            "Use a password of at least 15 characters, up to 1024 bytes",
        ));
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| StoreError::InvalidData)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| StoreError::InvalidData)?;
    argon()?
        .hash_password(password.as_bytes(), &salt)
        .map(|v| v.to_string())
        .map_err(|_| StoreError::InvalidData)
}
fn verify_password(hash: &str, password: &str) -> Result<bool, StoreError> {
    if hash.len() > 256 || !hash.starts_with("$argon2id$v=19$m=65536,t=3,p=1$") {
        return Err(StoreError::InvalidData);
    }
    Ok(argon()?
        .verify_password(
            password.as_bytes(),
            &PasswordHash::new(hash).map_err(|_| StoreError::InvalidData)?,
        )
        .is_ok())
}
pub(crate) fn new_session(db: &rusqlite::Connection, now: u64) -> Result<String, StoreError> {
    db.execute(
        "DELETE FROM web_sessions WHERE expires<=?1 OR touched<=?2",
        (sql(now)?, sql(now.saturating_sub(1800))?),
    )?;
    if db.query_row("SELECT count(*) FROM web_sessions", [], |r| {
        r.get::<_, u32>(0)
    })? >= 16
    {
        return Err(StoreError::Busy);
    }
    let token = random()?;
    db.execute(
        "INSERT INTO web_sessions VALUES(?1,?2,?3)",
        (
            digest(&token).as_slice(),
            sql(crate::push::deadline(now, SESSION_SECONDS)?)?,
            sql(now)?,
        ),
    )?;
    Ok(token)
}
impl Store {
    pub fn web_setup_code(&mut self) -> Result<Option<String>, StoreError> {
        let claimed: bool =
            self.0
                .query_row("SELECT password IS NOT NULL FROM web_owner", [], |r| {
                    r.get(0)
                })?;
        if claimed {
            return Ok(None);
        }
        let code = random()?;
        self.0.execute(
            "UPDATE web_owner SET setup_hash=?1",
            [digest(&code).as_slice()],
        )?;
        Ok(Some(code))
    }
    pub(crate) fn web_session(&self, token: &str, now: u64) -> Result<(), StoreError> {
        if !sigil_protocol::accounts::valid_credential(token) {
            return Err(StoreError::Unauthorized);
        }
        let ok: bool = self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM web_sessions WHERE hash=?1 AND expires>?2 AND touched>?3)",
            (
                digest(token).as_slice(),
                sql(now)?,
                sql(now.saturating_sub(1800))?,
            ),
            |r| r.get(0),
        )?;
        if !ok {
            return Err(StoreError::Unauthorized);
        }
        let disabled: bool = self.0.query_row("SELECT EXISTS(SELECT 1 FROM web_owner w JOIN accounts a ON a.id=w.account LEFT JOIN account_policy p ON p.account=a.id WHERE a.disabled=1 OR p.role!='\"administrator\"')", [], |r| r.get(0))?;
        if disabled {
            return Err(StoreError::Unauthorized);
        }
        self.0.execute(
            "UPDATE web_sessions SET touched=?2 WHERE hash=?1",
            (digest(token).as_slice(), sql(now)?),
        )?;
        Ok(())
    }
    pub(crate) fn web_status(&self, token: Option<&str>, now: u64) -> Result<Status, StoreError> {
        let authenticated = token.is_some_and(|v| self.web_session(v, now).is_ok());
        let (claimed, account, issuer, password_login): (
            bool,
            Option<String>,
            Option<String>,
            bool,
        ) = self.0.query_row(
            "SELECT password IS NOT NULL,account,issuer,password_login FROM web_owner",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let username = if authenticated {
            account
                .as_ref()
                .map(|id| {
                    self.0
                        .query_row("SELECT username FROM accounts WHERE id=?1", [id], |r| {
                            r.get(0)
                        })
                })
                .transpose()?
        } else {
            None
        };
        let configuration = self.configuration()?;
        Ok(Status {
            claimed,
            authenticated,
            complete: claimed && account.is_some(),
            display_name: if authenticated {
                account
                    .as_ref()
                    .map(|id| crate::profile::read(&self.0, id).map(|p| p.display_name))
                    .transpose()?
            } else {
                None
            },
            suggested_display_name: if authenticated {
                self.0
                    .query_row("SELECT suggested_name FROM web_owner", [], |r| r.get(0))?
            } else {
                None
            },
            username,
            server_name: configuration.settings.map(|s| s.server_name),
            public_origin: self.administration_policy()?.public_origin,
            oidc_enabled: self.oidc_configuration()?.enabled,
            oidc_login: self.web_oidc_ready()?,
            oidc_linked: authenticated && issuer.is_some(),
            password_login,
            suggested_username: if authenticated {
                self.0
                    .query_row("SELECT suggested FROM web_owner", [], |r| r.get(0))?
            } else {
                None
            },
        })
    }
    pub(crate) fn web_claim(&mut self, value: Claim, now: u64) -> Result<String, StoreError> {
        use subtle::ConstantTimeEq;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (old, claimed): (Option<Vec<u8>>, bool) = tx.query_row(
            "SELECT setup_hash,password IS NOT NULL FROM web_owner",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if claimed || !old.is_some_and(|v| bool::from(v.ct_eq(&digest(&value.code)))) {
            return Err(StoreError::Unauthorized);
        }
        crate::admin::origin_url(&value.public_origin)?;
        let settings = sigil_protocol::Settings {
            server_name: value.server_name,
            default_quota_bytes: 10 * 1024u64.pow(3),
            max_attachment_bytes: 1024u64.pow(3),
        };
        settings.validate().map_err(StoreError::Invalid)?;
        let prior: Option<String> =
            tx.query_row("SELECT settings FROM configuration", [], |r| r.get(0))?;
        if let Some(prior) = prior {
            let prior: sigil_protocol::Settings =
                serde_json::from_str(&prior).map_err(|_| StoreError::InvalidData)?;
            if prior.server_name != settings.server_name {
                return Err(StoreError::Conflict);
            }
        } else {
            tx.execute(
                "UPDATE configuration SET revision=revision+1,settings=?1",
                [serde_json::to_string(&settings).map_err(|_| StoreError::InvalidData)?],
            )?;
        }
        let mut policy = crate::admin::policy(&tx)?;
        if policy
            .public_origin
            .as_ref()
            .is_some_and(|old| old != &value.public_origin)
        {
            return Err(StoreError::Conflict);
        }
        policy.public_origin = Some(value.public_origin);
        policy.revision += 1;
        tx.execute(
            "UPDATE admin_policy SET value=?1",
            [serde_json::to_string(&policy).map_err(|_| StoreError::InvalidData)?],
        )?;
        let password = hash_password(&value.password)?;
        tx.execute(
            "UPDATE web_owner SET password=?1,setup_hash=NULL,password_login=1",
            [password],
        )?;
        let session = new_session(&tx, now)?;
        tx.commit()?;
        Ok(session)
    }
    pub(crate) fn web_login(&mut self, value: Login, now: u64) -> Result<String, StoreError> {
        let (hash, enabled, after, account): (Option<String>, bool, u64, Option<String>) =
            self.0.query_row(
                "SELECT password,password_login,login_after,account FROM web_owner",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, unsigned(r, 2)?, r.get(3)?)),
            )?;
        if !enabled || after > now || value.password.len() > 1024 {
            return Err(StoreError::Unauthorized);
        }
        self.0.execute(
            "UPDATE web_owner SET login_after=?1",
            [sql(crate::push::deadline(now, 2)?)?],
        )?;
        let hash = hash.ok_or(StoreError::Unauthorized)?;
        let matches = verify_password(&hash, &value.password)?;
        let username_matches = match account {
            Some(id) => self.0.query_row(
                "SELECT username=?2 AND disabled=0 FROM accounts WHERE id=?1",
                (id, &value.username),
                |r| r.get::<_, bool>(0),
            )?,
            None => value.username.is_empty(),
        };
        if !matches || !username_matches {
            return Err(StoreError::Unauthorized);
        }
        new_session(&self.0, now)
    }
    pub(crate) fn web_finish_setup(
        &mut self,
        token: &str,
        username: &str,
        display_name: Option<&str>,
        now: u64,
    ) -> Result<(), StoreError> {
        self.web_session(token, now)?;
        if !valid_username(username) {
            return Err(StoreError::Invalid("Choose a valid lowercase username"));
        }
        if display_name.is_some_and(|name| !sigil_protocol::profile::valid_name(name)) {
            return Err(StoreError::Invalid(
                "Choose a display name of up to 128 characters without control characters",
            ));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> =
            tx.query_row("SELECT account FROM web_owner", [], |r| r.get(0))?;
        if existing.is_some() {
            return Err(StoreError::Conflict);
        }
        let id = random()?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE username=?1)",
            [username],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(StoreError::AlreadyExists);
        }
        tx.execute(
            "INSERT INTO accounts(id,username) VALUES(?1,?2)",
            (&id, username),
        )?;
        tx.execute(
            "INSERT INTO account_policy(account,role) VALUES(?1,?2)",
            (
                &id,
                serde_json::to_string(&Role::Administrator).map_err(|_| StoreError::InvalidData)?,
            ),
        )?;
        tx.execute("UPDATE web_owner SET account=?1", [&id])?;
        if let Some(name) = display_name {
            tx.execute("INSERT INTO account_profiles VALUES(?1,1,?2)", (&id, name))?;
        }
        let binding: (Option<String>, Option<String>) =
            tx.query_row("SELECT issuer,subject FROM web_owner", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        if let (Some(issuer), Some(subject)) = binding {
            crate::oidc::bind(&tx, &issuer, &subject, &id)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn web_password_policy(
        &mut self,
        token: &str,
        enabled: bool,
        now: u64,
    ) -> Result<(), StoreError> {
        self.web_session(token, now)?;
        if !enabled {
            let linked: bool = self.0.query_row("SELECT issuer IS NOT NULL AND subject IS NOT NULL AND account IS NOT NULL FROM web_owner", [], |r| r.get(0))?;
            if !linked || !self.web_oidc_ready()? {
                return Err(StoreError::Invalid(
                    "Link and verify OIDC before disabling password login",
                ));
            }
        }
        self.0
            .execute("UPDATE web_owner SET password_login=?1", [enabled])?;
        Ok(())
    }
    pub(crate) fn web_logout(&self, token: &str) -> Result<(), StoreError> {
        self.0.execute(
            "DELETE FROM web_sessions WHERE hash=?1",
            [digest(token).as_slice()],
        )?;
        self.0.execute(
            "DELETE FROM web_oidc WHERE browser=?1",
            [digest(token).as_slice()],
        )?;
        Ok(())
    }
}

pub(crate) struct Identity {
    pub subject: String,
    pub username: Option<String>,
    pub picture: Option<String>,
    pub name: Option<String>,
}
impl Store {
    pub(crate) fn web_oidc_start(
        &mut self,
        cookie: Option<&str>,
        now: u64,
    ) -> Result<(String, sigil_protocol::oidc::Started), StoreError> {
        let link = cookie.is_some_and(|c| self.web_session(c, now).is_ok());
        let browser = if link {
            cookie.unwrap().to_owned()
        } else {
            random()?
        };
        if !link {
            let linked = self.web_oidc_ready()?;
            if !linked {
                return Err(StoreError::Forbidden);
            }
        }
        self.0.execute(
            "DELETE FROM web_oidc WHERE expires<=?1 OR browser=?2",
            (sql(now)?, digest(&browser).as_slice()),
        )?;
        let id = random()?;
        let started = self.oidc_begin(
            sigil_protocol::oidc::Start {
                request_id: id.clone(),
                secret: random()?,
                username: None,
                replace_devices: false,
            },
            None,
            now,
            true,
        )?;
        self.0.execute(
            "INSERT INTO web_oidc VALUES(?1,?2,?3,?4)",
            (
                &id,
                digest(&browser).as_slice(),
                link,
                sql(started.expires_at)?,
            ),
        )?;
        Ok((browser, started))
    }
    pub(crate) fn web_oidc_browser(
        &self,
        state: &str,
        cookie: Option<&str>,
        now: u64,
    ) -> Result<bool, StoreError> {
        let flow: Option<(Vec<u8>, bool, u64)> = self.0.query_row("SELECT w.browser,w.link,w.expires FROM web_oidc w JOIN oidc_flows f ON f.id=w.flow WHERE f.state=?1", [state], |r| Ok((r.get(0)?, r.get(1)?, unsigned(r,2)?))).optional()?;
        let Some((browser, link, expires)) = flow else {
            return Ok(false);
        };
        let cookie = cookie.ok_or(StoreError::Unauthorized)?;
        if browser != digest(cookie) || expires <= now {
            return Err(StoreError::Unauthorized);
        }
        if link {
            self.web_session(cookie, now)?;
        }
        Ok(true)
    }
    pub(crate) fn web_oidc_verified(
        &mut self,
        callback: crate::oidc::Callback,
        identity: Result<Identity, StoreError>,
        cookie: &str,
        now: u64,
    ) -> Result<String, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let link: bool = tx
            .query_row(
                "SELECT link FROM web_oidc WHERE flow=?1 AND browser=?2 AND expires>?3",
                (callback.id(), digest(cookie).as_slice(), sql(now)?),
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::Unauthorized)?;
        let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM oidc_flows f JOIN oidc_configuration c ON c.revision=f.revision WHERE f.id=?1 AND f.status=0 AND f.expires>?2 AND f.revision=?3)", (callback.id(), sql(now)?, sql(callback.revision())?), |r| r.get(0))?;
        if !valid {
            return Err(StoreError::Unauthorized);
        }
        tx.execute("DELETE FROM web_oidc WHERE flow=?1", [callback.id()])?;
        tx.execute("DELETE FROM oidc_flows WHERE id=?1", [callback.id()])?;
        let identity = match identity {
            Ok(v) => v,
            Err(e) => {
                tx.commit()?;
                return Err(e);
            }
        };
        let (issuer, subject, account): (Option<String>, Option<String>, Option<String>) = tx
            .query_row("SELECT issuer,subject,account FROM web_owner", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        if link {
            let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM web_sessions WHERE hash=?1 AND expires>?2 AND touched>?3)", (digest(cookie).as_slice(),sql(now)?,sql(now.saturating_sub(1800))?), |r|r.get(0))?;
            if !live {
                return Err(StoreError::Unauthorized);
            }
            // Replacing an established administrator identity requires an explicit unlink first.
            if issuer.as_deref().is_some_and(|i| i != callback.issuer())
                || subject.as_ref().is_some_and(|s| s != &identity.subject)
            {
                return Err(StoreError::Conflict);
            }
            if let Some(account) = &account {
                crate::oidc::bind(&tx, callback.issuer(), &identity.subject, account)?;
            }
            tx.execute(
                "UPDATE web_owner SET issuer=?1,subject=?2,picture=?3,suggested=?4,oidc_revision=?5,suggested_name=?6",
                (
                    callback.issuer(),
                    &identity.subject,
                    identity
                        .picture
                        .filter(|p| p.len() <= 2048 && crate::egress::endpoint(p).is_ok()),
                    identity.username.filter(|u| valid_username(u)),
                    sql(callback.revision())?,
                    identity.name.filter(|name| sigil_protocol::profile::valid_name(name)),
                ),
            )?;
        } else if issuer.as_deref() != Some(callback.issuer())
            || subject.as_ref() != Some(&identity.subject)
        {
            tx.commit()?;
            return Err(StoreError::Unauthorized);
        }
        if let Some(account) = &account {
            let active: bool = tx.query_row(
                "SELECT disabled=0 FROM accounts WHERE id=?1",
                [account],
                |r| r.get(0),
            )?;
            if !active {
                return Err(StoreError::Unauthorized);
            }
        }
        tx.execute(
            "DELETE FROM web_sessions WHERE hash=?1",
            [digest(cookie).as_slice()],
        )?;
        let session = new_session(&tx, now)?;
        tx.commit()?;
        Ok(session)
    }
}

#[cfg(test)]
#[path = "web_admin_tests.rs"]
mod tests;

impl Store {
    pub fn web_reset_login(&mut self) -> Result<(), StoreError> {
        self.0.execute_batch("BEGIN IMMEDIATE; DELETE FROM web_sessions; DELETE FROM web_oidc; UPDATE web_owner SET password=NULL,setup_hash=NULL,password_login=1,login_after=0; COMMIT;")?;
        Ok(())
    }
    pub(crate) fn web_change_password(
        &mut self,
        token: &str,
        old: &str,
        new: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        self.web_session(token, now)?;
        if old.len() > 1024 {
            return Err(StoreError::Unauthorized);
        }
        let hash: String = self
            .0
            .query_row("SELECT password FROM web_owner", [], |r| r.get(0))?;
        if !verify_password(&hash, old)? {
            return Err(StoreError::Unauthorized);
        }
        let hash = hash_password(new)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE web_owner SET password=?1", [hash])?;
        tx.execute(
            "DELETE FROM web_sessions WHERE hash!=?1",
            [digest(token).as_slice()],
        )?;
        tx.execute("DELETE FROM web_oidc", [])?;
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    pub(crate) fn web_unlink_oidc(
        &mut self,
        token: &str,
        password: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        self.web_session(token, now)?;
        if password.len() > 1024 {
            return Err(StoreError::Unauthorized);
        }
        let (hash, enabled): (String, bool) =
            self.0
                .query_row("SELECT password,password_login FROM web_owner", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        if !enabled || !verify_password(&hash, password)? {
            return Err(StoreError::Unauthorized);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM oidc_bindings WHERE account=(SELECT account FROM web_owner) AND issuer=(SELECT issuer FROM web_owner)",[])?;
        tx.execute(
            "UPDATE web_owner SET issuer=NULL,subject=NULL,picture=NULL,suggested=NULL,suggested_name=NULL,oidc_revision=NULL",
            [],
        )?;
        tx.execute("DELETE FROM web_oidc", [])?;
        tx.execute(
            "DELETE FROM web_sessions WHERE hash!=?1",
            [digest(token).as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    fn web_oidc_ready(&self) -> Result<bool, StoreError> {
        Ok(self.0.query_row("SELECT EXISTS(SELECT 1 FROM web_owner w JOIN oidc_configuration o ON o.revision=w.oidc_revision WHERE o.value IS NOT NULL AND w.password IS NOT NULL AND w.issuer IS NOT NULL AND w.subject IS NOT NULL)",[],|r|r.get(0))?)
    }
}

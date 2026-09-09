use crate::{
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sigil_protocol::profile::{valid_name, Profile};

pub(crate) const MIGRATION: &str = "
CREATE TABLE account_profiles(account TEXT PRIMARY KEY REFERENCES accounts(id),revision INTEGER NOT NULL,display_name TEXT NOT NULL);
ALTER TABLE web_owner ADD COLUMN suggested_name TEXT;
";

pub(crate) fn read(db: &Connection, account: &str) -> Result<Profile, StoreError> {
    Ok(db
        .query_row(
            "SELECT revision,display_name FROM account_profiles WHERE account=?1",
            [account],
            |r| {
                Ok(Profile {
                    revision: unsigned(r, 0)?,
                    display_name: r.get(1)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}
fn update(db: &mut Connection, account: &str, value: Profile) -> Result<Profile, StoreError> {
    if !valid_name(&value.display_name) {
        return Err(StoreError::Invalid(
            "Use a display name of up to 128 characters without control characters",
        ));
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if read(&tx, account)?.revision != value.revision {
        return Err(StoreError::Conflict);
    }
    let value = Profile {
        revision: crate::push_config::next(value.revision)?,
        ..value
    };
    tx.execute("INSERT INTO account_profiles VALUES(?1,?2,?3) ON CONFLICT(account) DO UPDATE SET revision=excluded.revision,display_name=excluded.display_name",
        (account,sql(value.revision)?,&value.display_name))?;
    tx.commit()?;
    Ok(value)
}
impl Store {
    pub fn profile(&self, token: &str, now: u64) -> Result<Profile, StoreError> {
        read(&self.0, &self.session(token, now)?.account_id)
    }
    pub fn update_profile(
        &mut self,
        token: &str,
        value: Profile,
        now: u64,
    ) -> Result<Profile, StoreError> {
        let account = self.session(token, now)?.account_id;
        update(&mut self.0, &account, value)
    }
    pub(crate) fn web_profile(&self, token: &str, now: u64) -> Result<Profile, StoreError> {
        read(&self.0, &self.web_account(token, now)?)
    }
    pub(crate) fn web_update_profile(
        &mut self,
        token: &str,
        value: Profile,
        now: u64,
    ) -> Result<Profile, StoreError> {
        let account = self.web_account(token, now)?;
        update(&mut self.0, &account, value)
    }
    fn web_account(&self, token: &str, now: u64) -> Result<String, StoreError> {
        self.web_session(token, now)?;
        self.0
            .query_row("SELECT account FROM web_owner", [], |r| {
                r.get::<_, Option<String>>(0)
            })?
            .ok_or(StoreError::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_are_account_scoped_revisioned_and_survive_reauthorization() {
        let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
        let original = store.session(&alice, now).unwrap();
        assert_eq!(store.profile(&alice, now).unwrap(), Profile::default());
        let saved = store
            .update_profile(
                &alice,
                Profile {
                    revision: 0,
                    display_name: "Alice ✒".into(),
                },
                now,
            )
            .unwrap();
        assert_eq!(store.profile(&bob, now).unwrap(), Profile::default());
        assert!(matches!(
            store.update_profile(&alice, Profile::default(), now),
            Err(StoreError::Conflict)
        ));
        for name in [
            " leading",
            "trailing ",
            "line\nbreak",
            "spoof\u{202e}",
            "x".repeat(129).as_str(),
        ] {
            assert!(store
                .update_profile(
                    &alice,
                    Profile {
                        revision: 1,
                        display_name: name.into()
                    },
                    now
                )
                .is_err());
        }
        let invitation = store
            .invite_reauthorization(&original.account_id, 600, now)
            .unwrap();
        let replacement = crate::auth::random_secret().unwrap();
        let renewed = store
            .reauthorize(
                sigil_protocol::accounts::Enrollment {
                    invitation: invitation.secret,
                    device_credential: replacement.clone(),
                    device_label: "Replacement".into(),
                },
                now,
            )
            .unwrap();
        assert_eq!(renewed.account_id, original.account_id);
        assert_eq!(renewed.address, original.address);
        assert!(store.update_profile(&alice, saved.clone(), now).is_err());
        drop(store);
        let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
        assert_eq!(store.profile(&replacement, now).unwrap(), saved);
        assert_eq!(
            store
                .update_profile(
                    &replacement,
                    Profile {
                        revision: 1,
                        display_name: String::new()
                    },
                    now
                )
                .unwrap()
                .revision,
            2
        );
    }
    #[test]
    fn schema_28_migrates_without_changing_existing_accounts() {
        let (dir, store, alice, _, now) = crate::admin::tests::setup();
        let before = store.session(&alice, now).unwrap();
        store.0.execute_batch("DROP TABLE account_passwords; DROP TABLE password_policy; ALTER TABLE oidc_grants DROP COLUMN replace_devices; DROP TABLE account_profiles; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; ALTER TABLE web_owner DROP COLUMN suggested_name; PRAGMA user_version=28;").unwrap();
        drop(store);
        let store = Store::open(&dir.path().join("sigil.db")).unwrap();
        assert_eq!(store.session(&alice, now).unwrap(), before);
        assert_eq!(store.profile(&alice, now).unwrap(), Profile::default());
        assert!(!store.oidc_access(&alice, now).unwrap().retiring);
        assert_eq!(
            store
                .0
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            30
        );
    }
}

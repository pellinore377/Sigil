//! Server revocation of an own-account device ends its local sessions and copies.
use super::*;

impl ClientStore {
    /// Page the signed-in account's inventory and suspend every device it reports
    /// revoked. Absence from a page proves nothing; only revoked=true acts.
    pub fn reconcile_own_devices_online(&mut self) -> Result<usize, Error> {
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let client = self.connected_client()?;
        self.ensure_account_key_online()?;
        // Sibling devices are trusted exactly when the account key endorsed them.
        let (username, server) = session
            .address
            .strip_prefix('@')
            .and_then(|v| v.split_once(':'))
            .ok_or(Error::InvalidStore)?;
        let directory = client.contact_directory(username)?;
        self.reconcile_contact_trust(
            &directory,
            (server, username, id(&session.account_id)?),
            true,
            None,
            conversations::now(),
        )?;
        let mut after: Option<String> = None;
        let mut revoked = Vec::new();
        for _ in 0..64 {
            let page = client.devices(after.as_deref())?;
            if page.account_id != session.account_id {
                return Err(Error::Conflict);
            }
            for entry in page.devices {
                if entry.revoked && entry.id != session.device_id {
                    revoked.push(id(&entry.id)?);
                }
            }
            match page.next_after {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        let mut count = 0;
        for device in revoked {
            count += usize::from(self.suspend_revoked_own_device(&device)?);
        }
        Ok(count)
    }
    /// Local half of a server-confirmed revocation: suspend the same-account peer,
    /// cancel its queued packets and retire its sessions. True on the first application.
    pub(crate) fn suspend_revoked_own_device(&mut self, device: &Id) -> Result<bool, Error> {
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        if session.device_id == transport::hex(device) {
            return Ok(false);
        }
        let account = id(&session.account_id)?;
        let server = session
            .address
            .strip_prefix('@')
            .and_then(|v| v.split_once(':'))
            .map(|(_, server)| server)
            .ok_or(Error::InvalidStore)?;
        let reference = reference(server, device);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = match load(&tx, &self.key, &reference) {
            Ok(record) => record,
            Err(Error::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        // Only a record that binds this device to the signed-in account qualifies.
        if record.signed.binding.server != server || record.signed.binding.account != account {
            return Ok(false);
        }
        let first = !record.revoked;
        if first {
            record.suspended = true;
            record.revoked = true;
            save(&tx, &self.key, &reference, &record)?;
        }
        release_sessions(&tx, &self.key, &reference)?;
        tx.commit()?;
        Ok(first)
    }
}

use super::*;

impl ClientStore {
    pub(super) fn mobile_recovery_generate(&self) -> Result<Value, Error> {
        self.connected_client()?;
        match self.recovery_status() {
            Err(Error::Unprepared | Error::NotFound) => (),
            Ok(_) => return Err(Error::Conflict),
            Err(error) => return Err(error),
        }
        let mut secret = Zeroizing::new([0; 32]);
        getrandom::fill(secret.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
        Ok(json!({"secret": transport::hex(secret.as_ref())}))
    }
    pub(super) fn mobile_devices(&self, cursor: Option<String>) -> Result<Value, Error> {
        if cursor.as_ref().is_some_and(|value| value.len() > 2048) {
            return Err(Error::Limit);
        }
        let cursor = cursor
            .map(|value| {
                serde_json::from_str::<DeviceReviewCursor>(&value).map_err(|_| Error::InvalidEvent)
            })
            .transpose()?;
        let page = self.review_devices_online(cursor.as_ref())?;
        let devices = page.devices.into_iter().map(|device| json!({
            "id":transport::hex(&device.device), "current":device.is_current,
            "label":device.inventory.as_ref().map(|entry| entry.label.as_str()),
            "revoked":device.inventory.as_ref().map(|entry| entry.revoked),
            "expires":device.inventory.as_ref().map(|entry| entry.expires_at),
            "fingerprint":device.peer.as_ref().map(|peer| transport::hex(&peer.fingerprint)),
            "verified":device.peer.as_ref().is_some_and(|peer| peer.verified && !peer.blocked && peer.changed_fingerprint.is_none() && peer.replaced_by.is_none()),
        })).collect::<Vec<_>>();
        Ok(
            json!({"devices":devices,"next":page.next.map(|cursor| serde_json::to_string(&cursor).map_err(|_| Error::InvalidStore)).transpose()?}),
        )
    }
    pub(super) fn mobile_storage(&self) -> Result<Value, Error> {
        let allocated = std::fs::metadata(self.db.path().ok_or(Error::InvalidStore)?)?.len();
        let (media, media_used, budget) = self.mobile_cache()?.storage_usage()?;
        let recovery = match self.history_recovery_progress() {
            Ok(value) => {
                json!({"enabled":true,"last":value.last_checkpoint_at,"pending":value.unprotected_records,"records":value.committed_records,"days":self.recovery_policy()?.history_days})
            }
            Err(Error::Unprepared | Error::NotFound) => json!({"enabled":false}),
            Err(error) => return Err(error),
        };
        Ok(
            json!({"database":allocated,"media":media,"media_used":media_used,"budget":budget,"recovery":recovery}),
        )
    }
}

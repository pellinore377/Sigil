use super::*;

impl ClientStore {
    pub(super) fn mobile_account_access(&self) -> Result<Value, Error> {
        Ok(
            json!({"access": self.connected_client()?.oidc_access()?, "link_pending": self.oidc_link_pending()?}),
        )
    }
    pub(super) fn mobile_oidc_account(&mut self, action: &str) -> Result<Value, Error> {
        self.connected_client()?;
        match action {
            "start" => {
                let access = self.connected_client()?.oidc_access()?;
                if access.linked || access.retiring || access.issuer.is_none() {
                    return Err(Error::Conflict);
                }
                if !self.oidc_link_pending()? {
                    self.prepare_oidc_link()?;
                }
                Ok(serde_json::to_value(self.start_oidc_online()?)
                    .map_err(|_| Error::InvalidStore)?)
            }
            "resume" => {
                if self.finish_oidc_link_online()? {
                    return self.mobile_account_access();
                }
                Ok(serde_json::to_value(self.start_oidc_online()?)
                    .map_err(|_| Error::InvalidStore)?)
            }
            "cancel" => {
                self.cancel_oidc_link()?;
                self.mobile_account_access()
            }
            _ => Err(Error::InvalidEvent),
        }
    }
    /// A connected device publishes its binding and, once, the account key.
    pub(super) fn after_sign_in(&mut self) -> Result<(), Error> {
        if self.enrollment_kind()? != "connected" {
            return Ok(());
        }
        self.publish_device_binding_online()?;
        self.ensure_account_key_online()?;
        self.ensure_backup()
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
            "verified":device.peer.as_ref().is_some_and(|peer| peer.trusted && !peer.blocked && peer.changed_fingerprint.is_none() && peer.replaced_by.is_none()),
        })).collect::<Vec<_>>();
        Ok(
            json!({"devices":devices,"next":page.next.map(|cursor| serde_json::to_string(&cursor).map_err(|_| Error::InvalidStore)).transpose()?}),
        )
    }
    /// A report for troubleshooting a device that cannot be inspected directly.
    /// Counts and stage names only: no message content, addresses, names, peer or
    /// device identifiers, and no key material. Mailbox sequences are the server's
    /// own numbering for this device and say nothing about what was said.
    pub(super) fn mobile_diagnostics(&self) -> Result<Value, Error> {
        let count = |sql: &str| -> Result<i64, Error> {
            Ok(self.db.query_row(sql, [], |r| r.get(0)).unwrap_or(-1))
        };
        let oldest = |table: &str, filter: &str| -> Result<String, Error> {
            let value: Option<i64> = self
                .db
                .query_row(
                    &format!("SELECT min(sequence) FROM {table} {filter}"),
                    [],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            Ok(value.map_or_else(|| "-".to_owned(), |v| v.to_string()))
        };
        let scalar = |sql: &str| -> i64 { self.db.query_row(sql, [], |r| r.get(0)).unwrap_or(-1) };
        let outgoing_gc = self.control_cleanup_progress(0);
        let accepted_gc = self.control_cleanup_progress(1);
        let report = format!(
            "Sigil diagnostics\nschema {schema}, client {client}\n\
             queued: outbox {outbox} (deepest session {deepest}, at cap {atcap}), intents {intents}, retry requests {retryout}, call jobs {calls} (waiting on recipient {callwait})\n\
             unacknowledged: incoming {incoming}, retry {retry}, recovered {recovered}, group {group}, abandoned {abandoned}\n\
             oldest unacknowledged: incoming {oi}, retry {orr}, recovered {orc}, group {og}, abandoned {oa}\n\
             prekeys: slots {slots}, publications pending {pubs}, initiations {inits}\n\
             backoff: outbound {ob} (unknown recipient {unknown}), intents {ib}\n\
             store: peers {peers}, sessions {sessions} (retired {retired}, deepest peer {perpeer}, unbound {unbound}), inbox {inbox}, deliveries {deliveries}\n\
             recovery: retry incoming {retryin}, unfinished responses {unfinished}, retired requests {retiredreq}\n\
             retry gc: pending controls {pendingout}, retired controls {retiredout}, cursor outgoing {c0}/{r0} ahead, accepted {c1}/{r1} ahead",
            schema = crate::DATABASE_VERSION,
            client = env!("CARGO_PKG_VERSION"),
            outbox = count("SELECT count(*) FROM outbox WHERE packet IS NOT NULL")?,
            deepest = scalar("SELECT coalesce(max(n),0) FROM (SELECT count(*) n FROM outbox WHERE packet IS NOT NULL GROUP BY session)"),
            atcap = scalar("SELECT count(*) FROM (SELECT session FROM outbox WHERE packet IS NOT NULL GROUP BY session HAVING count(*)>=256)"),
            intents = count("SELECT count(*) FROM send_intents")?,
            retryout = count("SELECT count(*) FROM retry_outbox")?,
            calls = scalar("SELECT count(*) FROM call_jobs"),
            callwait = scalar(&format!(
                "SELECT count(*) FROM call_job_backoff WHERE until>{}",
                crate::conversations::now()
            )),
            incoming = count("SELECT count(*) FROM incoming WHERE acknowledged=0")?,
            retry = count("SELECT count(*) FROM retry_incoming WHERE acknowledged=0")?,
            recovered = count("SELECT count(*) FROM recovered_deliveries WHERE acknowledged=0")?,
            group = count("SELECT count(*) FROM group_incoming WHERE acknowledged=0")?,
            abandoned = count("SELECT count(*) FROM abandoned_deliveries")?,
            oi = oldest("incoming", "WHERE acknowledged=0")?,
            orr = oldest("retry_incoming", "WHERE acknowledged=0")?,
            orc = oldest("recovered_deliveries", "WHERE acknowledged=0")?,
            og = oldest("group_incoming", "WHERE acknowledged=0")?,
            oa = oldest("abandoned_deliveries", "")?,
            slots = count("SELECT count(*) FROM prekeys")?,
            pubs = scalar("SELECT count(*) FROM prekey_publications WHERE retire_at IS NULL"),
            inits = scalar("SELECT count(*) FROM initiations"),
            ob = count("SELECT count(*) FROM outbound_backoff")?,
            unknown = scalar("SELECT count(*) FROM outbound_backoff WHERE since>0"),
            ib = count("SELECT count(*) FROM send_intent_backoff")?,
            peers = count("SELECT count(*) FROM peers")?,
            sessions = count("SELECT count(*) FROM sessions")?,
            retired = count("SELECT count(*) FROM sessions WHERE retired=1")?,
            perpeer = scalar("SELECT coalesce(max(n),0) FROM (SELECT count(*) n FROM sessions WHERE retired=0 AND peer IS NOT NULL GROUP BY peer)"),
            unbound = scalar("SELECT count(*) FROM sessions WHERE retired=0 AND peer IS NULL"),
            inbox = scalar("SELECT count(*) FROM inbox"),
            deliveries = scalar("SELECT count(*) FROM deliveries"),
            retryin = count("SELECT count(*) FROM retry_incoming")?,
            unfinished = scalar("SELECT count(*) FROM retry_requests WHERE finished=0"),
            retiredreq = scalar("SELECT count(*) FROM retired_retry_requests"),
            pendingout = scalar("SELECT count(*) FROM retry_outbox WHERE complete=0"),
            retiredout = scalar("SELECT count(*) FROM retired_retry_outbox"),
            c0 = outgoing_gc.0,
            r0 = outgoing_gc.1,
            c1 = accepted_gc.0,
            r1 = accepted_gc.1,
        );
        // One line per session with packets waiting: which peer, how many, and why
        // it is not draining. Identifiers are truncated; nothing said is included.
        let mut lines = Vec::new();
        {
            let own_account = self
                .db
                .unchecked_transaction()
                .ok()
                .and_then(|tx| crate::peers::own(&tx, &self.key).ok())
                .and_then(|own| crate::peers::parse(&own).ok())
                .map(|own| crate::event::account(&own.binding));
            let mut stmt = self.db.prepare(
                "SELECT o.session, s.peer, count(*), s.retired, \
                 (SELECT until FROM outbound_backoff b WHERE b.session=o.session), \
                 (SELECT since FROM outbound_backoff b WHERE b.session=o.session), \
                 EXISTS(SELECT 1 FROM initiations i WHERE i.session=o.session) \
                 FROM outbox o JOIN sessions s ON s.id=o.session WHERE o.packet IS NOT NULL \
                 GROUP BY o.session ORDER BY count(*) DESC LIMIT 8",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, Option<Vec<u8>>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })?;
            for row in rows {
                let (session, peer, pending, retired, until, since, initiating) = row?;
                let peer_note = match peer.as_deref().and_then(|p| <[u8; 32]>::try_from(p).ok()) {
                    Some(id) => match crate::peers::known(&self.db, &self.key, &id) {
                        Ok(known) => format!(
                            "peer {} trusted={} blocked={} changed={} replaced={} revoked={} own-account={}",
                            &transport::hex(&id)[..8],
                            known.trusted,
                            known.blocked,
                            known.changed_fingerprint.is_some(),
                            known.replaced_by.is_some(),
                            known.revoked,
                            own_account.as_ref().is_some_and(|a| *a == crate::event::account(&known.binding)),
                        ),
                        Err(_) => format!("peer {} (unreadable)", &transport::hex(&id)[..8]),
                    },
                    None => "no peer".to_string(),
                };
                lines.push(format!(
                    "  session {} pending {} retired={} initiating={} backoff-until={} unknown-since={} {}",
                    &transport::hex(&session)[..8],
                    pending,
                    retired,
                    initiating,
                    until.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                    since.filter(|v| *v > 0).map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                    peer_note
                ));
            }
        }
        let report = if lines.is_empty() {
            report
        } else {
            format!("{report}\nwaiting sessions:\n{}", lines.join("\n"))
        };
        Ok(json!({ "report": report }))
    }
    pub(super) fn mobile_storage(&self) -> Result<Value, Error> {
        let pages: i64 = self.db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let page_size: i64 = self.db.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let allocated = u64::try_from(pages.checked_mul(page_size).ok_or(Error::Limit)?)
            .map_err(|_| Error::InvalidStore)?;
        let (media, media_used, budget) = self.mobile_cache()?.storage_usage()?;
        let recovery = match self.history_recovery_progress() {
            Ok(value) => {
                let restoring = matches!(
                    self.recovery_status()?.pending,
                    Some((recovery::Operation::Import, _))
                ) || self.recovery_competition()?.is_some();
                json!({"enabled":true,"restoring":restoring,"last":value.last_checkpoint_at,"pending":value.unprotected_records,"records":value.committed_records,"days":self.recovery_policy()?.history_days})
            }
            Err(Error::Unprepared | Error::NotFound) => json!({"enabled":false}),
            Err(error) => return Err(error),
        };
        Ok(
            json!({"database":allocated,"media":media,"media_used":media_used,"budget":budget,"recovery":recovery}),
        )
    }
}

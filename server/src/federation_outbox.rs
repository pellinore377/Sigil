use crate::{
    federation_auth as auth, federation_config,
    prekeys::{active, authorize},
    push_config::{sql, unsigned},
    store::{Store, StoreError},
    with_store, AppState,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    federation::{Outbound, OutboundState, Queue, Receipt, Submit},
};
use zeroize::Zeroizing;
pub(crate) const METADATA: u64 = 2048;
pub(crate) const GLOBAL_BUDGET: u64 = 1024 * 1024 * 1024;
pub(crate) const PENDING_PER_PEER: u64 = 64;
pub(crate) const MIGRATION:&str="
CREATE TABLE federation_outbox(sender TEXT NOT NULL REFERENCES devices(id),message_id TEXT NOT NULL,destination TEXT NOT NULL REFERENCES federation_peers(server),body TEXT,request_hash TEXT NOT NULL,expires_at INTEGER NOT NULL,state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 5),remote_sequence INTEGER,due_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,lease TEXT,lease_until INTEGER NOT NULL DEFAULT 0,error TEXT,PRIMARY KEY(sender,message_id));
CREATE INDEX federation_outbox_due ON federation_outbox(due_at,sender,message_id) WHERE state=0;
CREATE INDEX federation_outbox_expiry ON federation_outbox(expires_at,sender,message_id) WHERE state=0;
CREATE INDEX federation_outbox_peer ON federation_outbox(destination,sender,message_id) WHERE state=0;
ALTER TABLE federation_admission ADD COLUMN egress_bytes INTEGER NOT NULL DEFAULT 0 CHECK(egress_bytes>=0);
ALTER TABLE federation_admission ADD COLUMN delivery_not_before INTEGER NOT NULL DEFAULT 0;
ALTER TABLE federation_usage ADD COLUMN egress_bytes INTEGER NOT NULL DEFAULT 0 CHECK(egress_bytes>=0);
";
fn decode_state(value: u8) -> rusqlite::Result<OutboundState> {
    match value {
        0 => Ok(OutboundState::Pending),
        1 => Ok(OutboundState::Accepted),
        2 => Ok(OutboundState::Rejected),
        3 => Ok(OutboundState::Expired),
        4 => Ok(OutboundState::Revoked),
        5 => Ok(OutboundState::Restored),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn read(db: &Connection, sender: &str, id: &str) -> Result<Option<Outbound>, StoreError> {
    Ok(db.query_row("SELECT destination,request_hash,expires_at,state,remote_sequence,due_at,attempts,error FROM federation_outbox WHERE sender=?1 AND message_id=?2",(sender,id),|r|{
        let hash:String=r.get(1)?;let expires_at=unsigned(r,2)?;let sequence:Option<i64>=r.get(4)?;
        Ok(Outbound{message_id:id.into(),destination:r.get(0)?,request_hash:hash.clone(),expires_at,state:decode_state(r.get(3)?)?,receipt:sequence.map(|sequence|Receipt{request_hash:hash,sequence,expires_at}),not_before:unsigned(r,5)?,attempts:r.get(6)?,error:r.get(7)?})
    }).optional()?)
}
impl Store {
    pub fn queue_federated_message(
        &mut self,
        credential: &str,
        request: Queue,
        now: u64,
    ) -> Result<Outbound, StoreError> {
        if !sigil_protocol::valid_server_name(&request.destination) {
            return Err(StoreError::Invalid("invalid federation destination"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sender = authorize(&tx, credential, now)?;
        let account: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [&sender],
            |r| r.get(0),
        )?;
        let submit = Submit {
            sender_account: account,
            sender_device: sender.clone(),
            recipient_device: request.recipient_device,
            message_id: request.message_id,
            payload: request.payload,
            expires_at: request.expires_at,
        };
        crate::federation_mailbox::validate(&submit, now)?;
        let body =
            Zeroizing::new(serde_json::to_string(&submit).map_err(|_| StoreError::InvalidData)?);
        let hash = auth::hex(&Sha256::digest(body.as_bytes()));
        if let Some(previous) = read(&tx, &sender, &submit.message_id)? {
            if previous.request_hash != hash || previous.destination != request.destination {
                return Err(StoreError::AlreadyExists);
            }
            return Ok(previous);
        }
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM mailbox WHERE sender=?1 AND message_id=?2)",
            (&sender, &submit.message_id),
            |r| r.get::<_, bool>(0),
        )? {
            return Err(StoreError::AlreadyExists);
        }
        let config = federation_config::read(&tx)?;
        let peer =
            federation_config::peer(&tx, &request.destination)?.ok_or(StoreError::Forbidden)?;
        if !config.enabled
            || !peer.allowed
            || config.server.as_deref() == Some(request.destination.as_str())
        {
            return Err(StoreError::Forbidden);
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM federation_outbox WHERE sender=?1 AND state=0",
            [&sender],
            |r| r.get(0),
        )?;
        let per_peer: i64 = tx.query_row(
            "SELECT count(*) FROM federation_outbox WHERE destination=?1 AND state=0",
            [&request.destination],
            |r| r.get(0),
        )?;
        let used:u64=tx.query_row("SELECT nonce_bytes+ingress_bytes+egress_bytes FROM federation_admission WHERE server=?1",[&request.destination],|r|unsigned(r,0))?;
        let global: u64 = tx.query_row(
            "SELECT egress_bytes FROM federation_usage WHERE id=1",
            [],
            |r| unsigned(r, 0),
        )?;
        let bytes = METADATA + body.len() as u64;
        if count >= 16
            || per_peer >= PENDING_PER_PEER as i64
            || used.saturating_add(bytes) > config.peer_quota_bytes
            || global.saturating_add(bytes) > GLOBAL_BUDGET
        {
            return Err(StoreError::Busy);
        }
        crate::storage_budget::for_device(&tx, &sender, bytes, now)?;
        // The account reserves body bytes too; terminal transitions release only
        // those bytes. Its immutable retry identity stays charged.
        tx.execute(
            "UPDATE federation_admission SET egress_bytes=egress_bytes+?2 WHERE server=?1",
            (&request.destination, sql(bytes)?),
        )?;
        tx.execute(
            "UPDATE federation_usage SET egress_bytes=egress_bytes+?1 WHERE id=1",
            [sql(bytes)?],
        )?;
        tx.execute("INSERT INTO federation_outbox(sender,message_id,destination,body,request_hash,expires_at,state,due_at) VALUES(?1,?2,?3,?4,?5,?6,0,?7)",(&sender,&submit.message_id,&request.destination,body.as_str(),hash,sql(submit.expires_at)?,sql(now)?))?;
        let result = read(&tx, &sender, &submit.message_id)?.ok_or(StoreError::InvalidData)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn federated_outbound(
        &mut self,
        credential: &str,
        id: &str,
        now: u64,
    ) -> Result<Outbound, StoreError> {
        if !valid_credential(id) {
            return Err(StoreError::Invalid("invalid message identifier"));
        }
        let tx = self.0.transaction()?;
        let sender = authorize(&tx, credential, now)?;
        let result = read(&tx, &sender, id)?.ok_or(StoreError::NotFound)?;
        tx.commit()?;
        Ok(result)
    }
}
pub(crate) fn terminal(
    tx: &Transaction<'_>,
    sender: &str,
    id: &str,
    state: u8,
    sequence: Option<i64>,
    error: Option<&str>,
) -> Result<(), StoreError> {
    let row:Option<(String,u64)>=tx.query_row("SELECT destination,length(body) FROM federation_outbox WHERE sender=?1 AND message_id=?2 AND state=0 AND body IS NOT NULL",(sender,id),|r|Ok((r.get(0)?,unsigned(r,1)?))).optional()?;
    let Some((destination, bytes)) = row else {
        return Err(StoreError::InvalidData);
    };
    if tx.execute("UPDATE federation_admission SET egress_bytes=egress_bytes-?2 WHERE server=?1 AND egress_bytes>=?2",(&destination,sql(bytes)?))?!=1{return Err(StoreError::InvalidData)}
    if tx.execute(
        "UPDATE federation_usage SET egress_bytes=egress_bytes-?1 WHERE id=1 AND egress_bytes>=?1",
        [sql(bytes)?],
    )? != 1
    {
        return Err(StoreError::InvalidData);
    }
    if tx.execute("UPDATE retained_storage SET bytes=bytes-?2 WHERE account_id=(SELECT account_id FROM devices WHERE id=?1) AND bytes>=?2",(sender,sql(bytes)?))?!=1{return Err(StoreError::InvalidData)}
    tx.execute("UPDATE federation_outbox SET body=NULL,state=?3,remote_sequence=?4,lease=NULL,lease_until=0,error=?5 WHERE sender=?1 AND message_id=?2",(sender,id,state,sequence,error))?;
    Ok(())
}
struct Job {
    sender: String,
    message: Outbound,
    body: Zeroizing<String>,
    lease: String,
    started: u64,
    config: federation_config::Stored,
    peer: federation_config::Peer,
}
enum Outcome {
    Accepted(Receipt),
    Rejected(&'static str),
    Retry {
        not_before: u64,
        error: &'static str,
        peer_backoff: bool,
    },
}
impl Store {
    fn claim_federation_delivery(&mut self, now: u64) -> Result<Option<Job>, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let config = federation_config::read(&tx)?;
        if !config.enabled {
            return Ok(None);
        }
        let candidates:Vec<(String,String)>=tx.prepare("SELECT j.sender,j.message_id FROM federation_outbox j JOIN federation_peers p ON p.server=j.destination JOIN federation_admission a ON a.server=p.server WHERE j.state=0 AND j.due_at<=?1 AND j.lease_until<=?1 AND p.allowed=1 AND p.error IS NULL AND p.checked_at>0 AND p.checked_at<=?1 AND p.checked_at>=?2 AND a.delivery_not_before<=?1 ORDER BY j.due_at,j.sender,j.message_id LIMIT 16")?.query_map((sql(now)?,sql(now.saturating_sub(3600))?),|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<Result<_,_>>()?;
        for (sender, id) in candidates {
            let message = read(&tx, &sender, &id)?.ok_or(StoreError::InvalidData)?;
            if message.expires_at <= now {
                terminal(&tx, &sender, &id, 3, None, Some("expired"))?;
                continue;
            }
            if !active(&tx, &sender)? {
                terminal(&tx, &sender, &id, 4, None, Some("sender_revoked"))?;
                continue;
            }
            let body:String=tx.query_row("SELECT CASE WHEN length(body)<=147456 THEN body END FROM federation_outbox WHERE sender=?1 AND message_id=?2",(&sender,&id),|r|r.get(0))?;
            let body = Zeroizing::new(body);
            let submit: Submit =
                serde_json::from_str(&body).map_err(|_| StoreError::InvalidData)?;
            crate::federation_mailbox::validate(&submit, now)
                .map_err(|_| StoreError::InvalidData)?;
            let account: String = tx.query_row(
                "SELECT account_id FROM devices WHERE id=?1",
                [&sender],
                |r| r.get(0),
            )?;
            if submit.sender_device != sender
                || submit.sender_account != account
                || submit.message_id != id
                || submit.expires_at != message.expires_at
                || auth::hex(&Sha256::digest(body.as_bytes())) != message.request_hash
                || serde_json::to_string(&submit).map_err(|_| StoreError::InvalidData)?
                    != body.as_str()
            {
                return Err(StoreError::InvalidData);
            }
            let peer = federation_config::peer(&tx, &message.destination)?
                .ok_or(StoreError::InvalidData)?;
            if peer.pinned.is_none() {
                return Err(StoreError::InvalidData);
            }
            let lease = crate::auth::random_secret().map_err(|_| StoreError::InvalidData)?;
            tx.execute("UPDATE federation_outbox SET lease=?3,lease_until=?4 WHERE sender=?1 AND message_id=?2",(&sender,&id,&lease,sql(now.checked_add(45).ok_or(StoreError::InvalidData)?)?))?;
            tx.commit()?;
            return Ok(Some(Job {
                sender,
                message,
                body,
                lease,
                started: now,
                config,
                peer,
            }));
        }
        tx.commit()?;
        Ok(None)
    }
    fn finish_federation_delivery(
        &mut self,
        job: &Job,
        outcome: Outcome,
        now: u64,
    ) -> Result<bool, StoreError> {
        if now < job.started {
            return Err(StoreError::InvalidData);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM federation_outbox WHERE sender=?1 AND message_id=?2 AND state=0 AND lease=?3)",(&job.sender,&job.message.message_id,&job.lease),|r|r.get(0))?;
        if !current {
            return Ok(false);
        }
        if job.message.expires_at <= now {
            terminal(
                &tx,
                &job.sender,
                &job.message.message_id,
                3,
                None,
                Some("expired"),
            )?;
            tx.commit()?;
            return Ok(true);
        }
        if !active(&tx, &job.sender)? {
            terminal(
                &tx,
                &job.sender,
                &job.message.message_id,
                4,
                None,
                Some("sender_revoked"),
            )?;
            tx.commit()?;
            return Ok(true);
        }
        let config = federation_config::read(&tx)?;
        let peer =
            federation_config::peer(&tx, &job.peer.server)?.ok_or(StoreError::InvalidData)?;
        if config.revision != job.config.revision
            || !config.enabled
            || peer.revision != job.peer.revision
            || !peer.allowed
            || peer.error.is_some()
            || peer.pinned != job.peer.pinned
        {
            tx.execute("UPDATE federation_outbox SET lease=NULL,lease_until=0,due_at=max(due_at,?3) WHERE sender=?1 AND message_id=?2",(&job.sender,&job.message.message_id,sql(now)?))?;
            tx.commit()?;
            return Ok(false);
        }
        match outcome {
            Outcome::Accepted(receipt) => {
                if receipt.request_hash != job.message.request_hash
                    || receipt.expires_at != job.message.expires_at
                    || receipt.sequence <= 0
                {
                    return Err(StoreError::InvalidData);
                }
                terminal(
                    &tx,
                    &job.sender,
                    &job.message.message_id,
                    1,
                    Some(receipt.sequence),
                    None,
                )?;
            }
            Outcome::Rejected(reason) => terminal(
                &tx,
                &job.sender,
                &job.message.message_id,
                2,
                None,
                Some(reason),
            )?,
            Outcome::Retry {
                not_before,
                error,
                peer_backoff,
            } => {
                let delay = (5u64 << job.message.attempts.min(10)).min(300);
                let mut random = [0; 2];
                getrandom::fill(&mut random).map_err(|_| StoreError::InvalidData)?;
                let jitter = u16::from_be_bytes(random) as u64 % (delay / 4 + 1);
                let due = now
                    .saturating_add(delay + jitter)
                    .max(not_before)
                    .min(i64::MAX as u64);
                tx.execute("UPDATE federation_outbox SET lease=NULL,lease_until=0,due_at=?3,attempts=?4,error=?5 WHERE sender=?1 AND message_id=?2",(&job.sender,&job.message.message_id,sql(due)?,job.message.attempts.saturating_add(1).min(31),error))?;
                if peer_backoff {
                    tx.execute("UPDATE federation_admission SET delivery_not_before=max(delivery_not_before,?2) WHERE server=?1",(&job.peer.server,sql(due)?))?;
                }
            }
        }
        tx.commit()?;
        Ok(true)
    }
}
impl Job {
    fn deliver(&self) -> Outcome {
        self.deliver_with(crate::enrollment::now, |request| {
            self.config.policy.federation(request)
        })
    }
    fn deliver_with(
        &self,
        clock: impl Fn() -> Result<u64, StoreError>,
        mut send: impl FnMut(
            ureq::http::Request<&[u8]>,
        ) -> Result<crate::egress::Response, crate::egress::Error>,
    ) -> Outcome {
        let retry = |error| Outcome::Retry {
            not_before: 0,
            error,
            peer_backoff: false,
        };
        let Ok(now) = clock() else {
            return retry("clock_unavailable");
        };
        if now < self.started || now >= self.message.expires_at {
            return retry("clock_or_expiry_changed");
        }
        let Some(key) = &self.config.key else {
            return retry("key_unavailable");
        };
        let Some(origin) = self.config.server.as_deref() else {
            return retry("key_unavailable");
        };
        let request = auth::Request {
            origin,
            destination: &self.peer.server,
            path: sigil_protocol::federation::DELIVER_PATH,
            body: self.body.as_bytes(),
        };
        let mut nonce = [0; 32];
        if getrandom::fill(&mut nonce).is_err() {
            return retry("random_unavailable");
        }
        let Ok(headers) = auth::sign(key, &request, now, nonce) else {
            return retry("signature_unavailable");
        };
        let url = format!(
            "https://{}:{}{}",
            self.peer.server,
            self.peer.port,
            sigil_protocol::federation::DELIVER_PATH
        );
        let Ok(mut request) = ureq::http::Request::post(url).body(self.body.as_bytes()) else {
            return retry("request_unavailable");
        };
        *request.headers_mut() = headers;
        let response = match send(request) {
            Ok(v) => v,
            Err(_) => return retry("transport_failed"),
        };
        let Ok(completed) = clock() else {
            return retry("clock_unavailable");
        };
        if response.status == 202 {
            if response.content_type.as_deref() != Some("application/json") {
                return retry("invalid_receipt");
            }
            let receipt: Receipt = match serde_json::from_slice(&response.body) {
                Ok(v) => v,
                Err(_) => return retry("invalid_receipt"),
            };
            if receipt.request_hash != self.message.request_hash
                || receipt.expires_at != self.message.expires_at
                || receipt.sequence <= 0
            {
                return retry("invalid_receipt");
            }
            return Outcome::Accepted(receipt);
        }
        match response.status {
            400 | 404 | 405 | 409 | 413 | 422 => Outcome::Rejected("remote_rejected"),
            status => Outcome::Retry {
                not_before: crate::push_provider::retry_after(
                    response.retry_after.as_deref(),
                    completed,
                )
                .unwrap_or(completed)
                .max(completed.saturating_add(if status == 429 { 60 } else { 5 })),
                error: if matches!(status, 401 | 403) {
                    "remote_unauthorized"
                } else {
                    "remote_unavailable"
                },
                peer_backoff: status == 429 || status == 503,
            },
        }
    }
}
pub(crate) fn cleanup(tx: &Transaction<'_>, now: u64) -> Result<usize, StoreError> {
    let rows:Vec<(String,String,bool)>=tx.prepare("SELECT j.sender,j.message_id,j.expires_at<=?1 FROM federation_outbox j JOIN devices d ON d.id=j.sender JOIN accounts a ON a.id=d.account_id WHERE j.state=0 AND (j.expires_at<=?1 OR d.revoked=1 OR a.disabled=1) ORDER BY j.expires_at,j.sender,j.message_id LIMIT 64")?.query_map([sql(now)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<Result<_,_>>()?;
    for (sender, id, expired) in &rows {
        terminal(
            tx,
            sender,
            id,
            if *expired { 3 } else { 4 },
            None,
            Some(if *expired {
                "expired"
            } else {
                "sender_revoked"
            }),
        )?;
    }
    Ok(rows.len())
}
pub(crate) fn reset_after_restore(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch("UPDATE federation_outbox SET state=5,body=NULL,lease=NULL,lease_until=0,error='restored_no_replay' WHERE state=0;UPDATE federation_admission SET delivery_not_before=0,egress_bytes=(SELECT coalesce(sum(2048+coalesce(length(body),0)),0) FROM federation_outbox WHERE destination=federation_admission.server);UPDATE federation_usage SET egress_bytes=(SELECT coalesce(sum(egress_bytes),0) FROM federation_admission);")?;
    Ok(())
}
pub(crate) async fn run(state: AppState) {
    tokio::join!(
        worker(state.clone()),
        worker(state.clone()),
        worker(state.clone()),
        worker(state)
    );
}
async fn worker(state: AppState) {
    loop {
        let wake = state.federation_wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let job = with_store(state.clone(), |s| {
            s.claim_federation_delivery(crate::enrollment::now()?)
        })
        .await;
        let Ok(Some(job)) = job else {
            tokio::select! {
                _ = wake => (),
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => (),
            }
            continue;
        };
        let result = tokio::task::spawn_blocking(move || {
            let outcome = job.deliver();
            (job, outcome)
        })
        .await;
        if let Ok((job, outcome)) = result {
            let _ = with_store(state.clone(), move |s| {
                s.finish_federation_delivery(&job, outcome, crate::enrollment::now()?)
            })
            .await;
        }
    }
}

#[cfg(test)]
#[path = "federation_outbox_tests.rs"]
pub(crate) mod tests;

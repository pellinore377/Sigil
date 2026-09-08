use crate::{
    auth::random_secret,
    egress,
    enrollment::now,
    push, push_config,
    push_provider::{self, AccessToken, Fcm, Outcome, Vapid},
    store::{Store, StoreError},
    with_store, AppState,
};
use push_config::{optional_unsigned, sql, unsigned};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sigil_protocol::push::{Payload, State, Target};
use std::time::Duration;
use zeroize::Zeroizing;

enum Hint {
    Wake,
    Challenge {
        channel: [u8; 32],
        proof: Zeroizing<[u8; 32]>,
    },
}
struct Job {
    device: String,
    revision: u64,
    lease: String,
    through: i64,
    expires: u64,
    attempts: u32,
    started: u64,
    config: push_config::Stored,
    target: Target,
    hint: Hint,
}
impl Job {
    fn payload(&self) -> Payload<'_> {
        match &self.hint {
            Hint::Wake => Payload::Wake,
            Hint::Challenge { channel, proof } => Payload::Challenge { channel, proof },
        }
    }
    fn fcm(&self) -> bool {
        matches!(self.target, Target::Fcm { .. })
    }
}
#[derive(Clone, Copy)]
struct Delivery {
    outcome: Outcome,
    global_backoff: bool,
}
impl Delivery {
    fn retry(now: u64, delay: u64, global_backoff: bool) -> Self {
        Self {
            outcome: Outcome::Retry {
                not_before: now.saturating_add(delay),
                refresh_auth: false,
            },
            global_backoff,
        }
    }
}
impl Store {
    fn claim_push(&mut self, now: u64) -> Result<Option<Job>, StoreError> {
        let lease_until = push::deadline(now, 45)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidates:Vec<String>=tx.prepare("SELECT j.device FROM push_jobs j JOIN push_channels c ON c.device=j.device JOIN push_configuration p ON p.id=1 WHERE j.due_at<=?1 AND j.lease_until<=?1 AND (c.provider!=0 OR p.fcm_not_before<=?1) ORDER BY j.due_at,j.device LIMIT 16")?
            .query_map([sql(now)?],|r|r.get(0))?.collect::<Result<_,_>>()?;
        if candidates.is_empty() {
            return Ok(None);
        }
        let config = push_config::read(&tx)?;
        for device in candidates {
            let Some(row) = push::current(&tx, &config, &device, now)? else {
                continue;
            };
            let existing:Option<(u64,i64,u64,u32)>=tx.query_row("SELECT revision,through_sequence,expires_at,attempts FROM push_jobs WHERE device=?1",[&device],|r|Ok((unsigned(r,0)?,r.get(1)?,unsigned(r,2)?,r.get(3)?))).optional()?;
            let Some((revision, mut through, mut expires, attempts)) = existing else {
                continue;
            };
            if revision != row.revision
                || expires <= now
                || !matches!(row.state, State::Pending | State::Active)
            {
                tx.execute("DELETE FROM push_jobs WHERE device=?1", [&device])?;
                continue;
            }
            if row.not_before > now {
                tx.execute(
                    "UPDATE push_jobs SET due_at=?2 WHERE device=?1",
                    (&device, sql(row.not_before)?),
                )?;
                continue;
            }
            let target: Target =
                serde_json::from_str(row.target.as_ref().ok_or(StoreError::InvalidData)?)
                    .map_err(|_| StoreError::InvalidData)?;
            let hint = if row.state == State::Pending {
                Hint::Challenge {
                    channel: push::bytes(row.channel.as_ref().ok_or(StoreError::InvalidData)?)?,
                    proof: Zeroizing::new(push::bytes(
                        row.proof.as_ref().ok_or(StoreError::InvalidData)?,
                    )?),
                }
            } else {
                let pending:(Option<i64>,Option<u64>)=tx.query_row("SELECT max(sequence),max(expires_at) FROM mailbox WHERE recipient=?1 AND payload IS NOT NULL AND expires_at>?2",(&device,sql(now)?),|r|Ok((r.get(0)?,optional_unsigned(r,1)?)))?;
                let (Some(sequence), Some(expiry)) = pending else {
                    tx.execute("DELETE FROM push_jobs WHERE device=?1", [&device])?;
                    continue;
                };
                through = through.max(sequence);
                expires = expiry.min(row.expires.ok_or(StoreError::InvalidData)?);
                Hint::Wake
            };
            let lease = random_secret().map_err(|_| StoreError::InvalidData)?;
            tx.execute("UPDATE push_jobs SET lease=?2,lease_until=?3,through_sequence=?4,expires_at=?5 WHERE device=?1",(&device,&lease,sql(lease_until)?,through,sql(expires)?))?;
            tx.commit()?;
            return Ok(Some(Job {
                device,
                revision,
                lease,
                through,
                expires,
                attempts,
                started: now,
                config,
                target,
                hint,
            }));
        }
        tx.commit()?;
        Ok(None)
    }
    fn finish_push(&mut self, job: &Job, delivery: Delivery, now: u64) -> Result<bool, StoreError> {
        if now < job.started || now > i64::MAX as u64 {
            return Err(StoreError::InvalidData);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let through:Option<i64>=tx.query_row("SELECT through_sequence FROM push_jobs WHERE device=?1 AND revision=?2 AND lease=?3",(&job.device,sql(job.revision)?,&job.lease),|r|r.get(0)).optional()?;
        let Some(through) = through else {
            return Ok(false);
        };
        let config = push_config::read(&tx)?;
        let current = push::current(&tx, &config, &job.device, now)?;
        if current
            .as_ref()
            .is_none_or(|c| !matches!(c.state, State::Pending | State::Active))
        {
            tx.commit()?;
            return Ok(false);
        }
        if config.revision != job.config.revision {
            tx.execute("UPDATE push_jobs SET lease=NULL,lease_until=0,due_at=max(due_at,?2) WHERE device=?1",(&job.device,sql(now)?))?;
            tx.commit()?;
            return Ok(false);
        }
        match delivery.outcome {
            Outcome::InvalidRegistration => push::clear(&tx, &job.device, State::Invalid)?,
            Outcome::Accepted => {
                if matches!(job.hint, Hint::Wake) && through <= job.through {
                    tx.execute("DELETE FROM push_jobs WHERE device=?1", [&job.device])?;
                } else {
                    let due = push::deadline(
                        now,
                        if matches!(job.hint, Hint::Challenge { .. }) {
                            30
                        } else {
                            1
                        },
                    )?;
                    tx.execute("UPDATE push_jobs SET lease=NULL,lease_until=0,due_at=?2,attempts=0 WHERE device=?1",(&job.device,sql(due)?))?;
                }
            }
            Outcome::Retry { not_before, .. } => {
                let base = if delivery.global_backoff && job.fcm() {
                    60u64
                } else {
                    5
                };
                let delay = (base << job.attempts.min(10)).min(3600);
                let mut random = [0u8; 2];
                getrandom::fill(&mut random).map_err(|_| StoreError::InvalidData)?;
                let jitter = u16::from_be_bytes(random) as u64 % (delay / 4 + 1);
                let due = now
                    .saturating_add(delay + jitter)
                    .max(not_before)
                    .min(i64::MAX as u64);
                tx.execute(
                    "UPDATE push_channels SET not_before=max(not_before,?2) WHERE device=?1",
                    (&job.device, sql(due)?),
                )?;
                if delivery.global_backoff && job.fcm() {
                    tx.execute("UPDATE push_configuration SET fcm_not_before=max(fcm_not_before,?1) WHERE id=1",[sql(due)?])?;
                }
                tx.execute("UPDATE push_jobs SET lease=NULL,lease_until=0,due_at=?2,attempts=?3 WHERE device=?1",(&job.device,sql(due)?,job.attempts.saturating_add(1).min(31)))?;
            }
        }
        tx.commit()?;
        Ok(true)
    }
}

#[derive(Default)]
struct Provider {
    token: Option<(u64, AccessToken)>,
}
impl Provider {
    fn deliver(&mut self, job: &Job) -> Delivery {
        let policy = match egress::Policy::new(job.config.settings.exceptions.clone()) {
            Ok(v) => v,
            Err(_) => return Delivery::retry(job.started, 300, job.fcm()),
        };
        self.deliver_with(job, now, |google, request| {
            if google {
                egress::Policy::default().send(request)
            } else {
                policy.send(request)
            }
        })
    }
    fn deliver_with(
        &mut self,
        job: &Job,
        clock: impl Fn() -> Result<u64, StoreError>,
        mut send: impl FnMut(
            bool,
            ureq::http::Request<&[u8]>,
        ) -> Result<egress::Response, egress::Error>,
    ) -> Delivery {
        let Ok(start) = clock() else {
            return Delivery::retry(job.started, 5, job.fcm());
        };
        if start < job.started || start >= job.expires {
            return Delivery::retry(start, 5, false);
        }
        let response = if job.fcm() {
            let Some(credentials) = job.config.settings.fcm.as_ref() else {
                return Delivery::retry(start, 300, true);
            };
            let Ok(fcm) = Fcm::new(credentials) else {
                return Delivery::retry(start, 300, true);
            };
            if self.token.as_ref().is_none_or(|(revision, token)| {
                *revision != job.config.revision || !token.valid_at(start)
            }) {
                self.token = None;
                let Ok(request) = fcm.token_request(start) else {
                    return Delivery::retry(start, 300, true);
                };
                let (parts, body) = request.into_parts();
                let response = match send(
                    true,
                    ureq::http::Request::from_parts(parts, body.as_slice()),
                ) {
                    Ok(r) => r,
                    Err(_) => return Delivery::retry(start, 5, true),
                };
                match AccessToken::from_response(&response, start) {
                    Ok(token) => self.token = Some((job.config.revision, token)),
                    Err(_) => {
                        let outcome =
                            push_provider::classify(&response, true, clock().unwrap_or(start));
                        return if matches!(outcome, Outcome::Retry { .. }) {
                            Delivery {
                                outcome,
                                global_backoff: true,
                            }
                        } else {
                            Delivery::retry(start, 300, true)
                        };
                    }
                }
            }
            let Ok(current) = clock() else {
                return Delivery::retry(start, 5, true);
            };
            if current < start || current >= job.expires {
                return Delivery::retry(current, 5, false);
            }
            let Some((_, token)) = &self.token else {
                return Delivery::retry(current, 300, true);
            };
            let Ok(request) = fcm.request(
                token,
                &job.target,
                &job.payload(),
                current,
                (job.expires - current).min(push_provider::MAX_TTL),
            ) else {
                return Delivery::retry(current, 300, true);
            };
            let (parts, body) = request.into_parts();
            send(
                true,
                ureq::http::Request::from_parts(parts, body.as_slice()),
            )
        } else {
            let (Some(key), Some(contact)) =
                (&job.config.settings.vapid, &job.config.settings.contact)
            else {
                return Delivery::retry(start, 300, false);
            };
            let Ok(vapid) = Vapid::from_pkcs8(key) else {
                return Delivery::retry(start, 300, false);
            };
            let Ok(request) = vapid.request(
                &job.target,
                contact,
                &job.payload(),
                start,
                (job.expires - start).min(push_provider::MAX_TTL),
            ) else {
                return Delivery::retry(start, 300, false);
            };
            let (parts, body) = request.into_parts();
            send(
                false,
                ureq::http::Request::from_parts(parts, body.as_slice()),
            )
        };
        let completed = clock().unwrap_or(start).max(start);
        match response {
            Ok(response) => {
                let outcome = push_provider::classify(&response, job.fcm(), completed);
                if matches!(
                    outcome,
                    Outcome::Retry {
                        refresh_auth: true,
                        ..
                    }
                ) {
                    self.token = None;
                }
                Delivery {
                    outcome,
                    global_backoff: job.fcm() && matches!(response.status, 401 | 429 | 500 | 503),
                }
            }
            Err(_) => Delivery::retry(completed, 5, job.fcm()),
        }
    }
}

pub(crate) fn cleanup(db: &rusqlite::Connection, now: u64) -> Result<usize, StoreError> {
    let devices:Vec<String>=db.prepare("SELECT c.device FROM push_channels c JOIN devices d ON d.id=c.device JOIN accounts a ON a.id=d.account_id JOIN push_configuration p ON p.id=1 WHERE c.target IS NOT NULL AND (c.expires_at<=?1 OR d.revoked=1 OR a.disabled=1 OR (c.provider=0 AND c.generation!=p.fcm_generation) OR (c.provider=1 AND c.generation!=p.unified_generation)) ORDER BY c.expires_at,c.device LIMIT 64")?.query_map([sql(now)?],|r|r.get(0))?.collect::<Result<_,_>>()?;
    let config = if devices.is_empty() {
        None
    } else {
        Some(push_config::read(db)?)
    };
    for device in &devices {
        push::current(
            db,
            config.as_ref().ok_or(StoreError::InvalidData)?,
            device,
            now,
        )?;
    }
    let expired=db.execute("DELETE FROM push_jobs WHERE device IN (SELECT device FROM push_jobs WHERE expires_at<=?1 ORDER BY expires_at,device LIMIT 64)",[sql(now)?])?;
    Ok(devices.len() + expired)
}

async fn worker(state: AppState) {
    let mut provider = Provider::default();
    let mut failed = false;
    loop {
        let claim = with_store(state.clone(), |store| store.claim_push(now()?)).await;
        let job = match claim {
            Ok(Some(job)) => job,
            Ok(None) | Err(StoreError::Busy) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Err(_) => {
                if !failed {
                    eprintln!("Push queue maintenance failed; retrying in background.");
                }
                failed = true;
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let result = tokio::task::spawn_blocking(move || {
            let delivery = provider.deliver(&job);
            (provider, job, delivery)
        })
        .await;
        let Ok((returned, job, delivery)) = result else {
            provider = Provider::default();
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        provider = returned;
        match with_store(state.clone(), move |store| {
            store.finish_push(&job, delivery, now()?)
        })
        .await
        {
            Ok(_) => failed = false,
            Err(_) => {
                if !failed {
                    eprintln!("Push completion persistence failed; the lease will be retried.");
                }
                failed = true;
            }
        }
    }
}
pub(crate) async fn run(state: AppState) {
    tokio::join!(
        worker(state.clone()),
        worker(state.clone()),
        worker(state.clone()),
        worker(state)
    );
}

#[cfg(test)]
#[path = "push_delivery_tests.rs"]
pub(crate) mod tests;

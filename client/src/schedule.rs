//! Durable scheduling for a platform-owned blocking worker, with no hidden timer.
use super::*;

pub(super) const RESERVATION_SECONDS: u64 = 60;
const POLL_SECONDS: u64 = 5;

pub struct ScheduledSync {
    /// None means deferred without running the sync step.
    pub step: Option<SyncStep>,
    pub next_at: u64,
    /// Work results survive a failed final schedule commit; the reservation remains.
    /// Callers must handle this error before resuming automation: the final backoff
    /// (including Retry-After) may not have been persisted.
    pub scheduling_error: Option<Error>,
}

pub(super) struct Schedule {
    pub(super) last: u64,
    pub(super) next: u64,
    pub(super) failures: u8,
    slot: Slot,
}
#[derive(Clone, Copy)]
enum Slot {
    Messaging = 1,
    Recovery = 2,
    Push = 3,
}
impl Slot {
    fn role(self) -> &'static [u8] {
        match self {
            Self::Messaging => b"sync schedule",
            Self::Recovery => b"recovery schedule",
            Self::Push => b"push schedule",
        }
    }
}
pub(super) fn read(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
) -> Result<(Schedule, Option<Vec<u8>>), Error> {
    read_slot(db, key, own, Slot::Messaging)
}
pub(super) fn read_recovery(
    db: &Connection,
    key: &StorageKey,
    scope: &Id,
) -> Result<(Schedule, Option<Vec<u8>>), Error> {
    read_slot(db, key, scope, Slot::Recovery)
}
pub(super) fn read_push(
    db: &Connection,
    key: &StorageKey,
    scope: &Id,
) -> Result<(Schedule, Option<Vec<u8>>), Error> {
    read_slot(db, key, scope, Slot::Push)
}
pub(super) fn nudge_push(
    db: &Transaction<'_>,
    key: &StorageKey,
    scope: &Id,
    now: u64,
) -> Result<(), Error> {
    let (mut state, _) = read_push(db, key, scope)?;
    if now < state.last || now == 0 || now > i64::MAX as u64 - 86400 {
        return Err(Error::Expired);
    }
    if state.failures == 0 && state.next != state.last.saturating_add(RESERVATION_SECONDS) {
        state.last = now;
        state.next = now;
        write(db, key, scope, &state)?;
    }
    Ok(())
}
fn read_slot(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    slot: Slot,
) -> Result<(Schedule, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=53 THEN state END FROM sync_schedule WHERE id=?1",
            [slot as i64],
            |r| r.get(0),
        )
        .optional()?;
    let state = match &sealed {
        None => Schedule {
            last: 0,
            next: 0,
            failures: 0,
            slot,
        },
        Some(bytes) => {
            let bytes = key.open(bytes, &binding(35, own, slot.role()))?;
            if bytes.len() != 17 {
                return Err(Error::InvalidStore);
            }
            let last = u64::from_be_bytes(bytes[..8].try_into().map_err(|_| Error::InvalidStore)?);
            let next =
                u64::from_be_bytes(bytes[8..16].try_into().map_err(|_| Error::InvalidStore)?);
            if last == 0 || next < last || next > i64::MAX as u64 || bytes[16] > 7 {
                return Err(Error::InvalidStore);
            }
            Schedule {
                last,
                next,
                failures: bytes[16],
                slot,
            }
        }
    };
    Ok((state, sealed))
}
pub(super) fn write(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &Schedule,
) -> Result<Vec<u8>, Error> {
    let bytes = [
        state.last.to_be_bytes().as_slice(),
        state.next.to_be_bytes().as_slice(),
        &[state.failures],
    ]
    .concat();
    let sealed = key.seal(&bytes, &binding(35, own, state.slot.role()))?;
    tx.execute(
        "INSERT INTO sync_schedule VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (state.slot as i64, &sealed),
    )?;
    Ok(sealed)
}
pub(super) fn clock() -> Result<u64, Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| Error::Expired)
}
fn network_error(step: &SyncStep) -> Option<&network::Error> {
    let error = match step.failure.as_ref()? {
        SyncFailure::Receive(e)
        | SyncFailure::Calls(e)
        | SyncFailure::Acknowledge(e)
        | SyncFailure::Prekeys(e)
        | SyncFailure::Recovery(e)
        | SyncFailure::Outbound(e)
        | SyncFailure::GroupOutbound(e)
        | SyncFailure::Groups(e)
        | SyncFailure::Invitations(e)
        | SyncFailure::History(e)
        | SyncFailure::SendIntents(e)
        | SyncFailure::RetryControls(e)
        | SyncFailure::Maintenance(e)
        | SyncFailure::Conversations(e)
        | SyncFailure::Structured(e) => Some(e),
        SyncFailure::PrekeyNetwork(i) => step.prekeys.get(*i)?.result.as_ref().err(),
        SyncFailure::CallNetwork(i) => step.calls.get(*i)?.result.as_ref().err(),
        SyncFailure::PrekeySupplyNetwork => step.prekey_supply.as_ref()?.as_ref().err(),
        SyncFailure::RecoveryNetwork(i) => step.retries.get(*i)?.result.as_ref().err(),
        SyncFailure::OutboundNetwork(i) => step.outbound.get(*i)?.result.as_ref().err(),
        SyncFailure::GroupOutboundNetwork(i) => step.group_outbound.get(*i)?.result.as_ref().err(),
        SyncFailure::GroupNetwork(i) => step.groups.get(*i)?.result.as_ref().err(),
        SyncFailure::InvitationNetwork(i) => step.invitations.get(*i)?.result.as_ref().err(),
        SyncFailure::HistoryNetwork(i) => step.history.get(*i)?.result.as_ref().err(),
        SyncFailure::SendIntentNetwork(i) => step.sends.get(*i)?.result.as_ref().err(),
        SyncFailure::RetryControlNetwork(i) => step.retry_controls.get(*i)?.result.as_ref().err(),
    }?;
    if let Error::Network(e) = error {
        Some(e)
    } else {
        None
    }
}
impl ClientStore {
    /// Run only when due. Persists a one-minute reservation before I/O; a crash
    /// leaves that retry window in place. Retry-After is measured from completion,
    /// never from pass start. Stage failures back off 5,10,20,... up to 300 seconds;
    /// successful passes reset to five seconds. Per-item local errors remain visible
    /// in the step and do not throttle unrelated messaging. Platform callers own
    /// wakeups and should serialize workers; an expired reservation is not a lock.
    pub fn sync_due_online(&mut self) -> Result<ScheduledSync, Error> {
        self.sync_with_clock(clock)
    }
    pub fn sync_foreground_online(&mut self) -> Result<ScheduledSync, Error> {
        self.sync_with_poll(clock, 1)
    }

    fn sync_with_clock(
        &mut self,
        clock: impl FnMut() -> Result<u64, Error>,
    ) -> Result<ScheduledSync, Error> {
        self.sync_with_poll(clock, POLL_SECONDS)
    }
    fn sync_with_poll(
        &mut self,
        mut clock: impl FnMut() -> Result<u64, Error>,
        poll: u64,
    ) -> Result<ScheduledSync, Error> {
        let now = clock()?;
        if now == 0 || now > i64::MAX as u64 - RESERVATION_SECONDS {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut state, _) = read(&tx, &self.key, &own)?;
        if now < state.last {
            return Err(Error::Expired);
        }
        if now < state.next {
            return Ok(ScheduledSync {
                step: None,
                next_at: state.next,
                scheduling_error: None,
            });
        }
        state.last = now;
        state.next = now + RESERVATION_SECONDS;
        let reserved = write(&tx, &self.key, &own, &state)?;
        tx.commit()?;
        let step = self.sync_step_online(now);
        let (next_at, scheduling_error) = match clock().and_then(|finished| {
            complete(
                &mut self.db,
                &self.key,
                &own,
                (&state, &reserved),
                finished,
                (step.failure.is_some(), network_error(&step)),
                poll,
            )
        }) {
            Ok(next) => (next, None),
            Err(error) => (state.next, Some(error)),
        };
        Ok(ScheduledSync {
            step: Some(step),
            next_at,
            scheduling_error,
        })
    }

    #[cfg(test)]
    fn finish_schedule(
        &mut self,
        own: &Id,
        reserved: &[u8],
        before: &Schedule,
        now: u64,
        step: &SyncStep,
    ) -> Result<u64, Error> {
        complete(
            &mut self.db,
            &self.key,
            own,
            (before, reserved),
            now,
            (step.failure.is_some(), network_error(step)),
            POLL_SECONDS,
        )
    }
}
/// Shared durable completion for messaging and the independently keyed media
/// cache. A stale worker can extend a newer deadline, never shorten it.
pub(super) fn complete(
    db: &mut Connection,
    key: &StorageKey,
    own: &Id,
    reservation: (&Schedule, &[u8]),
    now: u64,
    outcome: (bool, Option<&network::Error>),
    success_delay: u64,
) -> Result<u64, Error> {
    complete_with(db, key, own, reservation, now, outcome, |_| {
        Ok(success_delay)
    })
}
pub(super) fn complete_with(
    db: &mut Connection,
    key: &StorageKey,
    own: &Id,
    reservation: (&Schedule, &[u8]),
    now: u64,
    outcome: (bool, Option<&network::Error>),
    success_delay: impl FnOnce(&Connection) -> Result<u64, Error>,
) -> Result<u64, Error> {
    let (before, reserved) = reservation;
    if now < before.last || now > i64::MAX as u64 - 86400 {
        return Err(Error::Expired);
    }
    let failures = if outcome.0 {
        (before.failures + 1).min(7)
    } else {
        0
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let delay = if failures == 0 {
        success_delay(&tx)?
    } else {
        (5u64 << (failures - 1)).min(300)
    };
    let retry = match outcome.1 {
        Some(network::Error::Status {
            retry_after_seconds: Some(value),
            ..
        }) => *value,
        _ => 0,
    };
    let mut next = now
        .checked_add(delay.max(retry))
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(Error::Limit)?;
    let (current, sealed) = read_slot(&tx, key, own, before.slot)?;
    if now < current.last {
        return Err(Error::Expired);
    }
    let failures = if sealed.as_deref() != Some(reserved) {
        // A late worker may extend a newer deadline, never shorten it or
        // clear another worker's failure streak/reservation.
        next = next.max(current.next);
        failures.max(current.failures)
    } else {
        failures
    };
    write(
        &tx,
        key,
        own,
        &Schedule {
            last: now,
            next,
            failures,
            slot: before.slot,
        },
    )?;
    tx.commit()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::tests::Fixture;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    fn open(path: &Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    fn setup(
        retry: u64,
    ) -> (
        tempfile::TempDir,
        Fixture,
        ClientStore,
        Arc<AtomicBool>,
        Arc<AtomicUsize>,
        u64,
    ) {
        let (dir, old, invite, now) = crate::connection::tests::setup();
        drop(old);
        let reject = Arc::new(AtomicBool::new(true));
        let hits = Arc::new(AtomicUsize::new(0));
        let (rejected, count) = (reject.clone(), hits.clone());
        let router = sigil_server::router(
            sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
            sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token"))
                .unwrap(),
        )
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let (reject, count) = (rejected.clone(), count.clone());
                async move {
                    if request.uri().path() != sigil_protocol::discovery::PATH {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.uri().path() == "/client/v0/mailbox" && reject.load(Ordering::SeqCst)
                    {
                        return axum::http::Response::builder()
                            .status(429)
                            .header("retry-after", retry.to_string())
                            .body(axum::body::Body::empty())
                            .unwrap();
                    }
                    next.run(request).await
                }
            },
        ));
        let fixture = Fixture::new(router);
        let mut store = open(&dir.path().join("client.db"));
        crate::connection::tests::prepare(&mut store, &fixture, &invite.secret);
        store.enroll_online().unwrap();
        hits.store(0, Ordering::SeqCst);
        (dir, fixture, store, reject, hits, now)
    }
    fn run(store: &mut ClientStore, begin: u64, end: u64) -> Result<ScheduledSync, Error> {
        let mut times = [begin, end].into_iter();
        store.sync_with_clock(|| Ok(times.next().unwrap()))
    }
    #[test]
    fn foreground_polling_preserves_server_backoff_and_only_shortens_success_delay() {
        let (_dir, _fixture, mut store, reject, hits, now) = setup(120);
        let mut times = [now, now + 1].into_iter();
        let failed = store
            .sync_with_poll(|| Ok(times.next().unwrap()), 1)
            .unwrap();
        assert_eq!(failed.next_at, now + 121);
        assert!(store
            .sync_with_poll(|| Ok(now + 120), 1)
            .unwrap()
            .step
            .is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        reject.store(false, Ordering::SeqCst);
        let mut times = [now + 121, now + 122].into_iter();
        let success = store
            .sync_with_poll(|| Ok(times.next().unwrap()), 1)
            .unwrap();
        assert!(success.step.unwrap().failure.is_none());
        assert_eq!(success.next_at, now + 123);
    }
    #[test]
    fn retry_after_starts_at_completion_survives_restart_and_success_resets_backoff() {
        let (dir, _fixture, store, reject, hits, now) = setup(120);
        crate::test_schema::rewind(&store.db, 38);
        drop(store);
        let mut store = open(&dir.path().join("client.db"));
        let first = run(&mut store, now, now + 30).unwrap();
        assert!(first.scheduling_error.is_none());
        assert_eq!(first.next_at, now + 150);
        assert!(matches!(
            first.step.unwrap().failure,
            Some(SyncFailure::Receive(Error::Network(
                network::Error::Status { code: 429, .. }
            )))
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        drop(store);
        let mut store = open(&dir.path().join("client.db"));
        assert!(run(&mut store, now + 149, now + 149)
            .unwrap()
            .step
            .is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        reject.store(false, Ordering::SeqCst);
        let success = run(&mut store, now + 150, now + 151).unwrap();
        assert!(success.step.unwrap().failure.is_none());
        assert!(success.scheduling_error.is_none());
        assert_eq!(success.next_at, now + 156);
        reject.store(true, Ordering::SeqCst);
        assert_eq!(
            run(&mut store, now + 156, now + 156).unwrap().next_at,
            now + 276
        );
        let own = device_fingerprint(&store.own_device_binding().unwrap()).unwrap();
        assert_eq!(read(&store.db, &store.key, &own).unwrap().0.failures, 1);
    }
    #[test]
    fn failed_schedule_writes_preserve_work_and_reservation_and_reject_clock_rollback() {
        let (dir, _fixture, mut store, _reject, hits, now) = setup(0);
        store.db.execute_batch("CREATE TRIGGER fail_reserve BEFORE INSERT ON sync_schedule BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(matches!(run(&mut store, now, now), Err(Error::Storage(_))));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        store.db.execute_batch("DROP TRIGGER fail_reserve; CREATE TRIGGER fail_finish BEFORE UPDATE ON sync_schedule BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let result = run(&mut store, now, now).unwrap();
        assert!(result.step.unwrap().failure.is_some());
        assert!(matches!(result.scheduling_error, Some(Error::Storage(_))));
        assert_eq!(result.next_at, now + RESERVATION_SECONDS);
        store.db.execute_batch("DROP TRIGGER fail_finish;").unwrap();
        drop(store);
        let mut store = open(&dir.path().join("client.db"));
        assert!(run(&mut store, now + 1, now + 1).unwrap().step.is_none());
        assert!(matches!(
            run(&mut store, now - 1, now - 1),
            Err(Error::Expired)
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        store
            .db
            .execute("UPDATE sync_schedule SET state=zeroblob(53)", [])
            .unwrap();
        assert!(run(
            &mut store,
            now + RESERVATION_SECONDS,
            now + RESERVATION_SECONDS
        )
        .is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn exponential_backoff_caps_and_stale_completion_never_shortens_a_newer_deadline() {
        let (_dir, _fixture, mut store, _reject, _hits, mut now) = setup(0);
        for delay in [5, 10, 20, 40, 80, 160, 300, 300] {
            let result = run(&mut store, now, now).unwrap();
            assert!(result.scheduling_error.is_none());
            assert_eq!(result.next_at, now + delay);
            now = result.next_at;
        }
        let own = device_fingerprint(&store.own_device_binding().unwrap()).unwrap();
        let (before, reserved) = read(&store.db, &store.key, &own).unwrap();
        let tx = store.db.transaction().unwrap();
        write(
            &tx,
            &store.key,
            &own,
            &Schedule {
                last: now,
                next: now + 500,
                failures: 7,
                slot: Slot::Messaging,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            store
                .finish_schedule(&own, &reserved.unwrap(), &before, now, &SyncStep::default())
                .unwrap(),
            now + 500
        );
        assert_eq!(read(&store.db, &store.key, &own).unwrap().0.failures, 7);
    }
}

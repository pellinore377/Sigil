use super::*;
use crate::schedule;

#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Idle,
    AwaitingEndpoint,
    AwaitingConfirmation,
    Reconciled,
    Updated(RemoteStatus),
}
pub struct ScheduledPush {
    pub progress: Option<Result<Progress, Error>>,
    pub next_at: u64,
    pub scheduling_error: Option<Error>,
}
enum Action {
    Read,
    Send(Operation),
}
fn delay(state: &Local, now: u64) -> Result<u64, Error> {
    if !state.configured {
        return Ok(3600);
    }
    if state.reconcile || state.pending.is_some() {
        return Ok(1);
    }
    let desired = state.preference.target()?;
    if state.applied != state.generation {
        return Ok(1);
    }
    match &state.status {
        Some(status) if desired.is_none() && status.state == RemoteState::Disabled => Ok(3600),
        Some(status)
            if desired.as_ref() == state.registered.as_ref()
                && status.state == RemoteState::Active =>
        {
            let renew = status
                .expires_at
                .ok_or(Error::InvalidStore)?
                .saturating_sub(86400);
            Ok(renew
                .min(state.checked.saturating_add(86400))
                .saturating_sub(now)
                .max(1))
        }
        Some(status)
            if desired.as_ref() == state.registered.as_ref()
                && status.state == RemoteState::Pending =>
        {
            Ok(status
                .expires_at
                .ok_or(Error::InvalidStore)?
                .min(state.checked.saturating_add(30))
                .saturating_sub(now)
                .max(1))
        }
        _ => Ok(1),
    }
}
fn reconcile_status(state: &mut Local, status: RemoteStatus, now: u64) -> Result<(), Error> {
    if let Some(old) = &state.status {
        if status.revision < old.revision
            || (status.revision == old.revision && status.channel != old.channel)
        {
            return Err(Error::Conflict);
        }
        if status.revision > old.revision {
            state.registered = None;
            state.applied = 0;
        }
    }
    if state.reconcile {
        state.pending = None;
        state.registered = None;
        state.applied = 0;
    }
    state.reconcile = false;
    state.checked = now;
    state.status = Some(status);
    if state
        .status
        .as_ref()
        .is_some_and(|s| s.state == RemoteState::Active)
        && state.registered.as_ref() == state.preference.target()?.as_ref()
    {
        state.applied = state.generation;
    }
    Ok(())
}
impl ClientStore {
    pub fn sync_push_due_online(&mut self) -> Result<ScheduledPush, Error> {
        self.push_with_clock(schedule::clock)
    }
    pub(super) fn push_with_clock(
        &mut self,
        mut clock: impl FnMut() -> Result<u64, Error>,
    ) -> Result<ScheduledPush, Error> {
        let scope = scope(&self.db, &self.key)?;
        let now = clock()?;
        if now == 0 || now > i64::MAX as u64 - 86400 {
            return Err(Error::Expired);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut before, _) = schedule::read_push(&tx, &self.key, &scope)?;
        if now < before.last {
            return Err(Error::Expired);
        }
        if now < before.next {
            return Ok(ScheduledPush {
                progress: None,
                next_at: before.next,
                scheduling_error: None,
            });
        }
        before.last = now;
        before.next = now + schedule::RESERVATION_SECONDS;
        let reserved = schedule::write(&tx, &self.key, &scope, &before)?;
        tx.commit()?;
        let progress = self.push_step(now);
        let network = match &progress {
            Err(Error::Network(error)) => Some(error),
            _ => None,
        };
        let key = &self.key;
        let completed = clock().and_then(|finished| {
            schedule::complete_with(
                &mut self.db,
                key,
                &scope,
                (&before, &reserved),
                finished,
                (progress.is_err(), network),
                |db| delay(&read(db, key, &scope)?.0, finished),
            )
        });
        let (next_at, scheduling_error) = match completed {
            Ok(next) => (next, None),
            Err(error) => (before.next, Some(error)),
        };
        Ok(ScheduledPush {
            progress: Some(progress),
            next_at,
            scheduling_error,
        })
    }
    pub(super) fn push_step(&mut self, now: u64) -> Result<Progress, Error> {
        let scope = scope(&self.db, &self.key)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        if !state.configured {
            return Ok(Progress::Idle);
        }
        let desired = state.preference.target()?;
        if matches!(state.pending, Some(Operation::Confirm(_)))
            && (desired.as_ref() != state.registered.as_ref()
                || state
                    .status
                    .as_ref()
                    .is_none_or(|s| s.expires_at.is_none_or(|e| e <= now)))
        {
            state.pending = None;
        }
        let action = if state.reconcile {
            Action::Read
        } else if let Some(op) = &state.pending {
            Action::Send(op.clone())
        } else if let Some(status) = state.status.as_ref() {
            if state.applied == state.generation
                && status.state == RemoteState::Disabled
                && desired.is_none()
            {
                return Ok(if matches!(state.preference, Preference::Unified { .. }) {
                    Progress::AwaitingEndpoint
                } else {
                    Progress::Idle
                });
            }
            let matches = state.registered.as_ref() == desired.as_ref() && desired.is_some();
            if matches
                && status.state == RemoteState::Active
                && status
                    .expires_at
                    .is_some_and(|e| e > now.saturating_add(86400))
                && state.applied == state.generation
            {
                if now < state.checked.saturating_add(86400) {
                    return Ok(Progress::Idle);
                }
                Action::Read
            } else if matches
                && status.state == RemoteState::Pending
                && status.expires_at.is_some_and(|e| e > now)
            {
                if now < state.checked.saturating_add(30) {
                    return Ok(Progress::AwaitingConfirmation);
                }
                Action::Read
            } else {
                let operation = match desired {
                    Some(target) => Operation::Register(Register {
                        expected_revision: status.revision,
                        target,
                    }),
                    None => Operation::Disable(Disable {
                        expected_revision: status.revision,
                    }),
                };
                state.pending = Some(operation.clone());
                Action::Send(operation)
            }
        } else {
            Action::Read
        };
        let sealed = write(&tx, &self.key, &scope, &state, before.as_deref())?;
        tx.commit()?;
        let client = self.connected_client()?;
        let response = match &action {
            Action::Read => client.push_status(),
            Action::Send(Operation::Register(r)) => client.register_push(r),
            Action::Send(Operation::Confirm(r)) => client.confirm_push(r),
            Action::Send(Operation::Disable(r)) => client.disable_push(r),
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if super::scope(&tx, &self.key)? != scope {
            return Err(Error::Conflict);
        }
        match response {
            Ok(status) => {
                let result = match &action {
                    Action::Read => {
                        reconcile_status(&mut state, status, now)?;
                        Progress::Reconciled
                    }
                    Action::Send(operation) => {
                        state.checked = now;
                        state.status = Some(status.clone());
                        state.pending = None;
                        match operation {
                            Operation::Register(r) => {
                                state.registered = Some(r.target.clone());
                                if state.preference.target()?.as_ref() == Some(&r.target) {
                                    state.applied = state.generation;
                                }
                            }
                            Operation::Confirm(_) => {
                                if state.preference.target()?.as_ref() == state.registered.as_ref()
                                {
                                    state.applied = state.generation;
                                }
                            }
                            Operation::Disable(_) => {
                                state.registered = None;
                                if state.preference.target()?.is_none() {
                                    state.applied = state.generation;
                                }
                            }
                        }
                        Progress::Updated(status)
                    }
                };
                write(&tx, &self.key, &scope, &state, Some(&sealed))?;
                tx.commit()?;
                Ok(result)
            }
            Err(network::Error::Status { code: 409, .. }) if matches!(action, Action::Send(_)) => {
                state.reconcile = true;
                write(&tx, &self.key, &scope, &state, Some(&sealed))?;
                tx.commit()?;
                Ok(Progress::Reconciled)
            }
            Err(error) => Err(error.into()),
        }
    }
}

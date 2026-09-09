//! One bounded online pass; platform callers schedule it on a blocking worker.
use super::*;

#[derive(Debug)]
pub enum SyncFailure {
    Receive(Error),
    Acknowledge(Error),
    Prekeys(Error),
    /// Index into SyncStep::prekeys; preserves the network error and Retry-After.
    PrekeyNetwork(usize),
    /// The network error is retained in SyncStep::prekey_supply.
    PrekeySupplyNetwork,
    SendIntents(Error),
    SendIntentNetwork(usize),
    RetryControls(Error),
    RetryControlNetwork(usize),
    Recovery(Error),
    /// Index into SyncStep::retries; preserves the network error and Retry-After.
    RecoveryNetwork(usize),
    Outbound(Error),
    /// Index into SyncStep::outbound; preserves the network error and Retry-After.
    OutboundNetwork(usize),
    GroupOutbound(Error),
    GroupOutboundNetwork(usize),
    Groups(Error),
    GroupNetwork(usize),
    Invitations(Error),
    InvitationNetwork(usize),
    History(Error),
    HistoryNetwork(usize),
    Maintenance(Error),
    Structured(Error),
    Conversations(Error),
    Calls(Error),
    CallNetwork(usize),
}
impl SyncFailure {
    pub(crate) fn stage(&self) -> &'static str {
        match self {
            Self::Receive(_) => "receiving messages",
            Self::Acknowledge(_) => "acknowledging messages",
            Self::Prekeys(_) | Self::PrekeyNetwork(_) | Self::PrekeySupplyNetwork => {
                "publishing keys"
            }
            Self::SendIntents(_) | Self::SendIntentNetwork(_) => "starting conversations",
            Self::RetryControls(_) | Self::RetryControlNetwork(_) => "sending retry controls",
            Self::Recovery(_) | Self::RecoveryNetwork(_) => "recovering sessions",
            Self::Outbound(_) | Self::OutboundNetwork(_) => "sending messages",
            Self::GroupOutbound(_) | Self::GroupOutboundNetwork(_) => "sending group messages",
            Self::Groups(_) | Self::GroupNetwork(_) => "updating groups",
            Self::Invitations(_) | Self::InvitationNetwork(_) => "updating invitations",
            Self::History(_) | Self::HistoryNetwork(_) => "sharing history",
            Self::Maintenance(_) => "maintaining sessions",
            Self::Structured(_) => "updating structured content",
            Self::Conversations(_) => "synchronizing conversations",
            Self::Calls(_) | Self::CallNetwork(_) => "updating calls",
        }
    }
}
#[derive(Default)]
pub struct SyncStep {
    pub calls: Vec<calls::Attempt>,
    pub incoming: Vec<IncomingAttempt>,
    pub acknowledged: usize,
    pub prekeys: Vec<PrekeyAttempt>,
    pub prekey_supply: Option<Result<PrekeySupply, Error>>,
    pub sends: Vec<SendIntentAttempt>,
    pub retry_controls: Vec<RetryAttempt>,
    pub retries: Vec<RetryAttempt>,
    pub outbound: Vec<OutboundAttempt>,
    pub group_outbound: Vec<groups::GroupSendAttempt>,
    pub groups: Vec<groups::GroupWorkAttempt>,
    pub invitations: Vec<groups::InvitationAttempt>,
    pub history: Vec<groups::HistoryAttempt>,
    pub maintenance: Option<SessionMaintenance>,
    pub structured: usize,
    pub conversation_copies: usize,
    pub delivery_receipts: usize,
    /// Reports stage failure; inspect per-item results even when this is None.
    /// A failed stage can have partial durable effects; retry uses its journals.
    pub failure: Option<SyncFailure>,
}
impl ClientStore {
    /// A platform worker invokes this for the enabled backend services.
    /// Each network lane preserves its own durable deadline and error report.
    pub fn sync_backend_due_online(&mut self, cache: &mut attachments::Cache) -> BackendWork {
        let cleanup = self.maintain_history(conversations::now());
        let erasure = self.erase_obsolete_journals(conversations::now());
        let messaging = self.sync_due_online();
        let recovery = match recovery::configured(&self.db) {
            Ok(true) => Some(self.sync_recovery_due_online()),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        };
        let media = self.prepare_recovery_media_step(cache, conversations::now());
        let transfers = self.sync_attachments_due_online(cache);
        let push = self.sync_push_due_online();
        BackendWork {
            erasure,
            cleanup,
            messaging,
            recovery,
            media,
            transfers,
            push,
        }
    }
    /// Receive, acknowledge durable results, resume prepared prekey publications, authorized recovery and queued
    /// outbound packets, then check retirement eligibility. Stops network work on a stage/network error;
    /// callers honor Retry-After and schedule the next pass. Individual receive
    /// failures remain unacknowledged. No identity approval or new retry request
    /// is inferred. Replenishes at most one prekey toward a target of eight.
    /// This low-level pass bypasses scheduling; use sync_due_online for durable backoff.
    pub fn sync_step_online(&mut self, now: u64) -> SyncStep {
        let mut step = SyncStep::default();
        match self.advance_structured_actions() {
            Ok(count) => step.structured = count,
            Err(error) => {
                step.failure = Some(SyncFailure::Structured(error));
                return step;
            }
        }
        match self.receive_mailbox_online(now) {
            Ok(incoming) => step.incoming = incoming,
            Err(error) => {
                step.failure = Some(SyncFailure::Receive(error));
                return step;
            }
        }
        match self.acknowledge_incoming_online() {
            Ok(count) => step.acknowledged = count,
            Err(error) => {
                step.failure = Some(SyncFailure::Acknowledge(error));
                return step;
            }
        }
        match self.resume_prekey_publications_online() {
            Ok(prekeys) => step.prekeys = prekeys,
            Err(error) => {
                step.failure = Some(SyncFailure::Prekeys(error));
                return step;
            }
        }
        if let Some(index) = step
            .prekeys
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::PrekeyNetwork(index));
            return step;
        }
        step.prekey_supply = Some(self.replenish_prekey_online());
        if matches!(step.prekey_supply, Some(Err(Error::Network(_)))) {
            step.failure = Some(SyncFailure::PrekeySupplyNetwork);
            return step;
        }
        match self.resume_calls_online(now) {
            Ok(calls) => step.calls = calls,
            Err(error) => {
                step.failure = Some(SyncFailure::Calls(error));
                return step;
            }
        }
        if let Some(index) = step
            .calls
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::CallNetwork(index));
            return step;
        }
        match self.resume_group_invitations_online(now) {
            Ok(invitations) => step.invitations = invitations,
            Err(error) => {
                step.failure = Some(SyncFailure::Invitations(error));
                return step;
            }
        }
        if let Some(index) = step
            .invitations
            .iter()
            .position(|v| matches!(v.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::InvitationNetwork(index));
            return step;
        }
        match self.resume_groups_online(now) {
            Ok(groups) => step.groups = groups,
            Err(error) => {
                step.failure = Some(SyncFailure::Groups(error));
                return step;
            }
        }
        if let Some(index) = step
            .groups
            .iter()
            .position(|v| matches!(v.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::GroupNetwork(index));
            return step;
        }
        match self.resume_group_history_online(now) {
            Ok(history) => step.history = history,
            Err(error) => {
                step.failure = Some(SyncFailure::History(error));
                return step;
            }
        }
        if let Some(index) = step
            .history
            .iter()
            .position(|v| matches!(v.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::HistoryNetwork(index));
            return step;
        }
        match self.resume_retry_controls_online(now) {
            Ok(controls) => step.retry_controls = controls,
            Err(error) => {
                step.failure = Some(SyncFailure::RetryControls(error));
                return step;
            }
        }
        if let Some(index) = step
            .retry_controls
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::RetryControlNetwork(index));
            return step;
        }
        match self.resume_retries_online(now) {
            Ok(retries) => step.retries = retries,
            Err(error) => {
                step.failure = Some(SyncFailure::Recovery(error));
                return step;
            }
        }
        if let Some(index) = step
            .retries
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::RecoveryNetwork(index));
            return step;
        }
        match self.resume_conversation_receipts(now) {
            Ok(count) => step.delivery_receipts = count,
            Err(error) => {
                step.failure = Some(SyncFailure::Conversations(error));
                return step;
            }
        }
        match self.sync_conversation_devices(now) {
            Ok(count) => step.conversation_copies = count,
            Err(error) => {
                step.failure = Some(SyncFailure::Conversations(error));
                return step;
            }
        }
        match self.resume_send_intents_online(now) {
            Ok(sends) => step.sends = sends,
            Err(error) => {
                step.failure = Some(SyncFailure::SendIntents(error));
                return step;
            }
        }
        if let Some(index) = step
            .sends
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::SendIntentNetwork(index));
            return step;
        }
        match self.resume_outbound_online(now) {
            Ok(outbound) => step.outbound = outbound,
            Err(error) => {
                step.failure = Some(SyncFailure::Outbound(error));
                return step;
            }
        }
        if let Some(index) = step
            .outbound
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::OutboundNetwork(index));
            return step;
        }
        match self.resume_group_outbound_online(now) {
            Ok(outbound) => step.group_outbound = outbound,
            Err(error) => {
                step.failure = Some(SyncFailure::GroupOutbound(error));
                return step;
            }
        }
        if let Some(index) = step
            .group_outbound
            .iter()
            .position(|item| matches!(item.result, Err(Error::Network(_))))
        {
            step.failure = Some(SyncFailure::GroupOutboundNetwork(index));
            return step;
        }
        match self.maintain_sessions_online(now) {
            Ok(maintenance) => step.maintenance = Some(maintenance),
            Err(error) => step.failure = Some(SyncFailure::Maintenance(error)),
        }
        step
    }
}

pub struct BackendWork {
    pub erasure: Result<JournalErasure, Error>,
    pub push: Result<push::ScheduledPush, Error>,
    pub cleanup: Result<conversations::HistoryCleanup, Error>,
    pub messaging: Result<ScheduledSync, Error>,
    pub recovery: Option<Result<recovery::ScheduledRecovery, Error>>,
    pub media: Result<attachments::MediaRecovery, Error>,
    pub transfers: Result<attachments::ScheduledTransfers, Error>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        claims::tests::pair,
        incoming::tests::{start, trust},
    };
    fn reopen(path: &Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn interrupted_ack_preserves_received_result_and_resumes_after_restart() {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        start(&mut alice, b, now);
        bob.db.execute_batch("CREATE TRIGGER fail_ack BEFORE UPDATE ON incoming BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let step = bob.sync_step_online(now);
        assert!(matches!(
            step.failure,
            Some(SyncFailure::Acknowledge(Error::Storage(_)))
        ));
        assert_eq!(step.incoming.len(), 1);
        assert!(
            matches!(&step.incoming[0].result, Ok(MailboxEvent::Text(text)) if text.text().unwrap().body == "synthetic initial")
        );
        assert!(step.maintenance.is_none() && step.retries.is_empty());
        bob.db.execute_batch("DROP TRIGGER fail_ack;").unwrap();
        drop(bob);
        let mut bob = reopen(&dir.path().join("bob.db"));
        let resumed = bob.sync_step_online(now);
        assert!(resumed.failure.is_none());
        assert_eq!(resumed.acknowledged, 1);
        assert!(resumed.maintenance.is_some());
        assert!(bob
            .connected_client()
            .unwrap()
            .mailbox()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn explicit_recovery_resumes_without_automatic_requests_for_receive_failures() {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        start(&mut alice, b, now);
        bob.db.execute("UPDATE prekeys SET state=NULL", []).unwrap();
        let step = bob.sync_step_online(now);
        assert!(step.failure.is_none());
        assert!(step.incoming[0].result.is_err());
        assert_eq!(step.acknowledged, 0);
        assert_eq!(
            bob.db
                .query_row("SELECT count(*) FROM retry_outbox", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let RecoveryAdvice::Offer(action) = &step.incoming[0].recovery else {
            panic!("missing prekey must offer an explicit recovery action");
        };
        let retry = bob.approve_recovery(action, now).unwrap();
        assert_eq!(bob.approve_recovery(action, now).unwrap(), retry);
        bob.db.execute_batch("CREATE TRIGGER fail_control BEFORE UPDATE ON retry_outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let attempts = bob.resume_retry_controls_online(now).unwrap();
        assert_eq!(attempts.len(), 1);
        assert!(matches!(attempts[0].result, Err(Error::Storage(_))));
        bob.db.execute_batch("DROP TRIGGER fail_control;").unwrap();
        drop(bob);
        let mut bob = reopen(&dir.path().join("bob.db"));
        // The durable cursor advances past the failed control, then wraps.
        assert!(bob.resume_retry_controls_online(now).unwrap().is_empty());
        let resumed = bob.sync_step_online(now);
        assert!(resumed.failure.is_none());
        assert_eq!(resumed.retry_controls.len(), 1);
        assert!(resumed.retry_controls[0].result.is_ok());
        assert!(bob.resume_retry_controls_online(now).unwrap().is_empty());
        assert!(matches!(
            step.prekey_supply,
            Some(Ok(PrekeySupply::Published { .. }))
        ));
        let resumed = alice.sync_step_online(now);
        assert!(resumed.failure.is_none());
        assert_eq!(resumed.acknowledged, 1);
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        // Existing recovery/mailbox cursors wrap independently across passes.
        let mut recovered = false;
        for _ in 0..4 {
            assert!(alice.sync_step_online(now).failure.is_none());
            let step = bob.sync_step_online(now);
            assert!(step.failure.is_none());
            recovered |= step.incoming.iter().any(|item| matches!(&item.result, Ok(MailboxEvent::Text(text)) if text.text().unwrap().body == "synthetic initial"));
        }
        assert!(recovered);
        assert!(bob
            .connected_client()
            .unwrap()
            .mailbox()
            .unwrap()
            .is_empty());
    }
}

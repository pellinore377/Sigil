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
/// Totals over the four retry GC passes; the first failing pass is in SyncStep::failure.
#[derive(Debug, Default)]
pub struct RetryCleanup {
    pub scanned: usize,
    pub reclaimed: usize,
    pub failed: usize,
}
#[derive(Default)]
pub struct SyncStep {
    pub calls: Vec<calls::Attempt>,
    pub incoming: Vec<IncomingAttempt>,
    pub acknowledged: usize,
    /// Acknowledgements refused per item; they never feed the schedule's failure streak.
    pub acknowledgements: Vec<AcknowledgeAttempt>,
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
    /// Retry GC passes run this step; None when maintenance was skipped.
    pub retry_cleanup: Option<RetryCleanup>,
    /// Own-account devices newly suspended because the server reports them revoked.
    pub revoked_devices: usize,
    pub structured: usize,
    pub conversation_copies: usize,
    pub delivery_receipts: usize,
    /// Reports stage failure; inspect per-item results even when this is None.
    /// A failed stage can have partial durable effects; retry uses its journals.
    pub failure: Option<SyncFailure>,
    /// Stage durations in milliseconds, in execution order; diagnostics only.
    pub timings: Vec<(&'static str, u32)>,
    started: Option<(&'static str, crate::clock::Instant)>,
}
impl SyncStep {
    /// Closes the running stage's timing and starts the named one.
    pub(super) fn begin(&mut self, name: &'static str) {
        if let Some((previous, at)) = self.started.take() {
            self.timings.push((previous, at.elapsed().as_millis() as u32));
        }
        self.started = Some((name, crate::clock::Instant::now()));
    }
    pub(crate) fn issue(&self) -> Option<(&'static str, &Error)> {
        if let Some(error) = schedule::failure_error(self) {
            return Some((self.failure.as_ref()?.stage(), error));
        }
        macro_rules! lane {
            ($items:expr, $stage:literal) => {
                if let Some(error) = $items
                    .iter()
                    .filter_map(|item| item.result.as_ref().err())
                    .find(|error| !matches!(error, Error::Obsolete)
                        // A recipient awaiting acceptance or identity consent is shown in its conversation, not as a sync failure.
                        && !(matches!(error, Error::Unprepared) && $stage == "sending messages")
                        // A peer whose inbox stopped draining keeps its queue; the bubble shows the pending state.
                        && !(matches!(error, Error::Limit) && matches!($stage, "starting conversations" | "sending messages" | "recovering sessions" | "sending retry controls"))
                        // A deadline the clock has not reached yet is deliberately kept
                        // rather than cancelled, so it is a wait, not something to report.
                        && !(matches!(error, Error::Expired) && matches!($stage, "recovering sessions" | "sending retry controls"))
                        && !matches!(error, Error::Network(error) if outbound::recipient_full(error)
                            // A recipient the server no longer knows cannot be reached by
                            // any lane; that is a fact about them, not a failure here.
                            || matches!($stage, "sending messages" | "sending group messages" | "recovering sessions" | "sending retry controls")
                                && outbound::recipient_unavailable(error)
                            // Recovery targets one device; if the server no longer offers
                            // it a session there is nothing here for the reader to fix.
                            || matches!($stage, "recovering sessions" | "sending retry controls")
                                && outbound::recipient_specific(error)))
                {
                    return Some(($stage, error));
                }
            };
        }
        lane!(self.incoming, "receiving messages");
        lane!(self.acknowledgements, "acknowledging messages");
        lane!(self.prekeys, "publishing keys");
        if let Some(Err(error)) = &self.prekey_supply {
            if !matches!(error, Error::Obsolete) {
                return Some(("publishing keys", error));
            }
        }
        lane!(self.calls, "updating calls");
        lane!(self.invitations, "updating invitations");
        lane!(self.groups, "updating groups");
        lane!(self.history, "sharing history");
        lane!(self.retry_controls, "sending retry controls");
        lane!(self.retries, "recovering sessions");
        lane!(self.sends, "starting conversations");
        lane!(self.outbound, "sending messages");
        lane!(self.group_outbound, "sending group messages");
        None
    }
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
        self.sync_step_with_maintenance(now, true)
    }
    pub(super) fn sync_step_with_maintenance(&mut self, now: u64, maintenance: bool) -> SyncStep {
        let mut step = SyncStep::default();
        step.begin("advance_structured_actions");
        match self.advance_structured_actions() {
            Ok(count) => step.structured = count,
            Err(error) => {
                step.failure = Some(SyncFailure::Structured(error));
                return step;
            }
        }
        step.begin("receive_mailbox_online");
        match self.receive_mailbox_online(now) {
            Ok(incoming) => step.incoming = incoming,
            Err(error) => {
                step.failure = Some(SyncFailure::Receive(error));
                return step;
            }
        }
        step.begin("acknowledge_incoming_online");
        match self.acknowledge_incoming_report_online() {
            Ok(report) => {
                step.acknowledged = report.acknowledged;
                step.acknowledgements = report.rejected;
            }
            // Telling the server what has already been stored is not a condition of
            // sending. One delivery it will not accept must not leave this device able
            // to receive and unable to say anything back.
            Err(error) => step.failure = Some(SyncFailure::Acknowledge(error)),
        }
        step.begin("resume_prekey_publications_online");
        match self.resume_prekey_publications_online() {
            Ok(prekeys) => step.prekeys = prekeys,
            Err(error) => {
                step.failure = Some(SyncFailure::Prekeys(error));
                return step;
            }
        }
        if let Some(index) = step.prekeys.iter().position(|item| match &item.result {
            // A refused publication must not stop messaging: this device's own keys
            // are for others to reach it, and sending needs the recipient's. Preserve
            // global backoff for authentication, throttling and outages.
            Err(Error::Network(network::Error::Status {
                code: 400 | 403 | 404 | 409 | 410 | 422,
                retry_after_seconds: None,
            })) => false,
            Err(Error::Network(_)) => true,
            _ => false,
        }) {
            step.failure = Some(SyncFailure::PrekeyNetwork(index));
            return step;
        }
        step.begin("resume_calls_online");
        match self.resume_calls_online(now) {
            Ok(calls) => step.calls = calls,
            Err(error) => {
                step.failure = Some(SyncFailure::Calls(error));
                return step;
            }
        }
        if let Some(index) = step.calls.iter().position(|item| match &item.result {
            // A rejected call must not stop unrelated messaging. Preserve
            // global backoff for authentication, throttling and outages.
            Err(Error::Network(network::Error::Status {
                code: 400 | 403 | 404 | 409 | 410 | 422,
                retry_after_seconds: None,
            })) => false,
            Err(Error::Network(_)) => true,
            _ => false,
        }) {
            step.failure = Some(SyncFailure::CallNetwork(index));
            return step;
        }
        step.begin("resume_group_invitations_online");
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
        step.begin("resume_groups_online");
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
        step.begin("resume_group_history_online");
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
        step.begin("resume_retry_controls_online");
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
            .position(|item| matches!(&item.result, Err(Error::Network(e)) if !outbound::recipient_specific(e)))
        {
            step.failure = Some(SyncFailure::RetryControlNetwork(index));
            return step;
        }
        step.begin("resume_retries_online");
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
            .position(|item| matches!(&item.result, Err(Error::Network(e)) if !outbound::recipient_specific(e)))
        {
            step.failure = Some(SyncFailure::RecoveryNetwork(index));
            return step;
        }
        // A full intent table is only ever drained by the send stages below, so a
        // capacity limit here is recorded and the pass carries on to them.
        step.begin("resume_conversation_receipts");
        match self.resume_conversation_receipts(now) {
            Ok(count) => step.delivery_receipts = count,
            // Only the send stages drain a full intent table. Recording a failure here
            // would also back the whole schedule off, so the pass simply carries on.
            Err(Error::Limit) => {}
            Err(error) => {
                step.failure = Some(SyncFailure::Conversations(error));
                return step;
            }
        }
        step.begin("sync_conversation_devices");
        match self.sync_conversation_devices(now) {
            Ok(count) => step.conversation_copies = count,
            // Only the send stages drain a full intent table. Recording a failure here
            // would also back the whole schedule off, so the pass simply carries on.
            Err(Error::Limit) => {}
            Err(error) => {
                step.failure = Some(SyncFailure::Conversations(error));
                return step;
            }
        }
        if !self.send_stages(now, &mut step) || !maintenance {
            return step;
        }
        step.begin("replenish_prekey_online");
        step.prekey_supply = Some(self.replenish_prekey_online());
        if matches!(step.prekey_supply, Some(Err(Error::Network(_)))) {
            step.failure = Some(SyncFailure::PrekeySupplyNetwork);
            return step;
        }
        step.begin("reconcile_own_devices_online");
        match self.reconcile_own_devices_online() {
            Ok(count) => step.revoked_devices = count,
            // An unreachable inventory changes nothing; the next pass reads it again.
            Err(Error::Network(_)) => {}
            Err(error) => step.failure = Some(SyncFailure::Maintenance(error)),
        }
        step.begin("release_unreachable_claims");
        if let Err(error) = self.release_unreachable_claims() {
            step.failure = Some(SyncFailure::Maintenance(error));
        }
        step.begin("maintain_sessions_online");
        match self.maintain_sessions_online(now) {
            Ok(maintenance) => step.maintenance = Some(maintenance),
            Err(error) => step.failure = Some(SyncFailure::Maintenance(error)),
        }
        // Each GC pass is bounded and independent; one failing must not hide the others.
        let mut cleanup = RetryCleanup::default();
        step.begin("reclaim_outgoing_retry_controls");
        cleanup.record(
            self.reclaim_outgoing_retry_controls(now)
                .map(|c| (c.scanned, c.retired)),
            &mut step,
        );
        step.begin("reclaim_accepted_retry_controls");
        cleanup.record(
            self.reclaim_accepted_retry_controls(now)
                .map(|c| (c.scanned, c.retired)),
            &mut step,
        );
        step.begin("reclaim_retry_journals");
        cleanup.record(
            self.reclaim_retry_journals(now)
                .map(|c| (c.scanned, c.reclaimed)),
            &mut step,
        );
        step.begin("reclaim_recovered_journals");
        cleanup.record(
            self.reclaim_recovered_journals(now)
                .map(|c| (c.scanned, c.reclaimed)),
            &mut step,
        );
        step.retry_cleanup = Some(cleanup);
        step
    }
}
impl RetryCleanup {
    fn record(&mut self, result: Result<(usize, usize), Error>, step: &mut SyncStep) {
        match result {
            Ok((scanned, reclaimed)) => {
                self.scanned += scanned;
                self.reclaimed += reclaimed;
            }
            Err(error) => {
                self.failed += 1;
                if step.failure.is_none() {
                    step.failure = Some(SyncFailure::Maintenance(error));
                }
            }
        }
    }
}

impl ClientStore {
    /// Call setup: receive, acknowledge, advance call jobs and send, skipping the
    /// group, history, retry and maintenance stages that a 250 ms cadence cannot afford.
    pub(super) fn call_setup_step(&mut self, now: u64) -> SyncStep {
        let mut step = SyncStep::default();
        step.begin("receive_mailbox_online");
        match self.receive_mailbox_online(now) {
            Ok(incoming) => step.incoming = incoming,
            Err(error) => {
                step.failure = Some(SyncFailure::Receive(error));
                return step;
            }
        }
        // Call controls go out before the acknowledgement round trip; acks follow at the end.
        step.begin("resume_calls_online");
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
            .position(|item| match &item.result {
                Err(Error::Network(network::Error::Status {
                    code: 400 | 403 | 404 | 409 | 410 | 422,
                    retry_after_seconds: None,
                })) => false,
                Err(Error::Network(_)) => true,
                _ => false,
            })
        {
            step.failure = Some(SyncFailure::CallNetwork(index));
            return step;
        }
        step.begin("sync_conversation_devices");
        match self.sync_conversation_devices(now) {
            Ok(count) => step.conversation_copies = count,
            // Only the send stages drain a full intent table. Recording a failure here
            // would also back the whole schedule off, so the pass simply carries on.
            Err(Error::Limit) => {}
            Err(error) => {
                step.failure = Some(SyncFailure::Conversations(error));
                return step;
            }
        }
        if !self.send_stages(now, &mut step) {
            return step;
        }
        step.begin("acknowledge_incoming_online");
        match self.acknowledge_incoming_report_online() {
            Ok(report) => {
                step.acknowledged = report.acknowledged;
                step.acknowledgements = report.rejected;
            }
            Err(error) => step.failure = Some(SyncFailure::Acknowledge(error)),
        }
        step
    }
    /// Own-device copies plus the three send stages; false stops the pass.
    pub(super) fn outbound_step(&mut self, now: u64) -> SyncStep {
        let mut step = SyncStep::default();
        step.begin("sync_conversation_devices");
        match self.sync_conversation_devices(now) {
            Ok(count) => step.conversation_copies = count,
            // Only the send stages drain a full intent table. Recording a failure here
            // would also back the whole schedule off, so the pass simply carries on.
            Err(Error::Limit) => {}
            Err(error) => {
                step.failure = Some(SyncFailure::Conversations(error));
                return step;
            }
        }
        self.send_stages(now, &mut step);
        step
    }
    fn send_stages(&mut self, now: u64, step: &mut SyncStep) -> bool {
        step.begin("resume_send_intents_online");
        match self.resume_send_intents_online(now) {
            Ok(sends) => step.sends = sends,
            Err(error) => {
                step.failure = Some(SyncFailure::SendIntents(error));
                return false;
            }
        }
        // A recipient answering full or unknown is about them, not the server: it must
        // not halt the pass nor back the whole schedule off, as the outbound lane below
        // already treats it.
        if let Some(index) = step
            .sends
            .iter()
            .position(|item| matches!(&item.result, Err(Error::Network(e)) if !crate::outbound::recipient_specific(e) && !crate::outbound::recipient_deferred(e)))
        {
            step.failure = Some(SyncFailure::SendIntentNetwork(index));
            return false;
        }
        step.begin("resume_outbound_online");
        match self.resume_outbound_online(now) {
            Ok(outbound) => step.outbound = outbound,
            Err(error) => {
                step.failure = Some(SyncFailure::Outbound(error));
                return false;
            }
        }
        if let Some(index) = step
            .outbound
            .iter()
            .position(|item| matches!(&item.result, Err(Error::Network(e)) if !crate::outbound::recipient_unavailable(e)))
        {
            step.failure = Some(SyncFailure::OutboundNetwork(index));
            return false;
        }
        step.begin("resume_group_outbound_online");
        match self.resume_group_outbound_online(now) {
            Ok(outbound) => step.group_outbound = outbound,
            Err(error) => {
                step.failure = Some(SyncFailure::GroupOutbound(error));
                return false;
            }
        }
        if let Some(index) = step
            .group_outbound
            .iter()
            .position(|item| matches!(&item.result, Err(Error::Network(error)) if !outbound::recipient_unavailable(error)))
        {
            step.failure = Some(SyncFailure::GroupOutboundNetwork(index));
            return false;
        }
        true
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
    fn queued_messages_precede_inventory_work_and_preserve_its_backoff() {
        let (dir, fixture, mut alice, mut bob, now) = pair();
        let (_, peer) = trust(&mut alice, &mut bob);
        alice
            .queue_peer_text(peer, [90; 32], "synthetic priority", now, now)
            .unwrap();
        let port = fixture.port();
        drop(fixture);
        let (router, maintenance) = sigil_server::router_with_maintenance(
            sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
            sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token"))
                .unwrap(),
        );
        let router = router.layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                if request.method() == axum::http::Method::GET
                    && request.uri().path() == "/client/v0/prekeys"
                {
                    return axum::http::Response::builder()
                        .status(429)
                        .header("retry-after", "120")
                        .body(axum::body::Body::empty())
                        .unwrap();
                }
                next.run(request).await
            },
        ));
        let _fixture = network::tests::Fixture::maintained_at(router, maintenance, port);
        let result = alice.sync_due_online().unwrap();
        let step = result.step.unwrap();
        assert!(matches!(
            step.failure,
            Some(SyncFailure::PrekeySupplyNetwork)
        ));
        assert!(step.sends.iter().any(|item| item.result.is_ok()));
        assert!(result.next_at >= now + 120);
        assert!(alice.sync_due_online().unwrap().step.is_none());
        assert!(bob
            .receive_mailbox_online(now)
            .unwrap()
            .iter()
            .any(|item| matches!(&item.result,
            Ok(MailboxEvent::Text(text)) if text.text().unwrap().body == "synthetic priority")));
    }
    #[test]
    fn unavailable_call_service_does_not_block_queued_messages() {
        for setup in [false, true] {
            let (dir, _fixture, mut alice, mut bob, now) = pair();
            let (_, peer) = trust(&mut alice, &mut bob);
            alice.create_direct_call([91; 32], now, 3600).unwrap();
            alice.invite_to_call([91; 32], peer, now).unwrap();
            alice
                .queue_peer_text(peer, [92; 32], "synthetic queued message", now, now)
                .unwrap();
            drop(alice);
            let mut alice = reopen(&dir.path().join("alice.db"));
            let step = if setup { alice.call_setup_step(now) } else { alice.sync_step_online(now) };
            assert!(step.calls.iter().any(|attempt| matches!(
                attempt.result,
                Err(Error::Network(network::Error::Status { code: 404, .. }))
            )));
            assert!(step.failure.is_none());
            assert_eq!(step.issue().map(|(stage, _)| stage), Some("updating calls"));
            assert!(step.sends.iter().any(|attempt| attempt.result.is_ok()));
            let incoming = bob.receive_mailbox_online(now).unwrap();
            assert!(incoming.iter().any(|item| matches!(&item.result,
                Ok(MailboxEvent::Text(text)) if text.text().unwrap().body == "synthetic queued message")));
        }
    }
    #[test]
    fn denied_claim_does_not_hold_other_messages_or_call_setup() {
        for setup in [false, true] {
            let (dir, fixture, mut alice, mut bob, now) = pair();
            let (_, peer) = trust(&mut alice, &mut bob);
            alice.queue_peer_text(peer, [94; 32], "synthetic denied", now, now).unwrap();
            alice.queue_peer_text(peer, [95; 32], "synthetic allowed", now, now).unwrap();
            let port = fixture.port();
            drop(fixture);
            let (router, maintenance) = sigil_server::router_with_maintenance(
                sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
                sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
            );
            let refused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let router = router.layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let refused = refused.clone();
                    async move {
                        if request.uri().path().ends_with("/prekeys/claim")
                            && !refused.swap(true, std::sync::atomic::Ordering::SeqCst)
                        {
                            return axum::http::Response::builder().status(403)
                                .body(axum::body::Body::empty()).unwrap();
                        }
                        next.run(request).await
                    }
                },
            ));
            let _fixture = network::tests::Fixture::maintained_at(router, maintenance, port);
            let step = if setup { alice.call_setup_step(now) } else { alice.sync_step_online(now) };
            assert!(step.failure.is_none(), "{:?}", step.issue());
            assert!(step.sends.iter().any(|v| matches!(&v.result, Err(Error::Network(network::Error::Status { code: 403, .. })))));
            assert!(step.sends.iter().any(|v| v.result.is_ok()));
            assert_eq!(alice.db.query_row("SELECT count(*) FROM send_intent_backoff WHERE until>?1", [now as i64], |r| r.get::<_, i64>(0)).unwrap(), 1);
            let incoming = bob.receive_mailbox_online(now).unwrap();
            assert!(incoming.iter().any(|item| matches!(&item.result,
                Ok(MailboxEvent::Text(text)) if text.text().unwrap().body == "synthetic allowed")));
        }
    }
    #[test]
    fn small_positive_clock_skew_does_not_exceed_server_delivery_lifetime() {
        let (_dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, peer) = trust(&mut alice, &mut bob);
        alice
            .queue_peer_text(peer, [93; 32], "synthetic clock skew", now + 30, now + 30)
            .unwrap();
        let step = alice.sync_step_online(now + 30);
        assert!(step.failure.is_none(), "{:?}", step.issue());
        let incoming = bob.receive_mailbox_online(now + 30).unwrap();
        assert!(incoming.iter().any(|item| matches!(&item.result,
            Ok(MailboxEvent::Text(text)) if text.text().unwrap().body == "synthetic clock skew")));
    }
    #[test]
    fn call_authentication_and_retry_after_still_defer_network_work() {
        for setup in [false, true] {
            for (status, retry) in [(401, None), (429, Some(120)), (404, Some(120))] {
                let (dir, fixture, mut alice, mut bob, now) = pair();
                let (_, peer) = trust(&mut alice, &mut bob);
                alice.create_direct_call([91; 32], now, 3600).unwrap();
                alice
                    .queue_peer_text(peer, [92; 32], "synthetic queued message", now, now)
                    .unwrap();
                let port = fixture.port();
                drop(fixture);
                let (router, maintenance) = sigil_server::router_with_maintenance(
                    sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
                    sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token"))
                        .unwrap(),
                );
                let router = router.layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| async move {
                        if request.uri().path() == "/client/v0/calls" {
                            let mut response = axum::http::Response::builder().status(status);
                            if let Some(seconds) = retry {
                                response = response.header("retry-after", seconds.to_string());
                            }
                            return response.body(axum::body::Body::empty()).unwrap();
                        }
                        next.run(request).await
                    },
                ));
                let _fixture = network::tests::Fixture::maintained_at(router, maintenance, port);
                let result = if setup { alice.sync_call_setup_online() } else { alice.sync_due_online() }.unwrap();
                let step = result.step.unwrap();
                assert!(matches!(step.failure, Some(SyncFailure::CallNetwork(_))));
                assert!(step.sends.is_empty() && step.outbound.is_empty());
                if let Some(seconds) = retry {
                    assert!(result.next_at >= now + seconds);
                }
                assert!(alice.sync_due_online().unwrap().step.is_none());
                assert_eq!(
                    alice
                        .db
                        .query_row("SELECT count(*) FROM send_intents", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    1
                );
            }
        }
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
        // A refused acknowledgement no longer ends the pass: telling the server what
        // is already stored is not a condition of sending. The delivery is still held
        // and is acknowledged after the restart below.
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
    fn refused_acknowledgement_is_reported_without_throttling_the_schedule() {
        let (_dir, _fixture, mut alice, mut bob, _now) = pair();
        trust(&mut alice, &mut bob);
        // A sealed state that never opens: no later attempt can succeed.
        bob.db
            .execute("INSERT INTO incoming VALUES(?1,0,zeroblob(173))", [7i64])
            .unwrap();
        let own = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
        let before = schedule::clock().unwrap();
        let result = bob.sync_due_online().unwrap();
        let step = result.step.unwrap();
        assert!(step.failure.is_none(), "{:?}", step.failure);
        assert_eq!(step.issue().map(|(stage, _)| stage), Some("acknowledging messages"));
        assert!(step
            .acknowledgements
            .iter()
            .any(|item| item.sequence == 7 && item.result.is_err()));
        assert!(step.maintenance.is_some());
        assert_eq!(schedule::read(&bob.db, &bob.key, &own).unwrap().0.failures, 0);
        assert!(result.next_at >= before + schedule::POLL_SECONDS);
        assert!(result.next_at <= schedule::clock().unwrap() + schedule::POLL_SECONDS);
        // A healthy streak lets a wake pull the next pass forward for a fresh send.
        assert!(bob.sync_wake_online().unwrap().step.is_some());
        assert!(bob.acknowledge_incoming_online().is_err());
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
        assert_eq!(
            step.issue().map(|(stage, _)| stage),
            Some("receiving messages")
        );
        // The pass releases the slot the undecryptable copy was holding.
        assert_eq!(step.acknowledged, 1);
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

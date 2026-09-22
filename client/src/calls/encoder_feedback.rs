use rtc::{
    interceptor::{interceptor, Interceptor, Packet, StreamInfo, TaggedPacket},
    sansio,
    shared::error::Error,
};
use std::{sync::{Arc, atomic::{AtomicU8, AtomicU32, Ordering}}, time::{Duration, Instant}};

/// What the forwarder reports about this sender's camera upload, for rate control.
#[derive(Default)]
pub struct Uplink {
    /// Receiver reports seen; a change marks `loss` as fresh.
    pub reports: AtomicU32,
    /// RTCP fraction lost, out of 256.
    pub loss: AtomicU8,
    pub key_requests: AtomicU32,
    /// Retransmission requests for the camera upload.
    pub nacks: AtomicU32,
}
const CAMERA: u32 = 2;

#[derive(Interceptor)]
pub(super) struct Feedback<P> {
    #[next]
    inner: P,
    requests: Arc<AtomicU8>,
    uplink: Arc<Uplink>,
    last: [Option<Instant>; 2],
}
impl<P> Feedback<P> {
    pub fn new(inner: P, requests: Arc<AtomicU8>, uplink: Arc<Uplink>) -> Self {
        Self { inner, requests, uplink, last: [None; 2] }
    }
    fn reported(&self, reports: &[rtc::rtcp::reception_report::ReceptionReport]) {
        for report in reports.iter().filter(|r| r.ssrc == CAMERA) {
            self.uplink.loss.store(report.fraction_lost, Ordering::Relaxed);
            self.uplink.reports.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[interceptor]
impl<P: Interceptor> Feedback<P> {
    #[overrides]
    fn handle_read(&mut self, message: TaggedPacket) -> Result<(), Self::Error> {
        // The chain consumes RTCP before track polling. Observe authenticated feedback here.
        if let Packet::Rtcp(packets) = &message.message {
            for packet in packets {
                let any = packet.as_any();
                if let Some(rr) = any.downcast_ref::<rtc::rtcp::receiver_report::ReceiverReport>() {
                    self.reported(&rr.reports);
                } else if let Some(sr) = any.downcast_ref::<rtc::rtcp::sender_report::SenderReport>() {
                    self.reported(&sr.reports);
                } else if any.downcast_ref::<rtc::rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack>().is_some_and(|n| n.media_ssrc == CAMERA) {
                    self.uplink.nacks.fetch_add(1, Ordering::Relaxed);
                }
                if packet.as_any().is::<rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication>()
                    || packet.as_any().is::<rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest>()
                {
                    for ssrc in packet.destination_ssrc().into_iter().filter(|ssrc| (2..=3).contains(ssrc)) {
                        let last = &mut self.last[(ssrc - 2) as usize];
                        if last.is_none_or(|at| message.now.saturating_duration_since(at) >= Duration::from_millis(200)) {
                            *last = Some(message.now);
                            if ssrc == CAMERA { self.uplink.key_requests.fetch_add(1, Ordering::Relaxed); }
                            self.requests.fetch_or(1 << (ssrc - 1), Ordering::Relaxed);
                        }
                    }
                }
            }
        }
        self.inner.handle_read(message)
    }
}

use rtc::{
    interceptor::{interceptor, Interceptor, Packet, StreamInfo, TaggedPacket},
    sansio,
    shared::error::Error,
};
use std::{sync::{Arc, atomic::{AtomicU8, Ordering}}, time::{Duration, Instant}};

#[derive(Interceptor)]
pub(super) struct Feedback<P> {
    #[next]
    inner: P,
    requests: Arc<AtomicU8>,
    last: [Option<Instant>; 2],
}
impl<P> Feedback<P> {
    pub fn new(inner: P, requests: Arc<AtomicU8>) -> Self {
        Self { inner, requests, last: [None; 2] }
    }
}
#[interceptor]
impl<P: Interceptor> Feedback<P> {
    #[overrides]
    fn handle_read(&mut self, message: TaggedPacket) -> Result<(), Self::Error> {
        // The chain consumes RTCP before track polling. Observe authenticated feedback here.
        if let Packet::Rtcp(packets) = &message.message {
            for packet in packets {
                if packet.as_any().is::<rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication>()
                    || packet.as_any().is::<rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest>()
                {
                    for ssrc in packet.destination_ssrc().into_iter().filter(|ssrc| (2..=3).contains(ssrc)) {
                        let last = &mut self.last[(ssrc - 2) as usize];
                        if last.is_none_or(|at| message.now.saturating_duration_since(at) >= Duration::from_millis(200)) {
                            *last = Some(message.now);
                            self.requests.fetch_or(1 << (ssrc - 1), Ordering::Relaxed);
                        }
                    }
                }
            }
        }
        self.inner.handle_read(message)
    }
}

//! Recovery requires an explicit caller decision; failures never imply new trust.
use super::*;
#[cfg(test)]
#[path = "recovery_action_tests.rs"]
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryBlock {
    Trust,
    LocalState,
    Capacity,
    InvalidTraffic,
    Expired,
    NoMissingStateEvidence,
}
pub enum RecoveryAdvice {
    None,
    Offer(RecoveryAction),
    Refused(RecoveryBlock),
}
pub struct RecoveryAction {
    delivery: Delivery,
    fingerprint: Id,
}
impl RecoveryAction {
    pub fn sequence(&self) -> i64 {
        self.delivery.sequence
    }
    pub fn peer_fingerprint(&self) -> Id {
        self.fingerprint
    }
}
impl ClientStore {
    pub(crate) fn recovery_advice(
        &mut self,
        delivery: &Delivery,
        result: &Result<MailboxEvent, Error>,
        now: u64,
    ) -> RecoveryAdvice {
        let Err(error) = result else {
            return RecoveryAdvice::None;
        };
        if delivery.expires_at <= now {
            return RecoveryAdvice::Refused(RecoveryBlock::Expired);
        }
        if matches!(error, Error::Limit) {
            return RecoveryAdvice::Refused(RecoveryBlock::Capacity);
        }
        if matches!(
            error,
            Error::Storage(_)
                | Error::Io(_)
                | Error::InvalidStore
                | Error::Crypto(sigil_crypto::Error::Entropy | sigil_crypto::Error::State)
        ) {
            return RecoveryAdvice::Refused(RecoveryBlock::LocalState);
        }
        if !network::valid_hex(
            &delivery.payload,
            32,
            sigil_protocol::mailbox::MAX_PAYLOAD_HEX,
        ) {
            return RecoveryAdvice::Refused(RecoveryBlock::InvalidTraffic);
        }
        let own = match self
            .own_device_binding()
            .and_then(|v| SignedBinding::from_bytes(&v).map_err(|_| Error::InvalidStore))
        {
            Ok(v) => v,
            Err(_) => return RecoveryAdvice::Refused(RecoveryBlock::LocalState),
        };
        let peer = match crate::federation::delivery_peer(
            &self.db,
            &self.key,
            &own.binding.server,
            delivery,
        ) {
            Ok(peer) => peer,
            Err(_) => return RecoveryAdvice::Refused(RecoveryBlock::Trust),
        };
        let known = match peers::known(&self.db, &self.key, &peer) {
            Ok(v) if v.trusted => v,
            Ok(_) | Err(Error::NotFound | Error::Unprepared | Error::Conflict) => {
                return RecoveryAdvice::Refused(RecoveryBlock::Trust)
            }
            Err(_) => return RecoveryAdvice::Refused(RecoveryBlock::LocalState),
        };
        let packet: Vec<u8> = match delivery
            .payload
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                u8::from_str_radix(std::str::from_utf8(p).map_err(|_| Error::InvalidEvent)?, 16)
                    .map_err(|_| Error::InvalidEvent)
            })
            .collect::<Result<_, _>>()
        {
            Ok(v) => v,
            Err(_) => return RecoveryAdvice::Refused(RecoveryBlock::InvalidTraffic),
        };
        let eligible = if let Ok((initial, bootstrap)) = sigil_protocol::initial::decode(&packet) {
            if sigil_crypto::handshake::InitialMessage::from_bytes(initial).is_err()
                || Packet::from_bytes(bootstrap).is_err()
            {
                return RecoveryAdvice::Refused(RecoveryBlock::InvalidTraffic);
            }
            matches!(error, Error::NotFound | Error::AlreadyDelivered)
        } else {
            if Packet::from_bytes(&packet).is_err() {
                return RecoveryAdvice::Refused(RecoveryBlock::InvalidTraffic);
            }
            let live = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE peer=?1 AND retired=0)",
                [peer.as_slice()],
                |r| r.get::<_, bool>(0),
            );
            match live {
                Ok(false) => matches!(error, Error::Crypto(sigil_crypto::Error::Authentication)),
                Ok(true) => false,
                Err(_) => return RecoveryAdvice::Refused(RecoveryBlock::LocalState),
            }
        };
        if matches!(error, Error::Crypto(sigil_crypto::Error::Limit)) {
            return RecoveryAdvice::Refused(RecoveryBlock::Capacity);
        }
        if !eligible {
            return RecoveryAdvice::Refused(RecoveryBlock::NoMissingStateEvidence);
        }
        RecoveryAdvice::Offer(RecoveryAction {
            fingerprint: known.fingerprint,
            delivery: Delivery {
                origin: delivery.origin.clone(),
                sequence: delivery.sequence,
                sender_device: delivery.sender_device.clone(),
                message_id: delivery.message_id.clone(),
                payload: delivery.payload.clone(),
                expires_at: delivery.expires_at,
            },
        })
    }

    /// Approve only an offered action. Rechecks current delivery/trust and commits
    /// one exact signed control; the shared worker submits it across restarts.
    pub fn approve_recovery(&mut self, action: &RecoveryAction, now: u64) -> Result<Id, Error> {
        if now == 0 || now > i64::MAX as u64 || action.delivery.expires_at <= now {
            return Err(Error::Expired);
        }
        let result = self
            .accept_delivery(&action.delivery)
            .map(MailboxEvent::Text);
        if result.is_ok() || self.resolve_failed_delivery(&action.delivery)? {
            return Err(Error::AlreadyDelivered);
        }
        match self.recovery_advice(&action.delivery, &result, now) {
            RecoveryAdvice::Offer(current) if current.fingerprint == action.fingerprint => {
                self.prepare_retry_for(&action.delivery, now, Some(action.fingerprint))
            }
            RecoveryAdvice::Refused(RecoveryBlock::Expired) => Err(Error::Expired),
            _ => Err(Error::Unprepared),
        }
    }
}

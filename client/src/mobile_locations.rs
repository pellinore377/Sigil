use super::*;
use sigil_protocol::text::{action::Action as CardAction, location::Point};

impl ClientStore {
    pub(super) fn mobile_location_queue(&mut self, conversation: Id, action: CardAction) -> Result<(), Error> {
        let peer = if conversation == self.mobile_conversation("self")? {
            "self".to_owned()
        } else if let Some(peer) = self.mobile_peers()?.into_iter().find(|peer| self.direct_conversation(peer.id).ok() == Some(conversation)) {
            self.mobile_peer_display(&peer)?
        } else {
            self.group_status(conversation)?;
            format!("group:{}", transport::hex(&conversation))
        };
        self.mobile_action(&peer, &transport::hex(&action.id().map_err(|_| Error::InvalidEvent)?), action.created_at, Action::Post {
            body: Body::Rich(action.to_bytes().map_err(|_| Error::InvalidEvent)?),
            reply: None, thread: None, expires_at: None, view_once: false,
        })?;
        Ok(())
    }
    pub(super) fn mobile_location_work(&mut self, after: Option<Id>, point: Option<Point>, stop: bool) -> Result<Value, Error> {
        let now = conversations::now();
        if let Some(point) = &point {
            point.validate().map_err(|_| Error::InvalidEvent)?;
            if stop || point.sampled_at > now || now.saturating_sub(point.sampled_at) > 60 { return Err(Error::InvalidEvent); }
        }
        let batch = self.location_jobs(after, now)?;
        let mut issue = None;
        let mut queued = 0;
        let mut active = 0;
        let mut until = None;
        let mut queue = |store: &mut Self, conversation, action| {
            match store.mobile_location_queue(conversation, action) {
                Ok(()) => queued += 1,
                Err(error) => issue = Some(error_message(&error)),
            }
        };
        for (conversation, action) in batch.stops { queue(self, conversation, action); }
        for job in batch.jobs {
            if stop {
                let action = self.stop_location(job.conversation, job.reference, now)?;
                queue(self, job.conversation, action);
            } else {
                active += 1;
                until = Some(until.unwrap_or(0).max(job.until));
                if let Some(point) = &point {
                    if point.sampled_at >= job.next_sample_at {
                        let action = self.update_location(job.conversation, job.reference, point.clone(), now)?;
                        queue(self, job.conversation, action);
                    }
                }
            }
        }
        Ok(json!({"active":active,"until":until,"queued":queued,"issue":issue,"next":batch.next.map(|id| transport::hex(&id))}))
    }
}

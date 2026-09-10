use super::*;
use crate::push::{Choice, ReceivedHint};
use base64ct::{Base64UrlUnpadded, Encoding};

impl ClientStore {
    pub(super) fn mobile_push(
        &mut self,
        action: &str,
        connection: Option<&str>,
        endpoint: Option<&str>,
        payload: Option<&str>,
        token: Option<&str>,
        replace: bool,
    ) -> Result<Value, Error> {
        let now = conversations::now();
        match action {
            "status" => (),
            "fcm" => {
                let state = self.push_state()?;
                if state.configured && !replace {
                    return Ok(json!({"ignored":true}));
                }
                if !self.connected_client()?.push_providers()?.fcm {
                    return Ok(json!({"unavailable":true}));
                }
                self.set_fcm_push_token(token.ok_or(Error::InvalidEvent)?, now)?;
            }
            "fcm_token" => {
                if self.push_state()?.choice != Choice::Fcm {
                    return Ok(json!({"ignored":true}));
                }
                self.set_fcm_push_token(token.ok_or(Error::InvalidEvent)?, now)?;
            }
            "fcm_receive" => {
                let hint = self.receive_fcm_push(payload.ok_or(Error::InvalidEvent)?, now)?;
                return Ok(json!({"accepted": hint != ReceivedHint::Ignored}));
            }
            "prepare" => {
                let providers = self.connected_client()?.push_providers()?;
                let Some(vapid) = providers
                    .vapid_public_key
                    .filter(|_| providers.unified_push)
                else {
                    return Ok(json!({"unavailable":true}));
                };
                self.prepare_unified_push(&vapid, replace, now)?;
            }
            "endpoint" => self.set_unified_push_endpoint(
                connection.ok_or(Error::InvalidEvent)?,
                endpoint.ok_or(Error::InvalidEvent)?,
                now,
            )?,
            "receive" => {
                let payload = payload
                    .filter(|p| p.len() <= 5462)
                    .ok_or(Error::InvalidEvent)?;
                let bytes =
                    Base64UrlUnpadded::decode_vec(payload).map_err(|_| Error::InvalidEvent)?;
                let hint =
                    self.receive_unified_push(connection.ok_or(Error::InvalidEvent)?, &bytes, now)?;
                return Ok(json!({"accepted": hint != ReceivedHint::Ignored}));
            }
            "disable" => self.disable_push(now)?,
            "unregistered" => {
                if self
                    .unified_push_registration()?
                    .is_some_and(|current| Some(current.connection.as_str()) == connection)
                {
                    self.disable_push(now)?;
                }
            }
            "retry" => self.retry_push_registration(now)?,
            _ => return Err(Error::InvalidEvent),
        }
        let state = self.push_state()?;
        let connector = self.unified_push_registration()?;
        Ok(
            json!({"configured":state.configured, "choice":match state.choice { Choice::Disabled => "disabled", Choice::Fcm => "fcm", Choice::UnifiedPush => "unified_push" },
            "awaiting_endpoint":state.awaiting_endpoint, "pending":state.configured && (state.pending || state.updating),
            "remote":state.remote.map(|s|s.state), "connection":connector.as_ref().map(|c|&c.connection), "vapid":connector.as_ref().map(|c|&c.vapid_key)}),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unavailable_server_does_not_create_a_registration() {
        let (_dir, _server, _alice, mut bob, _) = crate::claims::tests::pair();
        let result: serde_json::Value =
            serde_json::from_str(&bob.mobile_command(r#"{"command":"push","action":"prepare"}"#))
                .unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["value"]["unavailable"], true);
        assert!(!bob.push_state().unwrap().configured);
        assert!(bob.unified_push_registration().unwrap().is_none());
    }
}

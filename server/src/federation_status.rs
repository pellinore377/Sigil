//! Operator diagnostics contain reservations and aggregate queue state, never
//! packet bodies, user/device identifiers, capabilities or private signing keys.
use crate::{
    federation_config,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub reserved_bytes: u64,
    pub limit_bytes: u64,
    pub available_bytes: u64,
}
impl Budget {
    fn new(reserved_bytes: u64, limit_bytes: u64) -> Self {
        Self {
            reserved_bytes,
            limit_bytes,
            available_bytes: limit_bytes.saturating_sub(reserved_bytes),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub configuration_revision: u64,
    pub enabled: bool,
    pub peers: u64,
    pub peer_metadata: Budget,
    pub nonces: Budget,
    pub ingress: Budget,
    pub egress: Budget,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Block {
    FederationDisabled,
    PeerDisabled,
    DiscoveryFailed,
    DiscoveryRequired,
    ClockBeforeDiscovery,
    DiscoveryExpired,
    PeerCooldown,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    pub pending: u64,
    pub expired: u64,
    pub revoked: u64,
    pub leased: u64,
    pub ready: u64,
    pub retries: u64,
    pub next_attempt: Option<u64>,
    pub errors: std::collections::BTreeMap<String, u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerStatus {
    pub as_of: u64,
    pub configuration_revision: u64,
    pub peer: federation_config::Peer,
    pub block: Option<Block>,
    pub delivery_not_before: u64,
    pub resources: Budget,
    pub nonce_bytes: u64,
    pub ingress_bytes: u64,
    pub egress_bytes: u64,
    pub queue: Queue,
}
impl Store {
    /// Reservation accounting, not physical file sizes or a security readiness claim.
    pub fn federation_status(&mut self) -> Result<Status, StoreError> {
        let tx = self.0.transaction()?;
        let config = federation_config::read(&tx)?;
        let peers: u64 = tx.query_row("SELECT count(*) FROM federation_peers", [], |r| {
            unsigned(r, 0)
        })?;
        let (nonces, ingress, egress): (u64, u64, u64) = tx.query_row(
            "SELECT nonce_bytes,ingress_bytes,egress_bytes FROM federation_usage WHERE id=1",
            [],
            |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?, unsigned(r, 2)?)),
        )?;
        let result = Status {
            configuration_revision: config.revision,
            enabled: config.enabled,
            peers,
            peer_metadata: Budget::new(
                peers
                    .checked_mul(federation_config::PEER_METADATA)
                    .ok_or(StoreError::InvalidData)?,
                federation_config::METADATA_BUDGET,
            ),
            nonces: Budget::new(nonces, crate::federation_admission::GLOBAL_BUDGET),
            ingress: Budget::new(ingress, crate::federation_mailbox::GLOBAL_BUDGET),
            egress: Budget::new(egress, crate::federation_outbox::GLOBAL_BUDGET),
        };
        tx.commit()?;
        Ok(result)
    }
    pub fn federation_peer_status(
        &mut self,
        server: &str,
        now: u64,
    ) -> Result<PeerStatus, StoreError> {
        if !sigil_protocol::valid_server_name(server) {
            return Err(StoreError::Invalid("invalid peer"));
        }
        sql(now)?;
        let tx = self.0.transaction()?;
        let config = federation_config::read(&tx)?;
        let peer = federation_config::peer(&tx, server)?.ok_or(StoreError::NotFound)?;
        let (nonce_bytes, ingress_bytes, egress_bytes, delivery_not_before): (u64,u64,u64,u64) = tx.query_row("SELECT nonce_bytes,ingress_bytes,egress_bytes,delivery_not_before FROM federation_admission WHERE server=?1", [server], |r| Ok((unsigned(r,0)?,unsigned(r,1)?,unsigned(r,2)?,unsigned(r,3)?)))?;
        let block = if !config.enabled {
            Some(Block::FederationDisabled)
        } else if !peer.allowed {
            Some(Block::PeerDisabled)
        } else if peer.error.is_some() {
            Some(Block::DiscoveryFailed)
        } else if peer.pinned.is_none() || peer.checked_at == 0 {
            Some(Block::DiscoveryRequired)
        } else if peer.checked_at > now {
            Some(Block::ClockBeforeDiscovery)
        } else if now - peer.checked_at > 3600 {
            Some(Block::DiscoveryExpired)
        } else if delivery_not_before > now {
            Some(Block::PeerCooldown)
        } else {
            None
        };
        let resources = Budget::new(
            nonce_bytes
                .checked_add(ingress_bytes)
                .and_then(|v| v.checked_add(egress_bytes))
                .ok_or(StoreError::InvalidData)?,
            config.peer_quota_bytes,
        );
        let mut queue = Queue::default();
        {
            let mut query = tx.prepare("SELECT j.due_at,j.expires_at,j.lease_until,j.attempts,CASE WHEN length(j.error)<=64 THEN j.error WHEN j.error IS NOT NULL THEN '' END,d.revoked OR a.disabled FROM federation_outbox j JOIN devices d ON d.id=j.sender JOIN accounts a ON a.id=d.account_id WHERE j.destination=?1 AND j.state=0 ORDER BY j.sender,j.message_id LIMIT ?2")?;
            let mut rows =
                query.query((server, sql(crate::federation_outbox::PENDING_PER_PEER + 1)?))?;
            while let Some(row) = rows.next()? {
                queue.pending += 1;
                if queue.pending > crate::federation_outbox::PENDING_PER_PEER {
                    return Err(StoreError::InvalidData);
                }
                let due = unsigned(row, 0)?;
                let expires = unsigned(row, 1)?;
                let lease = unsigned(row, 2)?;
                let attempts = unsigned(row, 3)?;
                let error: Option<String> = row.get(4)?;
                let revoked: bool = row.get(5)?;
                if attempts > 31 || error.as_deref() == Some("") {
                    return Err(StoreError::InvalidData);
                }
                queue.retries += attempts;
                if let Some(error) = error {
                    *queue.errors.entry(error).or_default() += 1;
                }
                queue.expired += u64::from(expires <= now);
                queue.revoked += u64::from(revoked);
                queue.leased += u64::from(lease > now);
                if !revoked && expires > now {
                    let candidate = due.max(lease).max(delivery_not_before).max(now);
                    // A peer cooldown supplies a known next instant; policy and
                    // discovery failures need operator/background work first.
                    if (block.is_none() || block == Some(Block::PeerCooldown))
                        && candidate < expires
                    {
                        queue.next_attempt = Some(
                            queue
                                .next_attempt
                                .map_or(candidate, |old| old.min(candidate)),
                        );
                    }
                    if block.is_none() && due <= now && lease <= now {
                        queue.ready += 1;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(PeerStatus {
            as_of: now,
            configuration_revision: config.revision,
            peer,
            block,
            delivery_not_before,
            resources,
            nonce_bytes,
            ingress_bytes,
            egress_bytes,
            queue,
        })
    }
}

#[cfg(test)]
#[path = "federation_status_tests.rs"]
mod tests;

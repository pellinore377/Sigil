//! Recipient-specific opaque routing around signed Sender Key packets.
use super::*;
use crate::{connection::decode_id, transport::hex, ClientStore};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use sigil_crypto::storage::StorageKey;
use sigil_protocol::mailbox::{Delivery, Submit};

const PREFIX: &[u8; 8] = b"SGGE\0\x01\0\0";
const MAX: usize = sigil_protocol::mailbox::MAX_PAYLOAD_HEX / 2;
pub(crate) const MIGRATION:&str="
CREATE TABLE group_routes(id BLOB PRIMARY KEY,group_id BLOB NOT NULL UNIQUE REFERENCES group_service(group_id),state BLOB NOT NULL);
CREATE TABLE group_envelopes(id BLOB PRIMARY KEY REFERENCES group_delivery(id),packet BLOB NOT NULL);
PRAGMA user_version=56;";

fn tag(key: &StorageKey, group: &Id, recipient: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[group.as_slice(), recipient].concat(),
        b"Sigil/group-envelope-route/v0",
    )?)
}
fn aad(route: &Id, sender: &Id, recipient: &Id, message: &Id, expires: u64) -> Vec<u8> {
    [
        PREFIX.as_slice(),
        route,
        sender,
        recipient,
        message,
        &expires.to_be_bytes(),
    ]
    .concat()
}
pub(super) fn register(tx: &Transaction<'_>, key: &StorageKey, group: Id) -> Result<(), Error> {
    register_at(tx, key, group, false)
}
fn register_at(
    tx: &Transaction<'_>,
    key: &StorageKey,
    group: Id,
    legacy: bool,
) -> Result<(), Error> {
    let binding = peers::own(tx, key)?;
    let own = device_fingerprint(&binding)?;
    let state = store::load(tx, key, &own, &group)?.state;
    let access = service::load(tx, key, &group, &own, state.authority)?;
    let route = tag(&access.encryption, &group, &own)?;
    let minimum = state
        .epoch
        .checked_add(u64::from(legacy))
        .ok_or(Error::Limit)?;
    tx.execute(
        "INSERT INTO group_routes VALUES(?1,?2,?3)",
        (
            route.as_slice(),
            group.as_slice(),
            key.seal(&minimum.to_be_bytes(), &crate::binding(56, &own, &group))?,
        ),
    )?;
    Ok(())
}
pub(crate) fn migrate(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    let groups = tx
        .prepare("SELECT group_id FROM group_service ORDER BY group_id")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for group in groups {
        register_at(
            tx,
            key,
            group.try_into().map_err(|_| Error::InvalidStore)?,
            true,
        )?;
    }
    Ok(())
}
pub(super) fn legacy_allowed(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
    epoch: u64,
) -> Result<bool, Error> {
    let sealed: Vec<u8> = tx.query_row(
        "SELECT CASE WHEN length(state)=44 THEN state END FROM group_routes WHERE group_id=?1",
        [group.as_slice()],
        |r| r.get(0),
    )?;
    let minimum = u64::from_be_bytes(
        key.open(&sealed, &crate::binding(56, own, group))?
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?,
    );
    Ok(epoch < minimum)
}
pub(crate) fn is_envelope(payload: &str) -> bool {
    payload
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("53474745"))
}
impl ClientStore {
    pub(crate) fn wrap_group_request(
        &mut self,
        group: Id,
        recipient: Id,
        request: &mut Submit,
    ) -> Result<(), Error> {
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let fields = peers::parse(&binding)?.binding;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_service WHERE group_id=?1)",
            [group.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        let status = store::load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        status.state.device(own)?;
        let member = status
            .state
            .members
            .iter()
            .flat_map(|m| &m.devices)
            .find(|d| peers::fingerprint(&d.binding).ok() == Some(recipient))
            .ok_or(Error::Unprepared)?;
        if hex(&member.binding.device) != request.recipient_device {
            return Err(Error::Conflict);
        }
        let access = service::load(&tx, &self.key, &group, &own, status.state.authority)?;
        let route = tag(&access.encryption, &group, &recipient)?;
        let message = decode_id(&request.message_id)?;
        let associated = aad(
            &route,
            &fields.device,
            &member.binding.device,
            &message,
            request.expires_at,
        );
        let raw = service::decode(&request.payload, MAX)?;
        let context = sigil_crypto::sender_keys::Packet::from_bytes(&raw)?.context();
        keys::matches_state(&status.state, &context)?;
        if context.sender != own {
            return Err(Error::Conflict);
        }
        // An upgrade cannot change a request that may already be on the server.
        // Existing groups switch formats at their next authenticated key epoch.
        if legacy_allowed(&tx, &self.key, &own, &group, context.epoch)? {
            return Ok(());
        }
        let previous:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(packet)<=?2 THEN packet END FROM group_envelopes WHERE id=?1",(message.as_slice(),MAX as u32),|r|r.get(0)).optional()?;
        let packet = if let Some(previous) = previous {
            if previous.len() < 76
                || &previous[..8] != PREFIX
                || previous[8..40] != route
                || access
                    .encryption
                    .open(&previous[40..], &associated)?
                    .as_slice()
                    != raw
            {
                return Err(Error::InvalidStore);
            }
            previous
        } else {
            let sealed = access.encryption.seal(&raw, &associated)?;
            let packet = [PREFIX.as_slice(), &route, &sealed].concat();
            if packet.len() > MAX {
                return Err(Error::Limit);
            }
            tx.execute(
                "INSERT INTO group_envelopes VALUES(?1,?2)",
                (message.as_slice(), &packet),
            )?;
            packet
        };
        tx.commit()?;
        request.payload = hex(&packet);
        Ok(())
    }
    pub(crate) fn unwrap_group_delivery(
        &mut self,
        delivery: &Delivery,
    ) -> Result<(Id, Delivery), Error> {
        let packet = service::decode(&delivery.payload, MAX)?;
        if packet.len() < 76 || &packet[..8] != PREFIX {
            return Err(Error::InvalidEvent);
        }
        let route: Id = packet[8..40].try_into().map_err(|_| Error::InvalidEvent)?;
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let fields = peers::parse(&binding)?.binding;
        let tx = self.db.transaction()?;
        let group: Vec<u8> = tx
            .query_row(
                "SELECT group_id FROM group_routes WHERE id=?1",
                [route.as_slice()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::Unprepared)?;
        let group: Id = group.try_into().map_err(|_| Error::InvalidStore)?;
        let status = store::load(&tx, &self.key, &own, &group)?;
        let access = service::load(&tx, &self.key, &group, &own, status.state.authority)?;
        if tag(&access.encryption, &group, &own)? != route {
            return Err(Error::InvalidStore);
        }
        let raw = access.encryption.open(
            &packet[40..],
            &aad(
                &route,
                &decode_id(&delivery.sender_device)?,
                &fields.device,
                &decode_id(&delivery.message_id)?,
                delivery.expires_at,
            ),
        )?;
        if sigil_crypto::sender_keys::Packet::from_bytes(&raw)?
            .context()
            .group
            != group
        {
            return Err(Error::Conflict);
        }
        Ok((
            group,
            Delivery {
                origin: delivery.origin.clone(),
                sequence: delivery.sequence,
                sender_device: delivery.sender_device.clone(),
                message_id: delivery.message_id.clone(),
                payload: hex(&raw),
                expires_at: delivery.expires_at,
            },
        ))
    }
}

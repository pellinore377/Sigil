//! Invited devices may verify encrypted membership history, never send as members.
use super::*;
use sigil_protocol::groups::{Invitation, InvitationState};

pub(super) fn apply(
    tx: &Transaction<'_>,
    group: &[u8; 32],
    author: &[u8; 64],
    request: (&str, &str, u64),
    now: u64,
) -> Result<Invitation, StoreError> {
    let (request_id, target, expires) = request;
    let invitation = id(request_id)?;
    let target = decode(target, 64)?;
    sigil_crypto::private_credentials::validate_ciphertext(&target)
        .map_err(|_| StoreError::Invalid("invalid invited identity"))?;
    if invitation == [0; 32] {
        return Err(StoreError::Invalid("invalid invitation lifetime"));
    }
    tx.execute("UPDATE private_group_invitations SET status=3 WHERE group_id=?1 AND status=0 AND expires_at<=?2",(group.as_slice(),sql(now)?))?;
    type Row = (Vec<u8>, Vec<u8>, u64, u8);
    let previous: Option<Row> = tx.query_row("SELECT author,target,expires_at,status FROM private_group_invitations WHERE group_id=?1 AND id=?2",(group.as_slice(),invitation.as_slice()),|r|Ok((r.get(0)?,r.get(1)?,unsigned(r,2)?,r.get(3)?))).optional()?;
    let status = if let Some((creator, recipient, lifetime, status)) = previous {
        if recipient != target || (expires != 0 && (creator != author || expires != lifetime)) {
            return Err(StoreError::Conflict);
        }
        if expires == 0 && status == 0 {
            tx.execute(
                "UPDATE private_group_invitations SET status=2 WHERE group_id=?1 AND id=?2",
                (group.as_slice(), invitation.as_slice()),
            )?;
            2
        } else {
            status
        }
    } else {
        if expires != 0 && (expires <= now || expires > now.saturating_add(604800)) {
            return Err(StoreError::Invalid("invalid invitation lifetime"));
        }
        let status = if expires == 0 { 2 } else { 0 };
        if status == 0 {
            let pending: u32 = tx.query_row(
                "SELECT count(*) FROM private_group_invitations WHERE group_id=?1 AND status=0",
                [group.as_slice()],
                |r| r.get(0),
            )?;
            if pending >= MAX_DEVICES as u32 {
                return Err(StoreError::Busy);
            }
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM private_group_members WHERE group_id=?1 AND ciphertext=?2) OR EXISTS(SELECT 1 FROM private_group_invitations WHERE group_id=?1 AND target=?2 AND status=0)",(group.as_slice(),&target),|r|r.get::<_,bool>(0))? { return Err(StoreError::Conflict); }
        }
        reserve(tx, 384)?;
        tx.execute(
            "INSERT INTO private_group_invitations VALUES(?1,?2,?3,?4,?5,?6)",
            (
                group.as_slice(),
                invitation.as_slice(),
                author.as_slice(),
                &target,
                sql(expires)?,
                status,
            ),
        )?;
        status
    };
    Ok(Invitation {
        id: request_id.into(),
        state: match status {
            0 => InvitationState::Pending,
            1 => InvitationState::Consumed,
            2 => InvitationState::Cancelled,
            3 => InvitationState::Expired,
            _ => return Err(StoreError::InvalidData),
        },
    })
}
pub(super) fn may_read(
    tx: &Transaction<'_>,
    group: &[u8; 32],
    target: &[u8; 64],
    now: u64,
) -> Result<bool, StoreError> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM private_group_invitations i JOIN private_group_members m ON m.group_id=i.group_id AND m.ciphertext=i.author WHERE i.group_id=?1 AND i.target=?2 AND i.status=0 AND i.expires_at>?3 AND m.admin=1)",(group.as_slice(),target.as_slice(),sql(now)?),|r|r.get(0))?)
}
pub(super) fn may_cancel(
    tx: &Transaction<'_>,
    group: &[u8; 32],
    id: &[u8; 32],
    target: &[u8; 64],
    now: u64,
) -> Result<bool, StoreError> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM private_group_invitations WHERE group_id=?1 AND id=?2 AND target=?3 AND status IN(0,2) AND expires_at>?4)",(group.as_slice(),id.as_slice(),target.as_slice(),sql(now)?),|r|r.get(0))?)
}
pub(super) fn cutover(tx: &Transaction<'_>, group: &[u8; 32]) -> Result<(), StoreError> {
    tx.execute("UPDATE private_group_invitations SET status=1 WHERE group_id=?1 AND status=0 AND target IN(SELECT ciphertext FROM private_group_members WHERE group_id=?1)",[group.as_slice()])?;
    tx.execute("UPDATE private_group_invitations SET status=2 WHERE group_id=?1 AND status=0 AND author NOT IN(SELECT ciphertext FROM private_group_members WHERE group_id=?1 AND admin=1)",[group.as_slice()])?;
    Ok(())
}

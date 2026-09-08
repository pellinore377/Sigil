use crate::{
    federation_auth::{bytes32, hex},
    group_authority::{self as authority, crypto, decode, reserve},
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_crypto::{
    group_receipt::Receipt,
    private_group::{Operation as Kind, Request as Context},
};
use sigil_protocol::groups::{Commit, Member, Operation, Reply, Request, MAX_CONTROL, MAX_DEVICES};
type RelayRecord = (Vec<u8>, Vec<u8>, u64, Vec<u8>);
#[path = "group_invitations.rs"]
mod invitations;

fn relayed(
    db: &rusqlite::Connection,
    group: &[u8; 32],
    author: &[u8; 64],
) -> Result<Option<RelayRecord>, StoreError> {
    Ok(db.query_row("SELECT predecessor,head,revision,CASE WHEN length(control)<=196608 THEN control END FROM private_group_proposals WHERE group_id=?1 AND author=?2",(group.as_slice(),author.as_slice()),|r|Ok((r.get(0)?,r.get(1)?,unsigned(r,2)?,r.get(3)?))).optional()?)
}

fn id(value: &str) -> Result<[u8; 32], StoreError> {
    bytes32(value).map_err(|_| StoreError::Invalid("invalid group reference"))
}

fn members(values: &[Member]) -> Result<Vec<(Vec<u8>, bool)>, StoreError> {
    if values.len() > MAX_DEVICES {
        return Err(StoreError::Invalid("group has too many devices"));
    }
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let bytes = decode(&value.ciphertext, 64)?;
        sigil_crypto::private_credentials::validate_ciphertext(&bytes)
            .map_err(|_| StoreError::Invalid("invalid encrypted member"))?;
        if result.last().is_some_and(|(old, _)| old >= &bytes) {
            return Err(StoreError::Invalid("members must be unique and sorted"));
        }
        result.push((bytes, value.admin));
    }
    if !result.is_empty() && !result.iter().any(|(_, admin)| *admin) {
        return Err(StoreError::Invalid("live group needs an administrator"));
    }
    Ok(result)
}

fn release(tx: &Transaction<'_>, bytes: u64) -> Result<(), StoreError> {
    if tx.execute(
        "UPDATE group_authority SET used=used-?1 WHERE id=1 AND used>=?1",
        [sql(bytes)?],
    )? != 1
    {
        return Err(StoreError::InvalidData);
    }
    Ok(())
}

fn admit_nonce(
    tx: &Transaction<'_>,
    nonce: &[u8; 32],
    expires: u64,
    now: u64,
) -> Result<(), StoreError> {
    let expired=tx.prepare("SELECT nonce FROM private_group_nonces WHERE expires_at<=?1 ORDER BY expires_at,nonce LIMIT 64")?.query_map([sql(now)?],|r|r.get::<_,Vec<u8>>(0))?.collect::<Result<Vec<_>,_>>()?;
    for nonce in expired {
        tx.execute("DELETE FROM private_group_nonces WHERE nonce=?1", [nonce])?;
        release(tx, 128)?;
    }
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM private_group_nonces WHERE nonce=?1)",
        [nonce.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(StoreError::Conflict);
    }
    reserve(tx, 128)?;
    tx.execute(
        "INSERT INTO private_group_nonces VALUES(?1,?2)",
        (nonce.as_slice(), sql(expires)?),
    )?;
    Ok(())
}

fn commit(
    db: &rusqlite::Connection,
    group: &[u8; 32],
    revision: u64,
) -> Result<Commit, StoreError> {
    let (head,control,receipt):(Vec<u8>,Vec<u8>,Option<Vec<u8>>)=db.query_row("SELECT head,CASE WHEN length(control)<=196608 THEN control END,receipt FROM private_group_commits WHERE group_id=?1 AND revision=?2",(group.as_slice(),sql(revision)?),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if head.len() != 32 || receipt.as_ref().is_some_and(|v| v.len() != 208) {
        return Err(StoreError::InvalidData);
    }
    Ok(Commit {
        revision,
        head: hex(&head),
        control: hex(&control),
        receipt: receipt.map(|v| hex(&v)),
    })
}

impl Store {
    pub fn group_request(&mut self, request: Request, now: u64) -> Result<Reply, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = request_in(&tx, request, now)?;
        tx.commit()?;
        Ok(result)
    }
}
pub(crate) fn request_in(
    tx: &Transaction<'_>,
    request: Request,
    now: u64,
) -> Result<Reply, StoreError> {
    if now == 0 || now > i64::MAX as u64 || request.day as u64 != now / 86400 {
        return Err(StoreError::Invalid("invalid group request date"));
    }
    let group = id(&request.group)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM private_groups WHERE id=?1 AND blocked=1)",
        [group.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(StoreError::Forbidden);
    }
    let nonce = id(&request.nonce)?;
    let proof = decode(&request.proof, 480)?;
    let operation_bytes =
        serde_json::to_vec(&request.operation).map_err(|_| StoreError::InvalidData)?;
    if operation_bytes.len() > sigil_protocol::groups::MAX_BODY {
        return Err(StoreError::Invalid("group operation too large"));
    }
    let hash: [u8; 32] = Sha256::digest(&operation_bytes).into();
    let (kind, predecessor) = match &request.operation {
        Operation::Create { .. } => (Kind::Create, [0; 32]),
        Operation::Advance { predecessor, .. } | Operation::Relay { predecessor, .. } => {
            (Kind::Advance, id(predecessor)?)
        }
        Operation::Read { .. } | Operation::Proposals { .. } => (Kind::Read, [0; 32]),
        Operation::Invite { .. } => (Kind::Advance, [0; 32]),
    };
    let stored = authority::read(tx)?;
    if !stored.enabled {
        return Err(StoreError::Forbidden);
    }
    let profile = stored.authority.ok_or(StoreError::InvalidData)?;
    if request.authority != hex(&profile.id()) {
        return Err(StoreError::Conflict);
    }
    let context = Context {
        operation: kind,
        group,
        predecessor,
        body_hash: hash,
        nonce,
        expires_at: request.expires_at,
    }
    .context(&profile, now)
    .map_err(|_| StoreError::Invalid("invalid group request context"))?;
    type State = (Vec<u8>, u64, Vec<u8>, bool);
    let existing: Option<State> = tx
        .query_row(
            "SELECT public,revision,head,restored FROM private_groups WHERE id=?1",
            [group.as_slice()],
            |r| Ok((r.get(0)?, unsigned(r, 1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let public = match (&existing, &request.operation) {
        (Some((public, ..)), _) => public
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::InvalidData)?,
        (None, Operation::Create { public, .. }) => id(public)?,
        _ => return Err(StoreError::NotFound),
    };
    let author = stored
        .issuer
        .ok_or(StoreError::InvalidData)?
        .verify_presentation(public, request.day, &context, &proof)
        .map_err(|_| StoreError::Unauthorized)?;
    // A leaving member can recover only its exact submitted proposal's
    // ordering receipt, even after removal. This does not grant roster reads.
    if let Operation::Relay {
        predecessor: requested_predecessor,
        head,
        revision,
        control,
    } = &request.operation
    {
        let retained = relayed(tx, &group, &author)?;
        if let Some((p, h, r, c)) = retained {
            if r == *revision
                && hex(&p) == *requested_predecessor
                && hex(&h) == *head
                && hex(&c) == *control
                && existing.as_ref().is_some_and(|e| e.1 >= r)
            {
                let record = commit(tx, &group, r)?;
                if record.head != *head || record.control != *control {
                    return Err(StoreError::Conflict);
                }
                admit_nonce(tx, &nonce, request.expires_at, now)?;
                let result = Reply {
                    authority: request.authority,
                    group: request.group,
                    revision: r,
                    head: record.head.clone(),
                    restored: existing.as_ref().is_some_and(|e| e.3),
                    commits: vec![record],
                    proposals: Vec::new(),
                    invitation: None,
                };
                return Ok(result);
            }
        }
    }
    let requested_revision = match &request.operation {
        Operation::Create { .. } => Some(0),
        Operation::Advance { revision, .. } => Some(*revision),
        Operation::Read { .. }
        | Operation::Relay { .. }
        | Operation::Proposals { .. }
        | Operation::Invite { .. } => None,
    };
    if let Some(revision) = requested_revision {
        let previous:Option<(Vec<u8>,Vec<u8>)>=tx.query_row("SELECT operation_hash,author FROM private_group_commits WHERE group_id=?1 AND revision=?2",(group.as_slice(),sql(revision)?),|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((old_hash, old_author)) = previous {
            if old_hash != hash || old_author != author {
                return Err(StoreError::Conflict);
            }
            admit_nonce(tx, &nonce, request.expires_at, now)?;
            let record = commit(tx, &group, revision)?;
            let result = Reply {
                authority: request.authority,
                group: request.group,
                revision,
                head: record.head.clone(),
                restored: existing.as_ref().is_some_and(|v| v.3),
                commits: vec![record],
                proposals: Vec::new(),
                invitation: None,
            };
            return Ok(result);
        }
    }
    let authorized: Option<bool> = tx
        .query_row(
            "SELECT admin FROM private_group_members WHERE group_id=?1 AND ciphertext=?2",
            (group.as_slice(), author.as_slice()),
            |r| r.get(0),
        )
        .optional()?;
    let mut result = Reply {
        authority: request.authority.clone(),
        group: request.group.clone(),
        revision: 0,
        head: String::new(),
        restored: false,
        commits: Vec::new(),
        proposals: Vec::new(),
        invitation: None,
    };
    match &request.operation {
        Operation::Invite {
            id,
            target,
            expires_at,
        } => {
            if authorized != Some(true) {
                let own_cancel = *expires_at == 0 && decode(target, 64)? == author;
                if !own_cancel
                    || !invitations::may_cancel(tx, &group, &self::id(id)?, &author, now)?
                {
                    return Err(StoreError::Forbidden);
                }
            }
            let (_, revision, head, restored) = existing.ok_or(StoreError::NotFound)?;
            if restored {
                return Err(StoreError::Conflict);
            }
            result.invitation = Some(invitations::apply(
                tx,
                &group,
                &author,
                (id, target, *expires_at),
                now,
            )?);
            result.revision = revision;
            result.head = hex(&head);
        }
        Operation::Relay {
            head,
            revision,
            control,
            ..
        } => {
            if authorized.is_none() {
                return Err(StoreError::Forbidden);
            }
            let (_, current, old_head, restored) = existing.ok_or(StoreError::NotFound)?;
            if restored
                || *revision != current.checked_add(1).ok_or(StoreError::InvalidData)?
                || old_head != predecessor
            {
                return Err(StoreError::Conflict);
            }
            let head = id(head)?;
            let control = decode(control, MAX_CONTROL)?;
            if head == [0; 32] || head == predecessor || control.is_empty() {
                return Err(StoreError::Invalid("invalid group proposal"));
            }
            let prior = relayed(tx, &group, &author)?;
            let prior = if let Some((_, _, r, ref c)) = prior {
                if r <= current {
                    release(tx, 512 + c.len() as u64)?;
                    tx.execute(
                        "DELETE FROM private_group_proposals WHERE group_id=?1 AND author=?2",
                        (group.as_slice(), author.as_slice()),
                    )?;
                    None
                } else {
                    prior
                }
            } else {
                None
            };
            if let Some((p, h, r, c)) = prior {
                if p != predecessor || h != head || r != *revision || c != control {
                    return Err(StoreError::Conflict);
                }
            } else {
                reserve(tx, 512 + control.len() as u64)?;
                tx.execute(
                    "INSERT INTO private_group_proposals VALUES(?1,?2,?3,?4,?5,?6)",
                    (
                        group.as_slice(),
                        author.as_slice(),
                        predecessor.as_slice(),
                        head.as_slice(),
                        sql(*revision)?,
                        &control,
                    ),
                )?;
            }
            result.revision = current;
            result.head = hex(&old_head);
            result.proposals.push(sigil_protocol::groups::Relayed {
                author: hex(&author),
                predecessor: hex(&predecessor),
                head: hex(&head),
                revision: *revision,
                control: hex(&control),
            });
        }
        Operation::Proposals { after } => {
            if authorized != Some(true) {
                return Err(StoreError::Forbidden);
            }
            let (_, revision, head, restored) = existing.ok_or(StoreError::NotFound)?;
            let after = after
                .as_ref()
                .map(|v| decode(v, 64))
                .transpose()?
                .unwrap_or_default();
            if !after.is_empty() && after.len() != 64 {
                return Err(StoreError::Invalid("invalid proposal cursor"));
            }
            result.revision = revision;
            result.head = hex(&head);
            result.restored = restored;
            result.proposals=tx.prepare("SELECT author,predecessor,head,revision,CASE WHEN length(control)<=196608 THEN control END FROM private_group_proposals WHERE group_id=?1 AND author>?2 AND revision>?3 ORDER BY author LIMIT 2")?.query_map((group.as_slice(),after,sql(revision)?),|r|Ok(sigil_protocol::groups::Relayed {author:hex(&r.get::<_,Vec<u8>>(0)?),predecessor:hex(&r.get::<_,Vec<u8>>(1)?),head:hex(&r.get::<_,Vec<u8>>(2)?),revision:unsigned(r,3)?,control:hex(&r.get::<_,Vec<u8>>(4)?)}))?.collect::<Result<Vec<_>,_>>()?;
        }
        Operation::Read { from_revision } => {
            if authorized.is_none() && !invitations::may_read(tx, &group, &author, now)? {
                return Err(StoreError::Forbidden);
            }
            let (_, revision, head, restored) = existing.ok_or(StoreError::NotFound)?;
            if *from_revision > revision.saturating_add(1) {
                return Err(StoreError::Conflict);
            }
            result.revision = revision;
            result.head = hex(&head);
            result.restored = restored;
            let revisions=tx.prepare("SELECT revision FROM private_group_commits WHERE group_id=?1 AND revision>=?2 ORDER BY revision LIMIT 2")?.query_map((group.as_slice(),sql(*from_revision)?),|r|unsigned(r,0))?.collect::<Result<Vec<_>,_>>()?;
            for revision in revisions {
                result.commits.push(commit(tx, &group, revision)?);
            }
        }
        Operation::Create {
            public: requested_public,
            head,
            control,
            members: roster,
        } => {
            if existing.is_some() {
                return Err(StoreError::Conflict);
            }
            if id(requested_public)? != public {
                return Err(StoreError::Conflict);
            }
            let roster = members(roster)?;
            if roster.len() != 1 || roster[0].0 != author || !roster[0].1 {
                return Err(StoreError::Forbidden);
            }
            let head = id(head)?;
            let control = decode(control, MAX_CONTROL)?;
            if head == [0; 32] || control.is_empty() {
                return Err(StoreError::Invalid("empty group creation"));
            }
            reserve(tx, 1536 + control.len() as u64)?;
            tx.execute(
                "INSERT INTO private_groups(id,public,revision,head,restored) VALUES(?1,?2,0,?3,0)",
                (group.as_slice(), public.as_slice(), head.as_slice()),
            )?;
            tx.execute(
                "INSERT INTO private_group_members VALUES(?1,?2,1)",
                (group.as_slice(), author.as_slice()),
            )?;
            tx.execute(
                "INSERT INTO private_group_commits VALUES(?1,0,?2,?3,?4,?5,NULL)",
                (
                    group.as_slice(),
                    head.as_slice(),
                    hash.as_slice(),
                    author.as_slice(),
                    control,
                ),
            )?;
            result.head = hex(&head);
            result.commits.push(commit(tx, &group, 0)?);
        }
        Operation::Advance {
            head,
            revision,
            control,
            members: roster,
            ..
        } => {
            let (_, current, old_head, restored) = existing.ok_or(StoreError::NotFound)?;
            if restored {
                return Err(StoreError::Conflict);
            }
            if *revision != current.checked_add(1).ok_or(StoreError::InvalidData)?
                || old_head != predecessor
            {
                return Err(StoreError::Conflict);
            }
            let head = id(head)?;
            if head == predecessor || head == [0; 32] {
                return Err(StoreError::Invalid("invalid group head"));
            }
            let roster = members(roster)?;
            let old=tx.prepare("SELECT ciphertext,admin FROM private_group_members WHERE group_id=?1 ORDER BY ciphertext LIMIT 1025")?.query_map([group.as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,bool>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
            if old.len() > MAX_DEVICES {
                return Err(StoreError::InvalidData);
            }
            // Opaque controls cannot establish member-level authorization
            // here. An administrator must validate and submit every change,
            // including member-signed leave/device proposals.
            if authorized != Some(true) {
                return Err(StoreError::Forbidden);
            }
            let control = decode(control, MAX_CONTROL)?;
            if control.is_empty() {
                return Err(StoreError::Invalid("empty group control"));
            }
            let receipt = Receipt::sign(
                group,
                predecessor,
                head,
                *revision,
                stored.signing.as_ref().ok_or(StoreError::InvalidData)?,
            )
            .map_err(crypto)?
            .to_bytes();
            if old.len() > roster.len() {
                release(tx, ((old.len() - roster.len()) * 512) as u64)?;
            }
            reserve(
                tx,
                512 + control.len() as u64 + (roster.len().saturating_sub(old.len()) * 512) as u64,
            )?;
            tx.execute(
                "DELETE FROM private_group_members WHERE group_id=?1",
                [group.as_slice()],
            )?;
            for (cipher, admin) in roster {
                tx.execute(
                    "INSERT INTO private_group_members VALUES(?1,?2,?3)",
                    (group.as_slice(), cipher, admin),
                )?;
            }
            invitations::cutover(tx, &group)?;
            tx.execute(
                "INSERT INTO private_group_commits VALUES(?1,?2,?3,?4,?5,?6,?7)",
                (
                    group.as_slice(),
                    sql(*revision)?,
                    head.as_slice(),
                    hash.as_slice(),
                    author.as_slice(),
                    &control,
                    receipt,
                ),
            )?;
            tx.execute(
                "UPDATE private_groups SET revision=?2,head=?3 WHERE id=?1",
                (group.as_slice(), sql(*revision)?, head.as_slice()),
            )?;
            let released: u64 = tx.query_row("SELECT coalesce(sum(512+length(control)),0) FROM private_group_proposals WHERE group_id=?1 AND revision=?3 AND NOT (head=?2 AND control=?4)",(group.as_slice(),head.as_slice(),sql(*revision)?,&control),|r|unsigned(r,0))?;
            tx.execute(
                    "DELETE FROM private_group_proposals WHERE group_id=?1 AND revision=?3 AND NOT (head=?2 AND control=?4)",
                    (group.as_slice(),head.as_slice(),sql(*revision)?,&control),
                )?;
            release(tx, released)?;
            result.revision = *revision;
            result.head = hex(&head);
            result.commits.push(commit(tx, &group, *revision)?);
        }
    }
    admit_nonce(tx, &nonce, request.expires_at, now)?;
    Ok(result)
}

#[cfg(test)]
#[path = "group_authority_tests.rs"]
mod tests;

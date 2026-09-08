use super::*;
const CHUNK: usize = 16384;

fn remember_origin(
    tx: &Transaction<'_>,
    key: &StorageKey,
    device: &Id,
    transfer: &Id,
    entry: Id,
) -> Result<(), Error> {
    let id = fragment_id(key, device, transfer, u32::MAX)?;
    let raw: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)=68 THEN state END FROM conversation_transfer_origins WHERE id=?1", [id.as_slice()], |r| r.get(0)).optional()?;
    if let Some(raw) = raw {
        if key
            .open(&raw, &binding(105, &id, b"sync origin"))?
            .as_slice()
            != entry
        {
            return Err(Error::Conflict);
        }
    } else {
        tx.execute(
            "INSERT INTO conversation_transfer_origins VALUES(?1,?2)",
            (
                id.as_slice(),
                key.seal(&entry, &binding(105, &id, b"sync origin"))?,
            ),
        )?;
    }
    tx.execute(
        "DELETE FROM conversation_transfer_parts WHERE id=?1",
        [id.as_slice()],
    )?;
    Ok(())
}

pub(crate) fn journal(
    tx: &Transaction<'_>,
    key: &StorageKey,
    kind: u8,
    session: Id,
    message: Id,
    event: &sigil_protocol::event::Direct<'_>,
) -> Result<(), Error> {
    let Content::Conversation(raw) = event.content else {
        return Ok(());
    };
    let op = Operation::from_bytes(raw).map_err(|_| Error::InvalidStore)?;
    let Action::SyncPart {
        transfer,
        index: part,
        total,
        digest,
        ..
    } = &op.action
    else {
        return Ok(());
    };
    let id = fragment_id(key, &op.version.device, transfer, u32::MAX)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_transfer_origins WHERE id=?1)",
        [id.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let reference = [&[kind], session.as_slice(), message.as_slice()].concat();
    let context = binding(106, &id, &part.to_be_bytes());
    tx.execute(
        "INSERT INTO conversation_transfer_parts VALUES(?1,?2,?3) ON CONFLICT(id,part) DO NOTHING",
        (id.as_slice(), part, key.seal(&reference, &context)?),
    )?;
    if tx.query_row(
        "SELECT count(*) FROM conversation_transfer_parts WHERE id=?1",
        [id.as_slice()],
        |r| r.get::<_, u32>(0),
    )? < *total
    {
        return Ok(());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    for n in 0..*total {
        let reference: Vec<u8> = tx.query_row("SELECT CASE WHEN length(state)=101 THEN state END FROM conversation_transfer_parts WHERE id=?1 AND part=?2", (id.as_slice(),n), |r| r.get(0))?;
        let reference = key.open(&reference, &binding(106, &id, &n.to_be_bytes()))?;
        if reference.len() != 65 || reference[0] > 1 {
            return Err(Error::InvalidStore);
        }
        let session: Id = reference[1..33]
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        let message: Id = reference[33..]
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        let (table, domain) = if reference[0] == 0 {
            ("inbox", 2)
        } else {
            ("outbox", 9)
        };
        let stored: Vec<u8> = tx.query_row(&format!("SELECT CASE WHEN length(content)<=65572 THEN content END FROM {table} WHERE session=?1 AND id=?2"), (session.as_slice(),message.as_slice()), |r| r.get(0))?;
        let stored = key.open(&stored, &binding(domain, &session, &message))?;
        let direct =
            sigil_protocol::event::Direct::from_bytes(&stored).map_err(|_| Error::InvalidStore)?;
        let Content::Conversation(raw) = direct.content else {
            return Err(Error::InvalidStore);
        };
        let old = Operation::from_bytes(raw).map_err(|_| Error::InvalidStore)?;
        let Action::SyncPart {
            transfer: old_transfer,
            index,
            total: old_total,
            digest: old_digest,
            payload,
        } = old.action
        else {
            return Err(Error::InvalidStore);
        };
        if old.version.device != op.version.device
            || old_transfer != *transfer
            || index != n
            || old_total != *total
            || old_digest != *digest
        {
            return Err(Error::InvalidStore);
        }
        for pair in payload.as_bytes().as_chunks::<2>().0 {
            bytes.push(
                u8::from_str_radix(
                    std::str::from_utf8(pair).map_err(|_| Error::InvalidStore)?,
                    16,
                )
                .map_err(|_| Error::InvalidStore)?,
            );
        }
        if bytes.len() > 32 * CHUNK {
            return Err(Error::Limit);
        }
    }
    if <Id>::from(Sha256::digest(&bytes)) != *digest {
        return Err(Error::InvalidStore);
    }
    let entry: Entry = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
    entry
        .operation
        .validate()
        .map_err(|_| Error::InvalidStore)?;
    remember_origin(
        tx,
        key,
        &op.version.device,
        transfer,
        entry_id(key, &entry)?,
    )
}

pub(crate) fn require_part(db: &Connection, key: &StorageKey, op: &Operation) -> Result<(), Error> {
    let Action::SyncPart { transfer, .. } = &op.action else {
        return Ok(());
    };
    let id = fragment_id(key, &op.version.device, transfer, u32::MAX)?;
    let raw: Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state)=68 THEN state END FROM conversation_transfer_origins WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
    if let Some(raw) = raw {
        let entry: Id = key
            .open(&raw, &binding(105, &id, b"sync origin"))?
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        if retention::removed(db, key, &entry)?.is_some() {
            return Err(Error::Obsolete);
        }
    }
    Ok(())
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Cursor {
    after: i64,
    transfer: Option<Transfer>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Transfer {
    sequence: i64,
    entry: Id,
    id: Id,
    digest: Id,
    total: u32,
    next: u32,
    pending: Option<Operation>,
}
#[derive(Serialize, Deserialize)]
struct Receiving {
    digest: Id,
    total: u32,
    hashes: Vec<Option<Id>>,
    complete: bool,
}
fn aad(id: &Id) -> Vec<u8> {
    binding(92, id, b"conversation sync")
}
fn read<T: serde::de::DeserializeOwned>(
    db: &Connection,
    key: &StorageKey,
    table: &str,
    id: &Id,
) -> Result<Option<T>, Error> {
    let raw: Option<Vec<u8>> = db
        .query_row(
            &format!(
                "SELECT CASE WHEN length(state)<=67266 THEN state END FROM {table} WHERE id=?1"
            ),
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|v| serde_json::from_slice(&key.open(&v, &aad(id))?).map_err(|_| Error::InvalidStore))
        .transpose()
}
fn save<T: Serialize>(
    db: &Connection,
    key: &StorageKey,
    table: &str,
    id: &Id,
    value: &T,
) -> Result<(), Error> {
    let raw = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
    db.execute(
        &format!(
            "INSERT INTO {table} VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state"
        ),
        (id.as_slice(), key.seal(&raw, &aad(id))?),
    )?;
    Ok(())
}
fn fragment_id(key: &StorageKey, device: &Id, transfer: &Id, part: u32) -> Result<Id, Error> {
    index(
        key,
        b"Sigil/conversation-fragment/v0",
        &[device, transfer, &part.to_be_bytes()],
    )
}
fn finish(
    db: &Connection,
    key: &StorageKey,
    peer: &Id,
    expected: &Cursor,
    next: &Cursor,
) -> Result<(), Error> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if read::<Cursor>(&tx, key, "conversation_sync", peer)?.as_ref() == Some(expected) {
        save(&tx, key, "conversation_sync", peer, next)?;
    }
    tx.commit()?;
    Ok(())
}
pub(super) fn receive(tx: &Transaction<'_>, key: &StorageKey, op: &Operation) -> Result<(), Error> {
    let Action::SyncPart {
        transfer,
        digest,
        index: part,
        total,
        payload,
    } = &op.action
    else {
        return Err(Error::InvalidEvent);
    };
    let id = fragment_id(key, &op.version.device, transfer, u32::MAX)?;
    let mut state =
        read::<Receiving>(tx, key, "conversation_fragments", &id)?.unwrap_or(Receiving {
            digest: *digest,
            total: *total,
            hashes: vec![None; *total as usize],
            complete: false,
        });
    if state.digest != *digest || state.total != *total || state.hashes.len() != *total as usize {
        return Err(Error::Conflict);
    }
    let raw = Zeroizing::new(
        payload
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| {
                u8::from_str_radix(std::str::from_utf8(v).map_err(|_| Error::InvalidEvent)?, 16)
                    .map_err(|_| Error::InvalidEvent)
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    if *part + 1 < *total && raw.len() != CHUNK {
        return Err(Error::InvalidEvent);
    }
    let hash: Id = Sha256::digest(&raw).into();
    let slot = &mut state.hashes[*part as usize];
    if let Some(old) = slot {
        return if *old == hash {
            Ok(())
        } else {
            Err(Error::Conflict)
        };
    }
    if state.complete {
        return Err(Error::InvalidStore);
    }
    *slot = Some(hash);
    let at = fragment_id(key, &op.version.device, transfer, *part)?;
    tx.execute(
        "INSERT INTO conversation_fragments VALUES(?1,?2)",
        (at.as_slice(), key.seal(&raw, &aad(&at))?),
    )?;
    if state.hashes.iter().all(Option::is_some) {
        let mut bytes = Zeroizing::new(Vec::new());
        for n in 0..*total {
            let at = fragment_id(key, &op.version.device, transfer, n)?;
            let raw:Vec<u8>=tx.query_row("SELECT CASE WHEN length(state)<=16420 THEN state END FROM conversation_fragments WHERE id=?1",[at.as_slice()],|r|r.get(0))?;
            let raw = key.open(&raw, &aad(&at))?;
            if Some(<Id>::from(Sha256::digest(&raw))) != state.hashes[n as usize] {
                return Err(Error::InvalidStore);
            }
            bytes.extend_from_slice(&raw);
        }
        if <Id>::from(Sha256::digest(&bytes)) != *digest {
            return Err(Error::InvalidEvent);
        }
        let e: Entry = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidEvent)?;
        if e.timestamp == 0
            || e.timestamp > i64::MAX as u64
            || e.operation.ephemeral()
            || matches!(e.operation.action, Action::SyncPart { .. })
        {
            return Err(Error::InvalidEvent);
        }
        e.operation.validate().map_err(|_| Error::InvalidEvent)?;
        let own = event::account(&peers::parse(&peers::own(tx, key)?)?.binding);
        if let Action::Private { conversation, .. } = &e.operation.action {
            if e.author != own || *conversation != e.conversation {
                return Err(Error::InvalidEvent);
            }
        }
        if e.author == own {
            observe(tx, key, e.operation.version.counter)?;
        }
        remember_origin(tx, key, &op.version.device, transfer, entry_id(key, &e)?)?;
        ingest(tx, key, e)?;
        for n in 0..*total {
            tx.execute(
                "DELETE FROM conversation_fragments WHERE id=?1",
                [fragment_id(key, &op.version.device, transfer, n)?.as_slice()],
            )?;
        }
        state.complete = true;
    }
    save(tx, key, "conversation_fragments", &id, &state)
}
impl ClientStore {
    /// Every pass queues at most sixteen fragments across linked devices.
    pub fn sync_conversation_devices(&mut self, now: u64) -> Result<usize, Error> {
        let own = self.own_device_binding()?;
        let account = event::account(&peers::parse(&own)?.binding);
        let device = device_fingerprint(&own)?;
        let position =
            read::<Id>(&self.db, &self.key, "conversation_sync", &[0; 32])?.unwrap_or([0; 32]);
        let ids = self
            .db
            .prepare("SELECT id FROM peers WHERE obsolete=0 AND id>?1 ORDER BY id LIMIT 16")?
            .query_map([position.as_slice()], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut count = 0;
        let mut last = [0; 32];
        for id in ids {
            let peer: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            last = peer;
            let known = peers::known(&self.db, &self.key, &peer)?;
            if !known.verified
                || known.fingerprint == device
                || event::account(&known.binding) != account
            {
                continue;
            }
            for _ in 0..16 {
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut cursor =
                    read::<Cursor>(&tx, &self.key, "conversation_sync", &peer)?.unwrap_or_default();
                if cursor.transfer.is_none() {
                    let deferred:Option<(i64,Vec<u8>,Vec<u8>)>=tx.query_row("SELECT ?2,e.id,CASE WHEN length(e.state)<=574000 THEN e.state END FROM conversation_sync_deferred d JOIN conversation_ops e ON e.id=d.entry JOIN conversation_ops p ON p.id=d.original WHERE d.peer=?1 ORDER BY d.entry LIMIT 1",(peer.as_slice(),cursor.after),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                    let row=match deferred {Some(row)=>Some(row),None=>tx.query_row("SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE rowid>?1 ORDER BY rowid LIMIT 1",[cursor.after],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?};
                    let Some((sequence, id, raw)) = row else {
                        break;
                    };
                    let at: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
                    let e = open(&self.key, &at, &raw)?;
                    tx.execute(
                        "DELETE FROM conversation_sync_deferred WHERE peer=?1 AND entry=?2",
                        (peer.as_slice(), at.as_slice()),
                    )?;
                    let allowed = copyable(&tx, &self.key, &e)?;
                    if allowed.is_none() {
                        let Action::Edit { target, .. } = &e.operation.action else {
                            return Err(Error::InvalidStore);
                        };
                        let original = index(
                            &self.key,
                            b"Sigil/conversation-event/v0",
                            &[&e.conversation, &target.author, &target.message],
                        )?;
                        tx.execute(
                            "INSERT INTO conversation_sync_deferred VALUES(?1,?2,?3)",
                            (peer.as_slice(), at.as_slice(), original.as_slice()),
                        )?;
                    }
                    if e.operation.version.device == known.fingerprint || allowed != Some(true) {
                        cursor.after = sequence;
                        save(&tx, &self.key, "conversation_sync", &peer, &cursor)?;
                        tx.commit()?;
                        continue;
                    }
                    let bytes =
                        Zeroizing::new(serde_json::to_vec(&e).map_err(|_| Error::InvalidStore)?);
                    if bytes.len() > 32 * CHUNK {
                        return Err(Error::Limit);
                    }
                    let id: Id = Sha256::digest(
                        [
                            b"Sigil/conversation-sync/v0".as_slice(),
                            &device,
                            &peer,
                            &e.conversation,
                            &e.author,
                            &e.operation.id,
                        ]
                        .concat(),
                    )
                    .into();
                    cursor.transfer = Some(Transfer {
                        sequence,
                        entry: at,
                        id,
                        digest: Sha256::digest(&bytes).into(),
                        total: bytes.len().div_ceil(CHUNK) as u32,
                        next: 0,
                        pending: None,
                    });
                    remember_origin(&tx, &self.key, &device, &id, at)?;
                }
                let transfer = cursor.transfer.as_mut().ok_or(Error::InvalidStore)?;
                if super::retention::removed(&tx, &self.key, &transfer.entry)?.is_some() {
                    cursor.after = transfer.sequence;
                    cursor.transfer = None;
                    save(&tx, &self.key, "conversation_sync", &peer, &cursor)?;
                    tx.commit()?;
                    continue;
                }
                if transfer.pending.is_none() {
                    let raw:Vec<u8>=tx.query_row("SELECT CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE id=?1",[transfer.entry.as_slice()],|r|r.get(0))?;
                    let e = open(&self.key, &transfer.entry, &raw)?;
                    let bytes =
                        Zeroizing::new(serde_json::to_vec(&e).map_err(|_| Error::InvalidStore)?);
                    if <Id>::from(Sha256::digest(&bytes)) != transfer.digest {
                        return Err(Error::Conflict);
                    }
                    let chunk = bytes
                        .chunks(CHUNK)
                        .nth(transfer.next as usize)
                        .ok_or(Error::InvalidStore)?;
                    let counter = clock(&tx, &self.key)?
                        .checked_add(1)
                        .filter(|v| *v <= i64::MAX as u64)
                        .ok_or(Error::Limit)?;
                    let id: Id = Sha256::digest(
                        [
                            b"Sigil/conversation-sync-part/v0".as_slice(),
                            &transfer.id,
                            &transfer.next.to_be_bytes(),
                        ]
                        .concat(),
                    )
                    .into();
                    let op = Operation {
                        id,
                        version: Version { device, counter },
                        action: Action::SyncPart {
                            transfer: transfer.id,
                            digest: transfer.digest,
                            index: transfer.next,
                            total: transfer.total,
                            payload: transport::hex(chunk),
                        },
                    };
                    op.to_bytes().map_err(|_| Error::Limit)?;
                    observe(&tx, &self.key, counter)?;
                    transfer.pending = Some(op);
                }
                save(&tx, &self.key, "conversation_sync", &peer, &cursor)?;
                tx.commit()?;
                let expected = cursor.clone();
                let transfer = cursor.transfer.as_mut().ok_or(Error::InvalidStore)?;
                let op = transfer.pending.as_ref().ok_or(Error::InvalidStore)?;
                self.queue_peer_operation(peer, op, 1, now)?;
                transfer.pending = None;
                transfer.next += 1;
                if transfer.next == transfer.total {
                    cursor.after = transfer.sequence;
                    cursor.transfer = None;
                }
                finish(&self.db, &self.key, &peer, &expected, &cursor)?;
                count += 1;
                if count == 16 {
                    break;
                }
            }
            if count == 16 {
                break;
            }
        }
        save(&self.db, &self.key, "conversation_sync", &[0; 32], &last)?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_conversation_sync_completion_cannot_rewind_another_worker() {
        let (dir, _fixture, a, _, _) = crate::claims::tests::pair();
        let peer = [1; 32];
        let first = Cursor {
            after: 10,
            transfer: None,
        };
        save(&a.db, &a.key, "conversation_sync", &peer, &first).unwrap();
        let b = ClientStore::open(
            &dir.path().join("alice.db"),
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap();
        let stalled = read::<Cursor>(&b.db, &b.key, "conversation_sync", &peer)
            .unwrap()
            .unwrap();
        let second = Cursor {
            after: 20,
            transfer: None,
        };
        finish(&a.db, &a.key, &peer, &first, &second).unwrap();
        let third = Cursor {
            after: 30,
            transfer: None,
        };
        finish(&a.db, &a.key, &peer, &second, &third).unwrap();
        finish(&b.db, &b.key, &peer, &stalled, &second).unwrap();
        assert_eq!(
            read::<Cursor>(&b.db, &b.key, "conversation_sync", &peer)
                .unwrap()
                .unwrap()
                .after,
            30
        );
    }
}

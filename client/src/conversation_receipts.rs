use super::*;
#[derive(Serialize, Deserialize)]
struct Work {
    destination: Destination,
    operation: Operation,
    timestamp: u64,
    done: bool,
}
fn aad(id: &Id) -> Vec<u8> {
    binding(97, id, b"delivery receipt")
}
pub(crate) fn schedule(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    destination: Destination,
    conversation: Id,
    author: Id,
    content: Content<'_>,
) -> Result<(), Error> {
    let Content::Conversation(raw) = content else {
        return Ok(());
    };
    let op = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
    let Action::Post { expires_at, .. } = op.action else {
        return Ok(());
    };
    let fields = peers::parse(own)?.binding;
    if author == event::account(&fields) || expires_at.is_some_and(|v| v <= now()) {
        return Ok(());
    }
    let device = peers::fingerprint(&fields)?;
    let id: Id = Sha256::digest(
        [
            b"Sigil/automatic-delivery-receipt/v0".as_slice(),
            &device,
            &conversation,
            &author,
            &op.id,
        ]
        .concat(),
    )
    .into();
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_receipts WHERE id=?1)",
        [id.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let counter = clock(tx, key)?
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(Error::Limit)?;
    let operation = Operation {
        id,
        version: Version { device, counter },
        action: Action::Receipt {
            target: Reference {
                author,
                message: op.id,
            },
            read: false,
        },
    };
    let work = Work {
        destination,
        operation,
        timestamp: now(),
        done: false,
    };
    save(tx, key, &id, &work)?;
    observe(tx, key, counter)
}
fn save(db: &Connection, key: &StorageKey, id: &Id, work: &Work) -> Result<(), Error> {
    let raw = Zeroizing::new(serde_json::to_vec(work).map_err(|_| Error::InvalidStore)?);
    db.execute("INSERT INTO conversation_receipts VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,done=excluded.done",(id.as_slice(),key.seal(&raw,&aad(id))?,work.done))?;
    Ok(())
}
impl ClientStore {
    pub fn resume_conversation_receipts(&mut self, now: u64) -> Result<usize, Error> {
        let raw:Option<Vec<u8>>=self.db.query_row("SELECT CASE WHEN length(state)=44 THEN state END FROM conversation_receipt_cursor WHERE id=1",[],|r|r.get(0)).optional()?;
        let after = raw
            .map(|v| {
                self.key
                    .open(&v, &aad(&[0; 32]))?
                    .as_slice()
                    .try_into()
                    .map(i64::from_be_bytes)
                    .map_err(|_| Error::InvalidStore)
            })
            .transpose()?
            .unwrap_or(0);
        let rows=self.db.prepare("SELECT rowid,id,CASE WHEN length(state)<=4096 THEN state END FROM conversation_receipts WHERE done=0 AND rowid>?1 ORDER BY rowid LIMIT 16")?.query_map([after],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
        let mut count = 0;
        let mut next = 0;
        for (sequence, id, raw) in rows {
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            let mut work: Work = serde_json::from_slice(&self.key.open(&raw, &aad(&id))?)
                .map_err(|_| Error::InvalidStore)?;
            if work.done || work.operation.id != id {
                return Err(Error::InvalidStore);
            }
            let result = match work.destination {
                Destination::Peer(peer) => {
                    self.queue_peer_operation(peer, &work.operation, work.timestamp, now)
                }
                Destination::Group(group) => {
                    self.queue_group_operation(group, &work.operation, work.timestamp, now)
                }
            };
            match result {
                Ok(()) => {
                    work.done = true;
                    count += 1;
                }
                Err(Error::Obsolete | Error::Cancelled) => work.done = true,
                Err(Error::Unprepared) => {}
                Err(e) => return Err(e),
            }
            save(&self.db, &self.key, &id, &work)?;
            next = sequence;
        }
        self.db.execute("INSERT INTO conversation_receipt_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",[self.key.seal(&next.to_be_bytes(),&aad(&[0;32]))?])?;
        Ok(count)
    }
}

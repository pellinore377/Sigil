//! Live page editing: a server-ordered base plus transformed inflight and pending batches give the speculative page.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sigil_crypto::{
    page::{Batch, Context, PageKey},
    IdentityKey,
};
use sigil_pages::{
    apply_batch, apply_local, apply_local_batch, invert_batch,
    ops::{table_cell, table_shape},
    transform_batch, Action, Attrs, Block, Delta, Kind, Line, MetaOp, Op, Page,
};
use sigil_protocol::pages::{
    self as wire, Append, Bytes, Cursor, Grant, PageId, PutPresence, PutSnapshot, Request, Sealed,
    Slot, Status,
};

/// Ops between snapshots; a reopen replays at most this many.
pub const SNAPSHOT_EVERY: u64 = if cfg!(test) { 24 } else { 300 };
/// Undo depth; every edit transforms the whole stack.
const MAX_UNDO: usize = 200;
/// Plaintext ops per sealed batch.
pub const MAX_BATCH: usize = Batch::MAX_OPS;
const COPY_SUFFIX: &str = " (offline copy)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// No answer at all; the request may or may not have landed.
    Unreachable,
    Refused {
        status: u16,
        code: String,
    },
    Invalid,
}
impl NetError {
    pub fn is(&self, name: &str) -> bool {
        matches!(self, Self::Refused { code, .. } if code == name)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PageError {
    Net(NetError),
    /// An op or snapshot is sealed under an epoch this device holds no key for.
    NeedsKey(u64),
    Invalid(&'static str),
}
impl From<NetError> for PageError {
    fn from(e: NetError) -> Self {
        Self::Net(e)
    }
}
impl From<&'static str> for PageError {
    fn from(e: &'static str) -> Self {
        Self::Invalid(e)
    }
}
impl From<sigil_crypto::Error> for PageError {
    fn from(_: sigil_crypto::Error) -> Self {
        Self::Invalid("page crypto failed")
    }
}
type Result<T> = std::result::Result<T, PageError>;

/// The page routes; tokens travel in headers.
pub trait PageServer {
    fn create(&self, request: &wire::Create) -> std::result::Result<Status, NetError>;
    fn append(
        &self,
        id: &PageId,
        access: &[u8; 32],
        request: &Append,
    ) -> std::result::Result<wire::Appended, NetError>;
    /// `wait` seconds of long-poll; zero answers at once.
    fn ops(
        &self,
        id: &PageId,
        access: &[u8; 32],
        after: u64,
        wait: u64,
    ) -> std::result::Result<wire::Ops, NetError>;
    fn snapshot(
        &self,
        id: &PageId,
        access: &[u8; 32],
    ) -> std::result::Result<wire::Snapshot, NetError>;
    /// False when the server already held an equal or newer one.
    fn put_snapshot(
        &self,
        id: &PageId,
        access: &[u8; 32],
        request: &PutSnapshot,
    ) -> std::result::Result<bool, NetError>;
    fn put_presence(
        &self,
        id: &PageId,
        access: &[u8; 32],
        request: &PutPresence,
    ) -> std::result::Result<(), NetError>;
    fn presence(
        &self,
        id: &PageId,
        access: &[u8; 32],
    ) -> std::result::Result<Vec<wire::PresenceEntry>, NetError>;
}

/// This device's batch signing identity.
pub struct Signer {
    identity: IdentityKey,
    device: [u8; 32],
}
impl Signer {
    pub fn new(identity: IdentityKey, device: [u8; 32]) -> Self {
        Self { identity, device }
    }
    pub fn device(&self) -> [u8; 32] {
        self.device
    }
    fn copy(&self) -> Self {
        Self::new(
            IdentityKey::from_private_bytes(&self.identity.private_bytes()),
            self.device,
        )
    }
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| PageError::Invalid("no randomness"))?;
    Ok(bytes)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn now() -> u64 {
    crate::clock::SystemTime::now()
        .duration_since(crate::clock::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1)
        .max(1)
}
fn size(ops: &[Op]) -> usize {
    serde_json::to_vec(ops)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
}
fn changes_nothing(ops: &[Op]) -> bool {
    ops.iter().all(Op::is_noop)
}

/// A batch on its way: sealed once, so a retry is byte-identical under its request name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Inflight {
    pub(crate) ops: Vec<Op>,
    pub(crate) base: u64,
    pub(crate) epoch: u64,
    pub(crate) request: Request,
    pub(crate) body: Sealed,
}

/// The editing state of one page; no I/O.
pub(crate) struct Doc {
    /// State at `head`, server ops only.
    pub(crate) base: Page,
    /// `base` with inflight and pending applied.
    pub(crate) current: Page,
    pub(crate) head: u64,
    pub(crate) snapshot_seq: u64,
    /// Effective batches since `snapshot_seq`, by seq.
    pub(crate) window: Vec<(u64, Vec<Op>)>,
    pub(crate) pending: Vec<Op>,
    pub(crate) inflight: Option<Inflight>,
    /// Scalar caret.
    pub(crate) cursor: u32,
    undo: Vec<Vec<Op>>,
    redo: Vec<Vec<Op>>,
    patch: bool,
    prints: Option<Vec<u64>>,
    views: u64,
}

impl Doc {
    pub(crate) fn new(base: Page, head: u64) -> Self {
        Doc {
            current: base.clone(),
            base,
            head,
            snapshot_seq: head,
            window: Vec::new(),
            pending: Vec::new(),
            inflight: None,
            cursor: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            patch: false,
            prints: None,
            views: 0,
        }
    }
    pub(crate) fn rebuild(&mut self) -> Result<()> {
        let mut current = self.base.clone();
        if let Some(inflight) = &self.inflight {
            apply_batch(&mut current, &inflight.ops)?;
        }
        apply_batch(&mut current, &self.pending)?;
        self.current = current;
        self.clamp();
        Ok(())
    }
    fn clamp(&mut self) {
        self.cursor = self
            .cursor
            .min(self.current.body.target_len().saturating_sub(1));
    }
    pub(crate) fn local(&self) -> bool {
        self.inflight.is_some() || self.pending.iter().any(|op| !op.is_noop())
    }
    /// A stored batch transformed over the effective batches in `(base, seq)`.
    fn effective(
        &self,
        ops: Vec<Op>,
        base: u64,
        seq: u64,
    ) -> std::result::Result<Vec<Op>, &'static str> {
        if base < self.snapshot_seq {
            return Err("op base precedes the snapshot");
        }
        let mut ops = ops;
        for (_, earlier) in self.window.iter().filter(|(s, _)| *s > base && *s < seq) {
            ops = transform_batch(&ops, earlier)?.0;
        }
        Ok(ops)
    }
    /// Our own batch landed at `seq`: it already sits on top of every batch before it.
    fn fold_own(&mut self, seq: u64) -> Result<()> {
        let Some(inflight) = self.inflight.take() else {
            return Err(PageError::Invalid("no batch in flight"));
        };
        let mut next = self.base.clone();
        // Past the replay limit every device steps over it; so does its author.
        if apply_batch(&mut next, &inflight.ops).is_ok() {
            self.base = next;
            self.window.push((seq, inflight.ops));
        } else {
            self.window.push((seq, Vec::new()));
        }
        self.head = seq;
        if self.rebuild().is_err() {
            self.pending.clear();
            self.rebuild()?;
        }
        Ok(())
    }
    /// Apply one server-ordered batch after `head`; `None` or an unusable batch is stepped over alike everywhere.
    pub(crate) fn receive(
        &mut self,
        seq: u64,
        base: u64,
        request: Option<(&Request, u64)>,
        ops: Option<Vec<Op>>,
    ) -> Result<()> {
        if seq <= self.head {
            return Ok(());
        }
        if seq != self.head + 1 {
            return Err(PageError::Invalid("op sequence gap"));
        }
        if let (Some((request, epoch)), Some(inflight)) = (request, &self.inflight) {
            if inflight.request == *request && inflight.epoch == epoch && inflight.base == base {
                return self.fold_own(seq);
            }
        }
        let step = ops.and_then(|ops| {
            let effective = self.effective(ops, base, seq).ok()?;
            let mut next = self.base.clone();
            apply_batch(&mut next, &effective).ok()?;
            Some((effective, next))
        });
        let Some((carried, base_next)) = step else {
            self.window.push((seq, Vec::new()));
            self.head = seq;
            return Ok(());
        };
        // Local work is carried over it; nothing below may fail half way.
        let mut remote = carried.clone();
        let mut inflight_next = None;
        if let Some(inflight) = &self.inflight {
            let (moved, over) = transform_batch(&inflight.ops, &remote)?;
            inflight_next = Some(moved);
            remote = over;
        }
        let (pending, remote) = transform_batch(&self.pending, &remote)?;
        if let (Some(moved), Some(inflight)) = (inflight_next, self.inflight.as_mut()) {
            inflight.ops = moved;
        }
        self.pending = pending;
        self.base = base_next;
        self.window.push((seq, carried));
        self.head = seq;
        self.carry(&remote);
        for op in &remote {
            if let Op::Delta { delta } = op {
                self.cursor = delta.transform_cursor(self.cursor);
            }
        }
        // Pending past the replay limit is dropped, as in `fold_own`; head and base stay consistent.
        if self.rebuild().is_err() {
            self.pending.clear();
            self.rebuild()?;
        }
        Ok(())
    }
    /// Carry both stacks over an op applied to `current`; emptied or unusable entries drop.
    fn carry(&mut self, ops: &[Op]) {
        for stack in [&mut self.undo, &mut self.redo] {
            *stack = stack
                .iter()
                .filter_map(|entry| transform_batch(entry, ops).ok().map(|(moved, _)| moved))
                .filter(|moved| !changes_nothing(moved))
                .collect();
        }
    }
    fn edit(&mut self, op: Op) -> Result<()> {
        if size(std::slice::from_ref(&op)) > MAX_BATCH {
            return Err(PageError::Invalid("edit too large"));
        }
        apply_local(&mut self.current, &op)?;
        self.carry(std::slice::from_ref(&op));
        self.pending.retain(|op| !op.is_noop());
        let merged = match (self.pending.last(), &op) {
            (Some(Op::Delta { delta }), Op::Delta { delta: next }) => {
                delta.compose(next).ok().map(Op::delta)
            }
            (
                Some(Op::Cell { cell, delta }),
                Op::Cell {
                    cell: at,
                    delta: next,
                },
            ) if cell == at => delta.compose(next).ok().map(|joined| Op::Cell {
                cell: cell.clone(),
                delta: joined,
            }),
            _ => None,
        }
        .filter(|merged| size(std::slice::from_ref(merged)) <= MAX_BATCH);
        match merged {
            Some(merged) => {
                self.pending.pop();
                self.pending.push(merged);
            }
            None => self.pending.push(op),
        }
        Ok(())
    }
    /// One command: apply its ops and push their inverse.
    pub(crate) fn commit(&mut self, ops: Vec<Op>) -> Result<()> {
        let back = invert_batch(&self.current, &ops)?;
        if ops.len() > 1 {
            apply_local_batch(&mut self.current.clone(), &ops)?;
        }
        for op in ops {
            self.edit(op)?;
        }
        if !changes_nothing(&back) {
            self.undo.push(back);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        Ok(())
    }
    /// Undo or redo one entry; entries that no longer fit are dropped.
    pub(crate) fn step(&mut self, redo: bool) -> Result<()> {
        let (entry, back) = loop {
            let stack = if redo { &mut self.redo } else { &mut self.undo };
            let entry = stack.pop().ok_or(PageError::Invalid(if redo {
                "nothing to redo"
            } else {
                "nothing to undo"
            }))?;
            let mut trial = self.current.clone();
            if apply_local_batch(&mut trial, &entry).is_err() {
                continue;
            }
            if let Ok(back) = invert_batch(&self.current, &entry) {
                break (entry, back);
            }
        };
        let caret = caret(&entry);
        for op in entry {
            self.edit(op)?;
        }
        if !changes_nothing(&back) {
            let stack = if redo { &mut self.undo } else { &mut self.redo };
            stack.push(back);
            if stack.len() > MAX_UNDO {
                stack.remove(0);
            }
        }
        if let Some(at) = caret {
            self.cursor = at;
        }
        self.clamp();
        Ok(())
    }
    /// Batches to send: a prefix of pending that fits one sealed batch.
    fn take_batch(&mut self) -> Result<Vec<Op>> {
        self.pending.retain(|op| !op.is_noop());
        let mut count = 0;
        while count < self.pending.len() && size(&self.pending[..=count]) <= MAX_BATCH {
            count += 1;
        }
        if count == 0 && !self.pending.is_empty() {
            return Err(PageError::Invalid("edit too large to send"));
        }
        Ok(self.pending.drain(..count).collect())
    }
    /// Back to pending: the server never stored it.
    fn requeue(&mut self) {
        if let Some(mut inflight) = self.inflight.take() {
            inflight.ops.append(&mut self.pending);
            self.pending = inflight.ops;
        }
    }
    pub(crate) fn view(&mut self) -> Value {
        let mut out = json!(sigil_pages::view(&self.current, self.head, self.cursor));
        let prints: Vec<u64> = sigil_pages::prints(&self.current.body)
            .into_iter()
            .map(|p| p.0)
            .collect();
        if self.patch {
            if let Some(old) = &self.prints {
                let (body, _) = sigil_pages::body_patch(&self.current.body, self.cursor, old);
                out["body"] = body;
                out["since"] = json!(self.views);
            }
        }
        self.views += 1;
        out["view"] = json!(self.views);
        out["can_undo"] = json!(!self.undo.is_empty());
        out["can_redo"] = json!(!self.redo.is_empty());
        out["pending"] = json!(self.local());
        self.prints = Some(prints);
        out
    }
    pub(crate) fn restart(&mut self, patch: bool) -> Value {
        self.patch = patch;
        self.prints = None;
        self.view()
    }
}

/// The caret an undo entry leaves: the end of the first change of its first delta.
fn caret(ops: &[Op]) -> Option<u32> {
    ops.iter().find_map(|op| {
        let Op::Delta { delta } = op else { return None };
        let mut at = 0u32;
        for part in serde_json::to_value(delta).ok()?.as_array()? {
            if let Some(n) = part.get("retain").and_then(Value::as_u64) {
                if part.get("attrs").is_some() {
                    return Some(at);
                }
                at = at.saturating_add(n as u32);
            } else if let Some(insert) = part.get("insert") {
                let len = insert.as_str().map_or(1, |s| s.chars().count() as u32);
                return Some(at + len);
            } else {
                return Some(at);
            }
        }
        Some(at)
    })
}

/// What a sync left: the view, whether the server answered, and any offline copy to save.
#[derive(Debug)]
pub struct Synced {
    pub view: Value,
    pub offline: bool,
    pub changed: bool,
    /// Local edits that could not be carried past compaction, as a page of their own.
    pub copy: Option<Page>,
    /// A newer epoch exists; the device needs its grant.
    pub stale_key: Option<u64>,
}

/// One open page: an in-memory session that never touches the client store.
pub struct PageSession {
    pub(crate) id: PageId,
    pub(crate) host: String,
    /// Keys by epoch, newest last.
    keys: Vec<PageKey>,
    access: [u8; 32],
    signer: Signer,
    pub(crate) doc: Doc,
    slot: Slot,
    pub(crate) unsaved: bool,
    /// Revision and head of the stored cache this session extends.
    pub(crate) cache: Option<([u8; 16], u64)>,
    stale: Option<u64>,
}

impl PageSession {
    pub(crate) fn from_doc(grant: &Grant, signer: Signer, doc: Doc) -> Result<Self> {
        Ok(Self {
            id: Bytes(grant.id),
            host: grant.host.clone(),
            keys: vec![PageKey::from_grant(grant)?],
            access: grant.access,
            signer,
            doc,
            slot: Bytes(random()?),
            unsaved: false,
            cache: None,
            stale: None,
        })
    }
    /// A new page on the server, sealed as snapshot 0; the grant carries the manage token.
    pub fn create(
        net: &impl PageServer,
        host: &str,
        page: Page,
        signer: Signer,
        expires_at: Option<u64>,
    ) -> Result<(Grant, Self)> {
        page.check()?;
        let id = random::<32>()?;
        let key = PageKey::generate(id, 1)?;
        let (access, manage) = (*sigil_crypto::page::token()?, *sigil_crypto::page::token()?);
        let body = seal_page(&key, 0, &page)?;
        net.create(&wire::Create {
            id: Bytes(id),
            access_hash: Bytes(sigil_crypto::page::token_hash(&access)),
            manage_hash: Bytes(sigil_crypto::page::token_hash(&manage)),
            expires_at,
            snapshot: body,
        })?;
        let grant = Grant {
            host: host.into(),
            id,
            epoch: 1,
            key: *key.key_bytes(),
            access,
            manage: Some(manage),
        };
        let session = Self::from_doc(&grant, signer, Doc::new(page, 0))?;
        Ok((grant, session))
    }
    /// Fetches the newest snapshot and every op after it.
    pub fn open(net: &impl PageServer, grant: &Grant, signer: Signer) -> Result<Self> {
        let mut session = Self::from_doc(grant, signer, Doc::new(Page::new(Kind::Page, ""), 0))?;
        session.reload(net)?;
        Ok(session)
    }
    pub fn id(&self) -> [u8; 32] {
        self.id.0
    }
    pub fn epoch(&self) -> u64 {
        self.keys.last().map_or(0, PageKey::epoch)
    }
    pub fn head(&self) -> u64 {
        self.doc.head
    }
    pub fn page(&self) -> &Page {
        &self.doc.current
    }
    /// Server-ordered state only.
    pub fn base(&self) -> &Page {
        &self.doc.base
    }
    /// Unsent or unconfirmed local edits.
    pub fn has_local_edits(&self) -> bool {
        self.doc.local()
    }
    /// Local work changed since the last `ClientStore::page_save`.
    pub fn unsaved(&self) -> bool {
        self.unsaved
    }
    /// Adds the key of another epoch; a newer one also replaces the access token.
    pub fn add_grant(&mut self, grant: &Grant) -> Result<()> {
        if grant.id != self.id.0 {
            return Err(PageError::Invalid("grant for another page"));
        }
        if self.keys.iter().any(|k| k.epoch() == grant.epoch) {
            return Ok(());
        }
        if grant.epoch > self.epoch() {
            self.access = grant.access;
            self.host = grant.host.clone();
            if self.stale.is_some_and(|e| e <= grant.epoch) {
                self.stale = None;
            }
        }
        self.keys.push(PageKey::from_grant(grant)?);
        self.keys.sort_by_key(PageKey::epoch);
        Ok(())
    }
    fn key(&self, epoch: u64) -> Result<&PageKey> {
        self.keys
            .iter()
            .find(|k| k.epoch() == epoch)
            .ok_or(PageError::NeedsKey(epoch))
    }
    fn current_key(&self) -> Result<&PageKey> {
        self.keys.last().ok_or(PageError::Invalid("no page key"))
    }

    pub fn view(&mut self) -> Value {
        self.doc.view()
    }
    /// A whole view; with `patch`, later ones carry only changed paragraphs.
    pub fn restart(&mut self, patch: bool) -> Value {
        self.doc.restart(patch)
    }
    fn local_edit(&mut self, ops: Vec<Op>, caret: Option<u32>) -> Result<Value> {
        self.doc.commit(ops)?;
        if let Some(at) = caret {
            self.doc.cursor = at;
        }
        self.doc.clamp();
        self.unsaved = true;
        Ok(self.doc.view())
    }
    fn range(&self, start: usize, end: usize) -> Result<(u32, u32)> {
        let body = &self.doc.current.body;
        let last = body.target_len().saturating_sub(1);
        let (start, end) = (body.scalar_from_utf16(start), body.scalar_from_utf16(end));
        if start > end {
            return Err(PageError::Invalid("range out of bounds"));
        }
        Ok((start.min(last), end.min(last)))
    }
    /// Replaces the UTF-16 range with `text`; typed newlines take the paragraph's attributes.
    pub fn edit(&mut self, start: usize, end: usize, text: &str) -> Result<Value> {
        let (start, end) = self.range(start, end)?;
        let body = &self.doc.current.body;
        let run: Attrs = match start.checked_sub(1).and_then(|i| body.scalar_at(i)) {
            Some((ch, attrs, _)) if ch != '\n' && ch != sigil_pages::EMBED_CHAR => attrs
                .iter()
                .filter(|(k, _)| k.as_str() != "link")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Attrs::new(),
        };
        let para = body
            .newline_from(end)
            .map(|(_, a)| a.clone())
            .unwrap_or_default();
        let len = body.target_len();
        let mut delta = Delta::new().retain(start).delete(end - start);
        for (i, piece) in text.split('\n').enumerate() {
            if i > 0 {
                delta = delta.insert_with("\n", para.clone());
            }
            delta = delta.insert_with(piece, run.clone());
        }
        let delta = delta.retain(len - end);
        let caret = start + text.chars().count() as u32;
        self.local_edit(vec![Op::delta(delta)], Some(caret))
    }
    /// Run attributes over the range, newlines excluded.
    pub fn format(&mut self, start: usize, end: usize, attrs: Attrs) -> Result<Value> {
        let (start, end) = self.range(start, end)?;
        let body = &self.doc.current.body;
        let mut delta = Delta::new().retain(start);
        for i in start..end {
            let newline = body.scalar_at(i).is_some_and(|(ch, _, _)| ch == '\n');
            delta = if newline {
                delta.retain(1)
            } else {
                delta.retain_with(1, attrs.clone())
            };
        }
        let delta = delta.retain(body.target_len() - end);
        self.local_edit(vec![Op::delta(delta)], None)
    }
    /// Paragraph attributes on every paragraph touching the range.
    pub fn paragraph(&mut self, start: usize, end: usize, attrs: Attrs) -> Result<Value> {
        let (start, end) = self.range(start, end)?;
        let text = self.doc.current.body.text();
        let (first, last) = (
            sigil_pages::paragraph_at(&text, start).0,
            sigil_pages::paragraph_at(&text, end).1,
        );
        let mut delta = Delta::new();
        for (i, ch) in text.chars().enumerate() {
            let i = i as u32;
            delta = if i >= first && i < last && ch == '\n' {
                delta.retain_with(1, attrs.clone())
            } else {
                delta.retain(1)
            };
        }
        self.local_edit(vec![Op::delta(delta)], None)
    }
    /// An embed at the UTF-16 offset; tables and blocks get a fresh id when they have none.
    pub fn insert(&mut self, at: usize, mut embed: Value) -> Result<Value> {
        for kind in ["table", "block"] {
            if let Some(object) = embed.get_mut(kind).and_then(Value::as_object_mut) {
                if !object.contains_key("id") {
                    object.insert("id".into(), json!(hex(&random::<16>()?)));
                }
            }
        }
        let body = &self.doc.current.body;
        let len = body.target_len();
        let at = body.scalar_from_utf16(at).min(len.saturating_sub(1));
        let delta = Delta::new()
            .retain(at)
            .embed(embed, Attrs::new())
            .retain(len - at);
        self.local_edit(vec![Op::delta(delta)], Some(at + 1))
    }
    pub fn block_set(&mut self, id: &str, source: &str) -> Result<Value> {
        let op = Op::BlockSet {
            block_set: Block {
                id: id.into(),
                source: source.into(),
            },
        };
        self.local_edit(vec![op], None)
    }
    /// Replaces a UTF-16 range of one cell.
    pub fn table_edit(
        &mut self,
        table: &str,
        row: u32,
        col: u32,
        start: usize,
        end: usize,
        text: &str,
    ) -> Result<Value> {
        if text.contains('\n') {
            return Err(PageError::Invalid("a table cell holds one paragraph"));
        }
        let cell = table_cell(&self.doc.current, table, row, col)
            .ok_or(PageError::Invalid("no such cell"))?;
        let len = cell.target_len();
        let (start, end) = (
            cell.scalar_from_utf16(start).min(len),
            cell.scalar_from_utf16(end).min(len),
        );
        if start > end {
            return Err(PageError::Invalid("range out of bounds"));
        }
        let delta = Delta::replace(len, start, end, text)?;
        self.local_edit(vec![Op::cell(table, row, col, delta)], None)
    }
    /// One row or column inserted or deleted.
    pub fn table_line(
        &mut self,
        table: &str,
        row: bool,
        at: u32,
        action: Action,
        width: Option<Value>,
    ) -> Result<Value> {
        let (rows, cols) =
            table_shape(&self.doc.current, table).ok_or(PageError::Invalid("no such table"))?;
        let have = if row { rows } else { cols };
        let limit = if action == Action::Insert {
            have
        } else {
            have.saturating_sub(1)
        };
        if at as usize > limit {
            return Err(PageError::Invalid("no such row or column"));
        }
        let mut line = Line::new(table, at, action);
        line.width = width.filter(|_| !row && action == Action::Insert);
        let op = if row {
            Op::TableRow { table_row: line }
        } else {
            Op::TableCol { table_col: line }
        };
        self.local_edit(vec![op], None)
    }
    pub fn meta(&mut self, meta: MetaOp) -> Result<Value> {
        self.local_edit(vec![Op::Meta { meta }], None)
    }
    pub fn undo(&mut self) -> Result<Value> {
        self.doc.step(false)?;
        self.unsaved = true;
        Ok(self.doc.view())
    }
    pub fn redo(&mut self) -> Result<Value> {
        self.doc.step(true)?;
        self.unsaved = true;
        Ok(self.doc.view())
    }

    /// Push, then pull (long-polling only when idle), snapshot on a cadence; never touches the store.
    pub fn sync(&mut self, net: &impl PageServer, wait: bool) -> Result<Synced> {
        let before = (self.doc.head, self.doc.local());
        let mut copy = None;
        let mut offline = false;
        match self.push(net) {
            Ok(()) => {}
            Err(PageError::Net(NetError::Unreachable)) => offline = true,
            Err(PageError::Net(e)) if e.is(wire::OPS_COMPACTED) => copy = self.fall_back(net)?,
            // Rotated away: the work stays until a newer grant arrives.
            Err(PageError::Net(e)) if rotated(&e) => {
                self.stale.get_or_insert(self.epoch() + 1);
            }
            Err(PageError::Net(NetError::Refused { status: 429, .. })) => {}
            Err(e) => return Err(e),
        }
        if !offline && self.stale.is_none() {
            let idle = wait && !self.doc.local();
            match self.pull(net, idle) {
                Ok(()) => {}
                Err(PageError::Net(NetError::Unreachable)) => offline = true,
                Err(PageError::Net(e)) if e.is(wire::OPS_COMPACTED) => {
                    copy = copy.or(self.fall_back(net)?)
                }
                Err(PageError::Net(e)) if rotated(&e) => {
                    self.stale.get_or_insert(self.epoch() + 1);
                }
                Err(PageError::Net(NetError::Refused { status: 429, .. })) => {}
                Err(e) => return Err(e),
            }
        }
        if !offline
            && self.stale.is_none()
            && !self.doc.local()
            && self.doc.head - self.doc.snapshot_seq >= SNAPSHOT_EVERY
        {
            let _ = self.snapshot(net);
        }
        if (self.doc.head, self.doc.local()) != before {
            self.unsaved = true;
        }
        Ok(Synced {
            changed: self.doc.head != before.0 || copy.is_some(),
            view: self.doc.view(),
            offline,
            copy,
            stale_key: self.stale,
        })
    }
    /// Sends what is unsent; used on close and backgrounding.
    pub fn flush(&mut self, net: &impl PageServer) -> Result<()> {
        self.push(net)
    }
    fn push(&mut self, net: &impl PageServer) -> Result<()> {
        // Sealed under an older epoch: it landed before rotation or never did.
        if self
            .doc
            .inflight
            .as_ref()
            .is_some_and(|f| f.epoch < self.epoch())
        {
            self.pull(net, false)?;
            if self
                .doc
                .inflight
                .as_ref()
                .is_some_and(|f| f.epoch < self.epoch())
            {
                self.doc.requeue();
            }
        }
        match self.push_once(net) {
            Err(PageError::Net(e)) if e.is(wire::BASE_BEFORE_SNAPSHOT) => {
                self.doc.requeue();
                self.pull(net, false)?;
                self.push_once(net)
            }
            other => other,
        }
    }
    fn push_once(&mut self, net: &impl PageServer) -> Result<()> {
        if self.doc.inflight.is_none() {
            let ops = self.doc.take_batch()?;
            if ops.is_empty() {
                return Ok(());
            }
            let base = self.doc.head;
            let request: Request = Bytes(random()?);
            let key = self.current_key()?;
            let bytes =
                serde_json::to_vec(&ops).map_err(|_| PageError::Invalid("unencodable ops"))?;
            let batch = Batch::sign(
                &self.signer.identity,
                key,
                base,
                &request,
                self.signer.device,
                now(),
                bytes,
            )?;
            let body = Sealed(key.seal_batch(base, &request, &batch)?);
            self.doc.inflight = Some(Inflight {
                ops,
                base,
                epoch: key.epoch(),
                request,
                body,
            });
            self.unsaved = true;
        }
        let Some(inflight) = self.doc.inflight.clone() else {
            return Ok(());
        };
        let appended = net.append(
            &self.id,
            &self.access,
            &Append {
                epoch: inflight.epoch,
                base: inflight.base,
                request: inflight.request,
                body: inflight.body,
            },
        )?;
        for op in appended.missed {
            self.take(op)?;
        }
        if self.doc.inflight.is_some() {
            if appended.seq == self.doc.head + 1 {
                self.doc.fold_own(appended.seq)?;
            } else {
                // A short missed list: the rest, our own batch included, comes from the log.
                self.pull(net, false)?;
                if self.doc.inflight.is_some() {
                    return Err(PageError::Invalid("own batch missing from the log"));
                }
            }
        }
        self.unsaved = true;
        Ok(())
    }
    /// Opens and applies one stored op; only a missing key stops the log.
    fn take(&mut self, op: wire::Op) -> Result<()> {
        if op.seq <= self.doc.head {
            return Ok(());
        }
        let key = self.key(op.epoch)?;
        let ops = key
            .open_batch(op.base, &op.request, &op.body.0)
            .ok()
            .and_then(|batch| serde_json::from_slice::<Vec<Op>>(&batch.ops).ok());
        self.doc
            .receive(op.seq, op.base, Some((&op.request, op.epoch)), ops)
    }
    fn pull(&mut self, net: &impl PageServer, wait: bool) -> Result<()> {
        loop {
            let ops = net.ops(
                &self.id,
                &self.access,
                self.doc.head,
                if wait { wire::MAX_WAIT_SECONDS } else { 0 },
            )?;
            if ops.epoch > self.epoch() {
                self.stale = Some(ops.epoch);
            }
            let count = ops.ops.len();
            for op in ops.ops {
                self.take(op)?;
            }
            if count < wire::MAX_OPS_PAGE || wait {
                return Ok(());
            }
        }
    }
    /// Seals the server-ordered state at head; trims the window once the server keeps it.
    pub fn snapshot(&mut self, net: &impl PageServer) -> Result<bool> {
        let seq = self.doc.head;
        let key = self.current_key()?;
        let body = seal_page(key, seq, &self.doc.base)?;
        let stored = net.put_snapshot(
            &self.id,
            &self.access,
            &PutSnapshot {
                epoch: key.epoch(),
                seq,
                body,
            },
        )?;
        if stored {
            self.doc.snapshot_seq = seq;
            self.doc.window.retain(|(s, _)| *s > seq);
            self.cache = None;
            self.unsaved = true;
        }
        Ok(stored)
    }
    /// Starts again from the newest snapshot; local work that cannot be carried becomes a copy.
    fn fall_back(&mut self, net: &impl PageServer) -> Result<Option<Page>> {
        let copy = self.doc.local().then(|| {
            let mut page = self.doc.current.clone();
            let room = sigil_pages::page::MAX_TITLE - COPY_SUFFIX.chars().count();
            page.meta.title = page.meta.title.chars().take(room).collect::<String>() + COPY_SUFFIX;
            page
        });
        self.reload(net)?;
        Ok(copy)
    }
    fn reload(&mut self, net: &impl PageServer) -> Result<()> {
        let snapshot = net.snapshot(&self.id, &self.access)?;
        let key = self.key(snapshot.epoch)?;
        let opened = key.open(Context::Snapshot { seq: snapshot.seq }, &snapshot.body.0)?;
        let page: Page = serde_json::from_slice(&opened)
            .map_err(|_| PageError::Invalid("unreadable snapshot"))?;
        let (cursor, patch) = (self.doc.cursor, self.doc.patch);
        self.doc = Doc::new(page, snapshot.seq);
        self.doc.cursor = cursor;
        self.doc.patch = patch;
        self.doc.clamp();
        self.cache = None;
        self.unsaved = true;
        self.pull(net, false)
    }

    /// Shares this session's caret, UTF-16 offsets, for 30 s.
    pub fn presence_set(
        &self,
        net: &impl PageServer,
        cursor: usize,
        selection_end: usize,
    ) -> Result<()> {
        let body = &self.doc.current.body;
        let value = Cursor {
            cursor: body.scalar_from_utf16(cursor),
            selection_end: body.scalar_from_utf16(selection_end),
        };
        let key = self.current_key()?;
        let plain =
            serde_json::to_vec(&value).map_err(|_| PageError::Invalid("unencodable presence"))?;
        let sealed = key.seal(Context::Presence { slot: &self.slot }, &plain)?;
        net.put_presence(
            &self.id,
            &self.access,
            &PutPresence {
                epoch: key.epoch(),
                slot: self.slot,
                body: Sealed(sealed),
            },
        )?;
        Ok(())
    }
    /// Other sessions' carets that open under the current key, as UTF-16 offsets.
    pub fn presence(&self, net: &impl PageServer) -> Result<Vec<Value>> {
        let key = self.current_key()?;
        let body = &self.doc.current.body;
        let last = body.target_len().saturating_sub(1);
        Ok(net
            .presence(&self.id, &self.access)?
            .into_iter()
            .filter(|entry| entry.slot != self.slot)
            .filter_map(|entry| {
                let plain = key
                    .open(Context::Presence { slot: &entry.slot }, &entry.body.0)
                    .ok()?;
                let at: Cursor = serde_json::from_slice(&plain).ok()?;
                Some(json!({
                    "slot": entry.slot,
                    "cursor": body.utf16_from_scalar(at.cursor.min(last)),
                    "selection_end": body.utf16_from_scalar(at.selection_end.min(last)),
                    "updated": entry.updated,
                }))
            })
            .collect())
    }
    /// Card summary for `Page::Edited`: title, two-line preview, kind and checklist progress.
    pub fn summary(&self) -> (String, String, Kind, u32, u32) {
        summary(&self.doc.current)
    }
    /// A copy of the signer, for a page made from this one.
    pub fn signer(&self) -> Signer {
        self.signer.copy()
    }
}

/// A rotation replaces the access token before it moves the epoch.
fn rotated(e: &NetError) -> bool {
    e.is(wire::WRONG_EPOCH) || e.is(wire::ACCESS_DENIED)
}

fn seal_page(key: &PageKey, seq: u64, page: &Page) -> Result<Sealed> {
    let bytes = serde_json::to_vec(page).map_err(|_| PageError::Invalid("unencodable page"))?;
    Ok(Sealed(key.seal(Context::Snapshot { seq }, &bytes)?))
}

/// Title, preview of the first two text lines, kind, checked and total check items.
pub fn summary(page: &Page) -> (String, String, Kind, u32, u32) {
    let (mut done, mut total) = (0, 0);
    let mut lines = Vec::new();
    let mut line = String::new();
    for (ch, attrs, _) in page.body.scalars() {
        if ch != '\n' {
            if ch != sigil_pages::EMBED_CHAR && !ch.is_control() {
                line.push(ch);
            }
            continue;
        }
        if attrs.get("list").and_then(Value::as_str) == Some("check") {
            total += 1;
            if attrs.get("checked").and_then(Value::as_bool) == Some(true) {
                done += 1;
            }
        }
        let text = std::mem::take(&mut line);
        if lines.len() < 2 && !text.trim().is_empty() {
            lines.push(text.trim().chars().take(140).collect::<String>());
        }
    }
    (
        page.meta.title.clone(),
        lines.join("\n"),
        page.meta.kind,
        done,
        total,
    )
}

#[cfg(test)]
#[path = "page_sync_tests.rs"]
mod tests;

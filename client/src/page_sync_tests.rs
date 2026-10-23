use super::*;
use crate::{network::HttpsClient, ClientStore};
use sigil_crypto::{storage::StorageKey, Secret32};
use std::cell::Cell;

const HOST: &str = "chat.example";

/// The real client with switchable faults: offline, or an append that lands but loses its reply.
struct Faulty<'a> {
    inner: &'a HttpsClient,
    offline: Cell<bool>,
    lose: Cell<u32>,
}
impl<'a> Faulty<'a> {
    fn new(inner: &'a HttpsClient) -> Self {
        Self {
            inner,
            offline: Cell::new(false),
            lose: Cell::new(0),
        }
    }
    fn up(&self) -> std::result::Result<(), NetError> {
        if self.offline.get() {
            return Err(NetError::Unreachable);
        }
        Ok(())
    }
}
impl PageServer for Faulty<'_> {
    fn create(&self, r: &wire::Create) -> std::result::Result<Status, NetError> {
        self.up()?;
        self.inner.create(r)
    }
    fn append(
        &self,
        id: &PageId,
        a: &[u8; 32],
        r: &Append,
    ) -> std::result::Result<wire::Appended, NetError> {
        self.up()?;
        let answer = self.inner.append(id, a, r);
        if self.lose.get() > 0 && answer.is_ok() {
            self.lose.set(self.lose.get() - 1);
            return Err(NetError::Unreachable);
        }
        answer
    }
    fn ops(
        &self,
        id: &PageId,
        a: &[u8; 32],
        after: u64,
        wait: u64,
    ) -> std::result::Result<wire::Ops, NetError> {
        self.up()?;
        self.inner.ops(id, a, after, wait)
    }
    fn snapshot(&self, id: &PageId, a: &[u8; 32]) -> std::result::Result<wire::Snapshot, NetError> {
        self.up()?;
        self.inner.snapshot(id, a)
    }
    fn put_snapshot(
        &self,
        id: &PageId,
        a: &[u8; 32],
        r: &PutSnapshot,
    ) -> std::result::Result<bool, NetError> {
        self.up()?;
        self.inner.put_snapshot(id, a, r)
    }
    fn put_presence(
        &self,
        id: &PageId,
        a: &[u8; 32],
        r: &PutPresence,
    ) -> std::result::Result<(), NetError> {
        self.up()?;
        self.inner.put_presence(id, a, r)
    }
    fn presence(
        &self,
        id: &PageId,
        a: &[u8; 32],
    ) -> std::result::Result<Vec<wire::PresenceEntry>, NetError> {
        self.up()?;
        self.inner.presence(id, a)
    }
}

struct World {
    dir: tempfile::TempDir,
    _fixture: crate::network::tests::Fixture,
    alice: ClientStore,
    bob: ClientStore,
    a_net: HttpsClient,
    b_net: HttpsClient,
}
fn world() -> World {
    let (dir, fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let (a_net, b_net) = (
        alice.connected_client().unwrap(),
        bob.connected_client().unwrap(),
    );
    let _ = (alice.page_signer().unwrap(), bob.page_signer().unwrap());
    World {
        dir,
        _fixture: fixture,
        alice,
        bob,
        a_net,
        b_net,
    }
}
fn resume(store: &mut ClientStore, grant: &Grant) -> Option<PageSession> {
    let signer = store.page_signer().unwrap();
    store.page_resume(grant, signer).unwrap()
}
/// Retries what the server's per-device rate limit refused.
fn retry<T>(mut call: impl FnMut() -> Result<T>) -> T {
    for _ in 0..100 {
        match call() {
            Err(PageError::Net(NetError::Refused { status: 429, .. })) => {
                std::thread::sleep(std::time::Duration::from_millis(150))
            }
            other => return other.unwrap(),
        }
    }
    panic!("still rate limited");
}
fn open(net: &HttpsClient, grant: &Grant, store: &mut ClientStore) -> PageSession {
    let signer = store.page_signer().unwrap();
    retry(|| PageSession::open(net, grant, signer.copy()))
}
fn text(session: &PageSession) -> String {
    session.page().body.text()
}
fn end(session: &PageSession) -> usize {
    let body = &session.page().body;
    body.utf16_from_scalar(body.target_len() - 1)
}
type SyncWith<'a> = &'a dyn Fn(&mut PageSession) -> Synced;
/// Syncs every session until none holds local work and all stand at one head.
fn settle(sessions: &mut [(&mut PageSession, SyncWith<'_>)]) {
    for _ in 0..200 {
        for (session, sync) in sessions.iter_mut() {
            sync(session);
        }
        let head = sessions[0].0.head();
        if sessions
            .iter()
            .all(|(s, _)| !s.has_local_edits() && s.head() == head)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    panic!("sessions did not settle");
}
fn synced(net: &HttpsClient) -> impl Fn(&mut PageSession) -> Synced + '_ {
    move |s: &mut PageSession| s.sync(net, false).unwrap()
}
fn create(w: &mut World, title: &str) -> (Grant, PageSession) {
    let signer = w.alice.page_signer().unwrap();
    retry(|| {
        PageSession::create(
            &w.a_net,
            HOST,
            Page::new(Kind::Page, title),
            signer.copy(),
            None,
        )
    })
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound.max(1)
    }
}
fn tables(page: &Page) -> Vec<String> {
    sigil_pages::page::embeds(&page.body)
        .filter_map(|e| e["table"]["id"].as_str().map(str::to_owned))
        .collect()
}
/// One random edit of any kind; refusals of impossible edits are fine.
fn random_edit(s: &mut PageSession, rng: &mut Lcg) {
    let len = end(s) as u64;
    let at = rng.next(len + 1) as usize;
    let to = (at + rng.next(4) as usize).min(len as usize);
    let _ = match rng.next(10) {
        0..=3 => s.edit(at, at, ["a", "bc", "é", "🙂", "\n", "xyz "][rng.next(6) as usize]),
        4 => s.edit(at, to, ""),
        5 => {
            let mut attrs = Attrs::new();
            attrs.insert("bold".into(), json!(true));
            s.format(at, to, attrs)
        }
        6 => {
            let mut attrs = Attrs::new();
            attrs.insert("list".into(), json!("check"));
            attrs.insert("checked".into(), json!(rng.next(2) == 0));
            s.paragraph(at, at, attrs)
        }
        7 => match tables(s.page()).first() {
            Some(table) => {
                let (rows, cols) = table_shape(s.page(), table).unwrap();
                let (r, c) = (rng.next(rows as u64) as u32, rng.next(cols as u64) as u32);
                s.table_edit(table, r, c, 0, 0, "t")
            }
            None => s.insert(at, json!({"table": {"rows": [[[{"insert": "a"}], [{"insert": "b"}]]], "widths": [1, 1]}})),
        },
        8 => match tables(s.page()).first() {
            Some(table) => s.table_line(table, rng.next(2) == 0, 0, Action::Insert, None),
            None => s.undo(),
        },
        9 if rng.next(2) == 0 => s.redo(),
        _ => s.meta(MetaOp { title: Some(format!("T{}", rng.next(100))), ..MetaOp::default() }),
    };
}

#[test]
fn concurrent_edits_from_three_devices_converge() {
    let mut w = world();
    for seed in 0..6 {
        let (grant, mut a) = create(&mut w, "Trip");
        let mut b = open(&w.b_net, &grant.reader(), &mut w.bob);
        let mut c = open(&w.a_net, &grant, &mut w.alice);
        let mut rng = Lcg(seed);
        for round in 0..16 {
            for s in [&mut a, &mut b, &mut c] {
                for _ in 0..1 + rng.next(3) {
                    random_edit(s, &mut rng);
                }
            }
            // A rotating subset syncs, so batches cross on the server.
            let skip = rng.next(3);
            if skip != 0 {
                a.sync(&w.a_net, false).unwrap();
            }
            if skip != 1 {
                b.sync(&w.b_net, false).unwrap();
            }
            if skip != 2 || round % 4 == 0 {
                c.sync(&w.a_net, false).unwrap();
            }
        }
        let (sa, sb) = (synced(&w.a_net), synced(&w.b_net));
        settle(&mut [(&mut a, &sa), (&mut b, &sb), (&mut c, &sa)]);
        assert!(a.head() > 5, "seed {seed}");
        assert_eq!(a.page(), b.page(), "seed {seed}");
        assert_eq!(a.page(), c.page(), "seed {seed}");
        assert_eq!(a.page(), a.base(), "seed {seed}");
        let fresh = open(&w.b_net, &grant, &mut w.bob);
        assert_eq!(fresh.page(), a.page(), "seed {seed}");
        if seed == 0 {
            assert!(!tables(a.page()).is_empty());
        }
    }
}

#[test]
fn a_lost_reply_is_resent_under_its_name_and_never_doubles() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Notes");
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    let faulty = Faulty::new(&w.a_net);
    // Bob lands first, so the retried reply carries a missed op as well.
    b.edit(0, 0, "bob ").unwrap();
    b.sync(&w.b_net, false).unwrap();
    a.edit(0, 0, "alice ").unwrap();
    faulty.lose.set(1);
    let lost = a.sync(&faulty, false).unwrap();
    assert!(lost.offline && a.has_local_edits());
    assert_eq!(a.head(), 0);
    let request = a.doc.inflight.as_ref().unwrap().request;
    a.edit(end(&a), end(&a), "!").unwrap();
    let retried = a.sync(&faulty, false).unwrap();
    assert!(!retried.offline);
    b.sync(&w.b_net, false).unwrap();
    let (sa, sb) = (synced(&w.a_net), synced(&w.b_net));
    settle(&mut [(&mut a, &sa), (&mut b, &sb)]);
    assert_eq!(text(&a), "bob alice !\n");
    assert_eq!(a.page(), b.page());
    // Three batches stored: bob's, alice's once, then the trailing edit.
    assert_eq!(a.head(), 3);
    let log = w.a_net.ops(&Bytes(grant.id), &grant.access, 0, 0).unwrap();
    assert_eq!(log.ops.iter().filter(|op| op.request == request).count(), 1);
    // A lost reply when nobody else wrote: the retry answers the first seq.
    a.edit(0, 0, "x").unwrap();
    faulty.lose.set(1);
    assert!(a.sync(&faulty, false).unwrap().offline);
    a.sync(&faulty, false).unwrap();
    assert_eq!((a.head(), a.has_local_edits()), (4, false));
}

#[test]
fn offline_edits_persist_and_reconnect_under_their_request_names() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "List");
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    a.edit(0, 0, "base\n").unwrap();
    a.sync(&w.a_net, false).unwrap();
    // One batch lands with its reply lost, then the device goes offline and keeps typing.
    let faulty = Faulty::new(&w.a_net);
    a.edit(0, 0, "landed ").unwrap();
    faulty.lose.set(1);
    assert!(a.sync(&faulty, false).unwrap().offline);
    faulty.offline.set(true);
    a.edit(end(&a), end(&a), "offline").unwrap();
    let report = a.sync(&faulty, false).unwrap();
    assert!(report.offline && report.view["pending"] == json!(true));
    assert!(a.unsaved());
    w.alice.page_save(&mut a).unwrap();
    assert!(!a.unsaved());
    let expected = a.page().clone();
    drop(a);
    // Others keep writing while the device is away.
    b.sync(&w.b_net, false).unwrap();
    for i in 0..5 {
        b.edit(0, 0, &format!("b{i} ")).unwrap();
        b.sync(&w.b_net, false).unwrap();
    }
    // A restart: the store reopens from disk and the page resumes without the network.
    drop(w.alice);
    let mut alice = ClientStore::open(
        &w.dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let mut a = resume(&mut alice, &grant).unwrap();
    assert_eq!(a.page(), &expected);
    assert!(a.has_local_edits());
    let back = a.sync(&w.a_net, false).unwrap();
    assert!(!back.offline && back.changed);
    let (sa, sb) = (synced(&w.a_net), synced(&w.b_net));
    settle(&mut [(&mut a, &sa), (&mut b, &sb)]);
    assert_eq!(a.page(), b.page());
    assert_eq!(text(&a).matches("landed").count(), 1);
    assert_eq!(text(&a), "b4 b3 b2 b1 b0 landed base\noffline\n");
    // Received batches build a tail over the stored cache, then force a rewrite.
    for i in 0..70 {
        b.edit(0, 0, &format!("{}", i % 10)).unwrap();
        b.sync(&w.b_net, false).unwrap();
        a.sync(&w.a_net, false).unwrap();
        alice.page_save(&mut a).unwrap();
    }
    settle(&mut [(&mut a, &sa), (&mut b, &sb)]);
    alice.page_save(&mut a).unwrap();
    let resumed = resume(&mut alice, &grant).unwrap();
    assert_eq!((resumed.page(), resumed.head()), (b.page(), b.head()));
    alice.page_forget(grant.id).unwrap();
    assert!(resume(&mut alice, &grant).is_none());
}

#[test]
fn edits_past_compaction_become_an_offline_copy() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Plan");
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    a.edit(0, 0, "shared").unwrap();
    a.sync(&w.a_net, false).unwrap();
    a.edit(end(&a), end(&a), " mine").unwrap();
    w.alice.page_save(&mut a).unwrap();
    drop(a);
    b.sync(&w.b_net, false).unwrap();
    b.edit(0, 0, "theirs ").unwrap();
    b.sync(&w.b_net, false).unwrap();
    assert!(b.snapshot(&w.b_net).unwrap());
    // Maintenance compacts the ops behind the snapshot.
    let server = rusqlite::Connection::open(w.dir.path().join("server.db")).unwrap();
    let removed = server
        .execute(
            "DELETE FROM page_ops WHERE page=?1 AND seq<=?2",
            (Bytes(grant.id).hex(), b.head() as i64),
        )
        .unwrap();
    assert_eq!(removed, 2);
    let mut a = resume(&mut w.alice, &grant).unwrap();
    assert_eq!(text(&a), "shared mine\n");
    let synced = a.sync(&w.a_net, false).unwrap();
    let copy = synced.copy.expect("offline copy");
    assert_eq!(copy.meta.title, "Plan (offline copy)");
    assert_eq!(copy.body.text(), "shared mine\n");
    assert!(!a.has_local_edits());
    assert_eq!(a.page(), b.page());
    assert_eq!(text(&a), "theirs shared\n");
    // The copy is a page of its own.
    let (copied, mut session) =
        PageSession::create(&w.a_net, HOST, copy, a.signer(), None).unwrap();
    assert_ne!(copied.id, grant.id);
    session.sync(&w.a_net, false).unwrap();
    let reopened = open(&w.b_net, &copied, &mut w.bob);
    assert_eq!(reopened.page().meta.title, "Plan (offline copy)");
    w.alice.page_save(&mut a).unwrap();
    let resumed = resume(&mut w.alice, &grant).unwrap();
    assert_eq!(resumed.page(), b.page());
}

#[test]
fn a_rotation_keeps_unsent_work_until_the_new_grant_reseals_it() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Plan");
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    a.edit(0, 0, "shared").unwrap();
    a.sync(&w.a_net, false).unwrap();
    b.sync(&w.b_net, false).unwrap();
    // Bob's batch is sealed under epoch 1 but never reaches the server.
    let faulty = Faulty::new(&w.b_net);
    faulty.offline.set(true);
    b.edit(0, 0, "bob ").unwrap();
    assert!(b.sync(&faulty, false).unwrap().offline);
    assert_eq!(b.doc.inflight.as_ref().unwrap().epoch, 1);
    // The managing device rotates at the head.
    let key = PageKey::generate(grant.id, 2).unwrap();
    let (access, manage) = (
        *sigil_crypto::page::token().unwrap(),
        *sigil_crypto::page::token().unwrap(),
    );
    let sealed = seal_page(&key, a.head(), a.base()).unwrap();
    let server = rusqlite::Connection::open(w.dir.path().join("server.db")).unwrap();
    let page = Bytes(grant.id).hex();
    server
        .execute(
            "UPDATE pages SET epoch=2,access_hash=?2,manage_hash=?3 WHERE id=?1",
            (
                &page,
                sigil_crypto::page::token_hash(&access).as_slice(),
                sigil_crypto::page::token_hash(&manage).as_slice(),
            ),
        )
        .unwrap();
    server
        .execute(
            "INSERT INTO page_snapshots(page,version,seq,epoch,created,body) VALUES(?1,(SELECT max(version)+1 FROM page_snapshots WHERE page=?1),?2,2,1,?3)",
            (&page, a.head() as i64, &sealed.0),
        )
        .unwrap();
    // The revoked token marks the key stale and keeps the work.
    faulty.offline.set(false);
    let stale = b.sync(&faulty, false).unwrap();
    assert_eq!(stale.stale_key, Some(2));
    assert!(b.has_local_edits());
    assert_eq!(text(&b), "bob shared\n");
    let rotated = Grant {
        epoch: 2,
        key: *key.key_bytes(),
        access,
        manage: Some(manage),
        ..grant.clone()
    };
    b.add_grant(&rotated).unwrap();
    a.add_grant(&rotated).unwrap();
    let fresh = b.sync(&faulty, false).unwrap();
    assert_eq!(fresh.stale_key, None);
    let (sa, sb) = (synced(&w.a_net), synced(&w.b_net));
    settle(&mut [(&mut a, &sa), (&mut b, &sb)]);
    assert_eq!(text(&a), "bob shared\n");
    assert_eq!(a.page(), b.page());
    let log = w.a_net.ops(&Bytes(grant.id), &access, 0, 0).unwrap();
    assert_eq!(log.ops.last().map(|op| op.epoch), Some(2));
    assert_eq!(log.ops.len(), 2);
}

#[test]
fn undo_reaches_across_remote_edits_and_drops_what_they_removed() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Draft");
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    a.edit(0, 0, "hello ").unwrap();
    a.sync(&w.a_net, false).unwrap();
    b.sync(&w.b_net, false).unwrap();
    a.edit(6, 6, "world").unwrap();
    b.edit(0, 0, "X").unwrap();
    b.sync(&w.b_net, false).unwrap();
    a.sync(&w.a_net, false).unwrap();
    assert_eq!(text(&a), "Xhello world\n");
    let view = a.undo().unwrap();
    assert_eq!(view["can_redo"], json!(true));
    assert_eq!(text(&a), "Xhello \n");
    a.undo().unwrap();
    assert_eq!(text(&a), "X\n");
    a.redo().unwrap();
    a.redo().unwrap();
    assert_eq!(text(&a), "Xhello world\n");
    let (sa, sb) = (synced(&w.a_net), synced(&w.b_net));
    settle(&mut [(&mut a, &sa), (&mut b, &sb)]);
    assert_eq!(text(&b), "Xhello world\n");
    // Formatting undone after a remote insert inside the range keeps the insert.
    let mut bold = Attrs::new();
    bold.insert("bold".into(), json!(true));
    a.format(1, 6, bold).unwrap();
    a.sync(&w.a_net, false).unwrap();
    b.sync(&w.b_net, false).unwrap();
    b.edit(3, 3, "_").unwrap();
    b.sync(&w.b_net, false).unwrap();
    a.sync(&w.a_net, false).unwrap();
    a.undo().unwrap();
    assert_eq!(text(&a), "Xhe_llo world\n");
    // Only the remote insert keeps the bold it took from its neighbour.
    let bold: String = a
        .page()
        .body
        .scalars()
        .iter()
        .filter(|(_, attrs, _)| attrs.contains_key("bold"))
        .map(|s| s.0)
        .collect();
    assert_eq!(bold, "_");
    // A remote delete of everything an entry touched empties it, so undo has nothing left.
    let mut fresh = open(&w.a_net, &grant, &mut w.alice);
    settle(&mut [(&mut a, &sa), (&mut b, &sb), (&mut fresh, &sa)]);
    fresh.edit(0, 0, "Z").unwrap();
    fresh.sync(&w.a_net, false).unwrap();
    b.sync(&w.b_net, false).unwrap();
    b.edit(0, 1, "").unwrap();
    b.sync(&w.b_net, false).unwrap();
    fresh.sync(&w.a_net, false).unwrap();
    assert_eq!(fresh.undo(), Err(PageError::Invalid("nothing to undo")));
    settle(&mut [(&mut a, &sa), (&mut b, &sb), (&mut fresh, &sa)]);
    assert_eq!(a.page(), b.page());
    assert_eq!(fresh.page(), b.page());
}

#[test]
fn an_unopenable_batch_is_stepped_over_by_every_device() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Log");
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    a.edit(0, 0, "one").unwrap();
    a.sync(&w.a_net, false).unwrap();
    let garbage = Append {
        epoch: 1,
        base: 1,
        request: Bytes([7; 16]),
        body: Sealed(vec![5; 80]),
    };
    w.b_net
        .append(&Bytes(grant.id), &grant.access, &garbage)
        .unwrap();
    b.edit(0, 0, "two ").unwrap();
    let (sa, sb) = (synced(&w.a_net), synced(&w.b_net));
    settle(&mut [(&mut a, &sa), (&mut b, &sb)]);
    assert_eq!(a.head(), 3);
    assert_eq!(text(&a), "onetwo \n");
    assert_eq!(a.page(), b.page());
    assert_eq!(
        a.doc
            .window
            .iter()
            .find(|(s, _)| *s == 2)
            .map(|e| e.1.len()),
        Some(0)
    );
    // A batch sealed under an epoch nobody here holds stops the log instead of being skipped.
    let other = Append {
        epoch: 1,
        base: 3,
        request: Bytes([8; 16]),
        body: Sealed(vec![5; 80]),
    };
    w.b_net
        .append(&Bytes(grant.id), &grant.access, &other)
        .unwrap();
    a.add_grant(&Grant {
        epoch: 2,
        ..grant.clone()
    })
    .unwrap();
    assert_eq!(a.epoch(), 2);
    a.keys.retain(|k| k.epoch() == 2);
    assert_eq!(a.sync(&w.a_net, false).unwrap_err(), PageError::NeedsKey(1));
}

#[test]
fn a_long_poll_runs_outside_the_store_and_wakes_on_another_append() {
    let w = world();
    let World {
        dir: _dir,
        _fixture,
        alice,
        mut bob,
        a_net,
        b_net,
    } = w;
    let alice = std::sync::Mutex::new(alice);
    let signer = alice.lock().unwrap().page_signer().unwrap();
    let (grant, a) = PageSession::create(
        &a_net,
        HOST,
        Page::new(Kind::List, "Groceries"),
        signer,
        None,
    )
    .unwrap();
    let mut b = PageSession::open(&b_net, &grant, bob.page_signer().unwrap()).unwrap();
    let a = std::sync::Mutex::new(a);
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            let started = std::time::Instant::now();
            let synced = b.sync(&b_net, true).unwrap();
            (synced.changed, started.elapsed())
        });
        std::thread::sleep(std::time::Duration::from_millis(400));
        // The store stays usable while the other session waits on the network.
        let mut session = a.lock().unwrap();
        session.edit(0, 0, "Milk").unwrap();
        alice
            .try_lock()
            .expect("store free during a long-poll")
            .page_save(&mut session)
            .unwrap();
        session.sync(&a_net, false).unwrap();
        let (changed, waited) = waiting.join().unwrap();
        assert!(changed);
        assert!(waited < std::time::Duration::from_secs(10));
    });
    assert_eq!(text(&b), "Milk\n");
}

#[test]
fn presence_is_sealed_per_session_and_reads_in_utf16() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Here");
    a.edit(0, 0, "🙂 hi").unwrap();
    a.sync(&w.a_net, false).unwrap();
    let mut b = open(&w.b_net, &grant, &mut w.bob);
    b.sync(&w.b_net, false).unwrap();
    a.presence_set(&w.a_net, 3, 5).unwrap();
    assert!(a.presence(&w.a_net).unwrap().is_empty());
    let seen = b.presence(&w.b_net).unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0]["cursor"].clone(), seen[0]["selection_end"].clone()),
        (json!(3), json!(5))
    );
    let (title, preview, kind, done, total) = a.summary();
    assert_eq!(
        (title.as_str(), preview.as_str(), kind, done, total),
        ("Here", "🙂 hi", Kind::Page, 0, 0)
    );
}

#[test]
fn snapshots_on_a_cadence_trim_the_window_and_open_from_there() {
    let mut w = world();
    let (grant, mut a) = create(&mut w, "Long");
    for i in 0..SNAPSHOT_EVERY + 2 {
        a.edit(0, 0, &format!("{}", i % 10)).unwrap();
        loop {
            a.sync(&w.a_net, false).unwrap();
            if !a.has_local_edits() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    assert!(a.doc.snapshot_seq >= SNAPSHOT_EVERY);
    assert!(a.doc.window.iter().all(|(s, _)| *s > a.doc.snapshot_seq));
    let opened = open(&w.b_net, &grant, &mut w.bob);
    assert_eq!(opened.doc.snapshot_seq, a.doc.snapshot_seq);
    assert_eq!(opened.page(), a.page());
}

#[test]
fn schema_eighty_six_gains_the_page_tables() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("client.db");
    let key = || StorageKey::new(Secret32::from_bytes([3; 32])).unwrap();
    let store = ClientStore::open(&path, key()).unwrap();
    crate::test_schema::rewind(&store.db, 86);
    let tables = |store: &ClientStore| -> i64 {
        store
            .db
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name IN ('page_cache','page_work')",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(tables(&store), 0);
    drop(store);
    let store = ClientStore::open(&path, key()).unwrap();
    assert_eq!(tables(&store), 2);
    let version: i64 = store
        .db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, i64::from(crate::DATABASE_VERSION));
    drop(store);
    assert!(ClientStore::open(&path, key()).is_ok());
}

/// The server's op-log rules in memory, for many seeds.
#[derive(Default)]
struct Memory {
    log: std::cell::RefCell<Vec<wire::Op>>,
    snapshot: std::cell::RefCell<Option<(u64, Sealed)>>,
}
fn refused(code: &str) -> NetError {
    NetError::Refused {
        status: 409,
        code: code.into(),
    }
}
impl PageServer for Memory {
    fn create(&self, r: &wire::Create) -> std::result::Result<Status, NetError> {
        *self.snapshot.borrow_mut() = Some((0, r.snapshot.clone()));
        Ok(Status {
            epoch: 1,
            head: 0,
            snapshot_seq: 0,
            bytes: 0,
            expires_at: None,
        })
    }
    fn append(
        &self,
        _: &PageId,
        _: &[u8; 32],
        r: &Append,
    ) -> std::result::Result<wire::Appended, NetError> {
        let mut log = self.log.borrow_mut();
        let between = |log: &[wire::Op], base: u64, seq: u64| {
            log.iter()
                .filter(|o| o.seq > base && o.seq < seq)
                .cloned()
                .collect()
        };
        if let Some(op) = log.iter().find(|o| o.request == r.request) {
            if op.body != r.body {
                return Err(refused(wire::REQUEST_REUSED));
            }
            return Ok(wire::Appended {
                seq: op.seq,
                missed: between(&log, op.base, op.seq),
            });
        }
        let head = log.len() as u64;
        if r.base > head {
            return Err(refused(wire::BASE_BEYOND_HEAD));
        }
        if self
            .snapshot
            .borrow()
            .as_ref()
            .is_some_and(|(s, _)| r.base < *s)
        {
            return Err(refused(wire::BASE_BEFORE_SNAPSHOT));
        }
        let seq = head + 1;
        log.push(wire::Op {
            seq,
            base: r.base,
            epoch: r.epoch,
            created: 1,
            request: r.request,
            body: r.body.clone(),
        });
        Ok(wire::Appended {
            seq,
            missed: between(&log, r.base, seq),
        })
    }
    fn ops(
        &self,
        _: &PageId,
        _: &[u8; 32],
        after: u64,
        _: u64,
    ) -> std::result::Result<wire::Ops, NetError> {
        let log = self.log.borrow();
        let ops = log
            .iter()
            .filter(|o| o.seq > after)
            .take(wire::MAX_OPS_PAGE)
            .cloned()
            .collect();
        Ok(wire::Ops {
            epoch: 1,
            head: log.len() as u64,
            ops,
        })
    }
    fn snapshot(&self, _: &PageId, _: &[u8; 32]) -> std::result::Result<wire::Snapshot, NetError> {
        let (seq, body) = self.snapshot.borrow().clone().ok_or(NetError::Invalid)?;
        Ok(wire::Snapshot {
            version: 1,
            seq,
            epoch: 1,
            created: 1,
            body,
            label: None,
        })
    }
    fn put_snapshot(
        &self,
        _: &PageId,
        _: &[u8; 32],
        r: &PutSnapshot,
    ) -> std::result::Result<bool, NetError> {
        if self
            .snapshot
            .borrow()
            .as_ref()
            .is_some_and(|(s, _)| *s >= r.seq)
        {
            return Ok(false);
        }
        if self
            .log
            .borrow()
            .iter()
            .any(|o| o.seq > r.seq && o.base < r.seq)
        {
            return Err(refused(wire::SNAPSHOT_ORPHANS_OP));
        }
        *self.snapshot.borrow_mut() = Some((r.seq, r.body.clone()));
        Ok(true)
    }
    fn put_presence(
        &self,
        _: &PageId,
        _: &[u8; 32],
        _: &PutPresence,
    ) -> std::result::Result<(), NetError> {
        Ok(())
    }
    fn presence(
        &self,
        _: &PageId,
        _: &[u8; 32],
    ) -> std::result::Result<Vec<wire::PresenceEntry>, NetError> {
        Ok(Vec::new())
    }
}
/// Loses replies to appends that landed, and sometimes refuses to answer at all.
struct Lossy<'a> {
    inner: &'a Memory,
    rng: std::cell::RefCell<Lcg>,
}
impl Lossy<'_> {
    fn dice(&self, bound: u64) -> bool {
        self.rng.borrow_mut().next(bound) == 0
    }
}
impl PageServer for Lossy<'_> {
    fn create(&self, r: &wire::Create) -> std::result::Result<Status, NetError> {
        self.inner.create(r)
    }
    fn append(
        &self,
        id: &PageId,
        a: &[u8; 32],
        r: &Append,
    ) -> std::result::Result<wire::Appended, NetError> {
        if self.dice(6) {
            return Err(NetError::Unreachable);
        }
        let answer = self.inner.append(id, a, r)?;
        if self.dice(4) {
            return Err(NetError::Unreachable);
        }
        Ok(answer)
    }
    fn ops(
        &self,
        id: &PageId,
        a: &[u8; 32],
        after: u64,
        wait: u64,
    ) -> std::result::Result<wire::Ops, NetError> {
        if self.dice(8) {
            return Err(NetError::Unreachable);
        }
        self.inner.ops(id, a, after, wait)
    }
    fn snapshot(&self, id: &PageId, a: &[u8; 32]) -> std::result::Result<wire::Snapshot, NetError> {
        self.inner.snapshot(id, a)
    }
    fn put_snapshot(
        &self,
        id: &PageId,
        a: &[u8; 32],
        r: &PutSnapshot,
    ) -> std::result::Result<bool, NetError> {
        self.inner.put_snapshot(id, a, r)
    }
    fn put_presence(
        &self,
        id: &PageId,
        a: &[u8; 32],
        r: &PutPresence,
    ) -> std::result::Result<(), NetError> {
        self.inner.put_presence(id, a, r)
    }
    fn presence(
        &self,
        id: &PageId,
        a: &[u8; 32],
    ) -> std::result::Result<Vec<wire::PresenceEntry>, NetError> {
        self.inner.presence(id, a)
    }
}
fn memory_signer() -> Signer {
    Signer::new(IdentityKey::generate().unwrap(), [1; 32])
}

#[test]
fn sessions_converge_over_lost_replies_dropped_polls_and_snapshots() {
    let seeds: u64 = std::env::var("PAGE_SYNC_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    for seed in 0..seeds {
        let memory = Memory::default();
        let lossy = Lossy {
            inner: &memory,
            rng: std::cell::RefCell::new(Lcg(seed ^ 0x5eed)),
        };
        let (grant, a) = PageSession::create(
            &memory,
            HOST,
            Page::new(Kind::Page, "S"),
            memory_signer(),
            None,
        )
        .unwrap();
        let mut sessions = vec![a];
        for _ in 0..1 + seed % 3 {
            sessions.push(PageSession::open(&memory, &grant, memory_signer()).unwrap());
        }
        let mut rng = Lcg(seed);
        for _ in 0..30 {
            for s in sessions.iter_mut() {
                for _ in 0..rng.next(3) {
                    random_edit(s, &mut rng);
                }
                if rng.next(3) != 0 {
                    s.sync(&lossy, false).unwrap();
                }
                if rng.next(12) == 0 {
                    let _ = s.snapshot(&memory);
                }
            }
        }
        for _ in 0..4 {
            for s in sessions.iter_mut() {
                s.sync(&memory, false).unwrap();
            }
        }
        let first = sessions[0].page().clone();
        for s in &sessions {
            assert!(!s.has_local_edits(), "seed {seed}");
            assert_eq!(s.page(), &first, "seed {seed}");
            assert_eq!(s.head(), memory.log.borrow().len() as u64, "seed {seed}");
        }
        let fresh = PageSession::open(&memory, &grant, memory_signer()).unwrap();
        assert_eq!(fresh.page(), &first, "seed {seed}");
        let requests: std::collections::BTreeSet<_> =
            memory.log.borrow().iter().map(|o| o.request).collect();
        assert_eq!(requests.len(), memory.log.borrow().len(), "seed {seed}");
    }
}

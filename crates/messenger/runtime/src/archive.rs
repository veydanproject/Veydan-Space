// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The runtime's side of the archive of the conversation
//! (`messenger-archive`; `internal/messenger-archive-plan.md` §4.4, stage
//! Э3; `internal/messenger-spec.md`, «Архив переписки»).
//!
//! One object, [`ArchiveBridge`], made with the runtime and wired into
//! every session, carries three things:
//!
//! - **recording**: the ingress tells it of every event it kept
//!   ([`messenger_ingress::KeptSink`]), and it hands the archive
//!   ([`ArchiveSink`]) an [`ArchivedEvent`] of the kinds the archive
//!   holds: an opened personal message or own note, an event of a group,
//!   a piece of the snapshot of my relations;
//! - **replay** ([`Replayer`]): what the archive restores goes into the
//!   ingress of the running session as history (`EventSource::Archive`):
//!   a wire event through the very steps a relay's event takes, an opened
//!   wrap straight to the DM handler;
//! - **the start of a fresh device**: the relay catch-up of personal
//!   messages and groups asks since the archive's high-water mark of the
//!   scope ([`ArchiveMarks`]) less the usual margin, instead of since the
//!   beginning, and on a fresh database waits for the archive to have
//!   restored its newest segments first ([`ArchiveBridge::wait_restored`];
//!   plan §5, step B), at most [`RESTORE_WAIT`].
//!
//! The handler of the sync entities (`messenger-app`, stage Э2) installs
//! the archive with `MessengerRuntime::set_archive`, releases the wait
//! with `archive_restored`, and asks once for the past of a device that
//! has one with `archive_backfill`. Without an archive installed nothing
//! here changes what the runtime did before.

use async_trait::async_trait;
use messenger_archive::{ArchiveScope, ArchivedEvent, Body, Replayed, Replayer, REFUSED_GROUP_UNKNOWN, REFUSED_SERVER_UNKNOWN};
use messenger_core::{Inbound, MessengerError, MetaInbound, PubKey, RawEvent, Result, Timestamp};
use messenger_ingress::{classify, Ingress, Kept, KeptSink, Processed};
use messenger_store::{events_raw, settings, Store};
use nostr::key::Keys;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Notify;

/// How long a fresh database holds its relay catch-up for the archive to
/// restore the newest months (the budget of step B of the restore, plan
/// §5 and §9 D9). Released earlier by `archive_restored`. Waited once per
/// session: the catch-up of every private server after the home set's
/// goes at once.
pub const RESTORE_WAIT: Duration = Duration::from_secs(20);
/// Rows of `msg_events_raw` the backfill reads and classifies in one go
/// before it yields.
const BACKFILL_PAGE: i64 = 500;
/// Setting: how far the past of the identity was fed to the archive —
/// `<hex key>:<rowid of msg_events_raw>`, saved page by page, so a walk
/// cut short goes on from there and a walk done starts after the last
/// row (recording is once by id: a row walked twice is harmless).
pub const KEY_BACKFILL_AFTER: &str = "archive.backfill_after";
/// The kinds the backfill walks: gift wraps, events of groups, the
/// snapshot of my relations.
const BACKFILL_KINDS: [u16; 3] = [messenger_archive::KIND_GIFT_WRAP, messenger_ingress::classify::KIND_GROUP_MESSAGE, messenger_ingress::classify::KIND_RELATIONS];

/// The seams of the archive the runtime reads and feeds — `ArchiveMarks`
/// (how far the archive reaches) and `ArchiveHooks` (a sink with marks) —
/// live in `messenger-archive` beside `ArchiveSink` since stages Э1–Э3
/// met; the names stay here for the host.
pub use messenger_archive::{ArchiveHooks, ArchiveMarks};

/// Who to tell that something a parked segment of the restore may have
/// waited for came ([`ArchiveBridge::wake`]): the handler of the sync
/// entities, which puts its parked segments back and looks again.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The seam itself; see the module.
pub struct ArchiveBridge {
    store: Store,
    hooks: RwLock<Option<Arc<dyn ArchiveHooks>>>,
    waker: RwLock<Option<Waker>>,
    /// The steps of the running session's ingress; `None` between sessions.
    ingress: RwLock<Option<Arc<Ingress>>>,
    /// Whose session runs: the only events replayed are of this key's
    /// archive (a personal message to or from me, a snapshot signed by me).
    me: RwLock<Option<PubKey>>,
    /// The archive has restored what a fresh device waits for.
    restored: AtomicBool,
    /// The session waited for the restore once already.
    waited: AtomicBool,
    released: Notify,
}

impl ArchiveBridge {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            hooks: RwLock::new(None),
            waker: RwLock::new(None),
            ingress: RwLock::new(None),
            me: RwLock::new(None),
            restored: AtomicBool::new(false),
            waited: AtomicBool::new(false),
            released: Notify::new(),
        }
    }

    /// Installs the archive (`Some`), or takes it away (`None`: the sync
    /// storage is gone). Takes effect at once, for the running session
    /// and the next ones. A newly installed archive has not restored yet:
    /// a fresh database waits for `restored` again.
    pub fn install(&self, hooks: Option<Arc<dyn ArchiveHooks>>) {
        self.restored.store(false, Ordering::SeqCst);
        self.waited.store(false, Ordering::SeqCst);
        *self.hooks.write().unwrap_or_else(|p| p.into_inner()) = hooks;
        // Nobody waits for an archive that is not there.
        if self.hooks().is_none() {
            self.released.notify_waiters();
        }
    }

    pub fn hooks(&self) -> Option<Arc<dyn ArchiveHooks>> {
        self.hooks.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_waker(&self, waker: Option<Waker>) {
        *self.waker.write().unwrap_or_else(|p| p.into_inner()) = waker;
    }

    /// A group's welcome landed, a private server was put in or taken out:
    /// a segment the restore parked for want of its group or server may go
    /// now, and the handler is told so.
    pub fn wake(&self) {
        if let Some(waker) = self.waker.read().unwrap_or_else(|p| p.into_inner()).clone() {
            waker();
        }
    }

    pub fn is_installed(&self) -> bool {
        self.hooks().is_some()
    }

    /// The archive has replayed every segment the storage holds, or as
    /// much as step B of the restore takes: the catch-up may go.
    pub fn restored(&self) {
        self.restored.store(true, Ordering::SeqCst);
        self.released.notify_waiters();
    }

    /// Holds a catch-up that would otherwise start from the beginning
    /// until the archive has restored, or `RESTORE_WAIT` passed. Returns
    /// at once without an archive, once it has restored, and once the
    /// session waited already (the catch-ups of the private servers come
    /// after the home set's: one wait, not one each).
    pub async fn wait_restored(&self) {
        // Made before the check, so that a release between the check and
        // the wait is not missed.
        let released = self.released.notified();
        if !self.is_installed() || self.restored.load(Ordering::SeqCst) {
            return;
        }
        if self.waited.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = tokio::time::timeout(RESTORE_WAIT, released).await;
    }

    /// Where the catch-up of `scope` starts when its cursor is empty:
    /// the archive's high-water mark, no later than `now` (the mark is
    /// capped by the archive already; a clock of the past is capped here),
    /// less `margin`; or from the beginning without one.
    pub async fn since_without_cursor(&self, scope: &ArchiveScope, margin: i64, now: Timestamp) -> i64 {
        let Some(hooks) = self.hooks() else { return 0 };
        match hooks.high_water(scope).await {
            Some(mark) => (mark.secs().min(now.secs()) - margin).max(0),
            None => 0,
        }
    }

    /// The ingress of the session that started, and whose it is.
    pub(crate) fn attach(&self, ingress: Arc<Ingress>, me: PubKey) {
        *self.ingress.write().unwrap_or_else(|p| p.into_inner()) = Some(ingress);
        *self.me.write().unwrap_or_else(|p| p.into_inner()) = Some(me);
        self.waited.store(false, Ordering::SeqCst);
    }

    /// The session with this ingress stopped. Another session's ingress,
    /// attached since, stays.
    pub(crate) fn detach(&self, ingress: &Arc<Ingress>) {
        let mut current = self.ingress.write().unwrap_or_else(|p| p.into_inner());
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, ingress)) {
            *current = None;
            *self.me.write().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }

    fn ingress(&self) -> Result<(Arc<Ingress>, PubKey)> {
        let ingress = self.ingress.read().unwrap_or_else(|p| p.into_inner()).clone().ok_or(MessengerError::NotLoggedIn)?;
        let me = self.me.read().unwrap_or_else(|p| p.into_inner()).clone().ok_or(MessengerError::NotLoggedIn)?;
        Ok((ingress, me))
    }

    /// Feeds the archive what this device kept before the archive
    /// existed: every gift wrap that opens with `keys`, every event of a
    /// group, the newest piece of each page of the snapshot of my
    /// relations; nothing that came from the archive itself. Pages of the
    /// store, classified off the async threads, a yield between; where
    /// the walk stands is saved page by page (`KEY_BACKFILL_AFTER`), after
    /// the page's events and snapshot pieces are handed over, so a walk
    /// cut short goes on without losing them and a walk done finds
    /// nothing more. How many events were handed over.
    pub async fn backfill(&self, keys: Keys) -> Result<u64> {
        self.backfill_pages(keys, BACKFILL_PAGE).await
    }

    /// [`backfill`], `page_size` rows of the store at a time.
    ///
    /// [`backfill`]: Self::backfill
    pub(crate) async fn backfill_pages(&self, keys: Keys, page_size: i64) -> Result<u64> {
        let hooks = self.hooks().ok_or_else(|| MessengerError::Other("no archive installed".into()))?;
        let me = keys.public_key().to_hex();
        let mut after = settings::get(&self.store, KEY_BACKFILL_AFTER)
            .await?
            .and_then(|v| {
                let (who, rowid) = v.split_once(':')?;
                (who == me).then(|| rowid.parse::<i64>().ok()).flatten()
            })
            .unwrap_or(0);
        let mut handed = 0u64;
        // The newest piece of each page of the snapshot handed over so far
        // (what a relay itself would answer: the newest per `d`); a page
        // of the store hands its pieces over before its position is saved.
        let mut recorded: BTreeMap<usize, i64> = BTreeMap::new();
        loop {
            let rows = events_raw::page_after(&self.store, &BACKFILL_KINDS, after, page_size).await?;
            let Some(last) = rows.last() else { break };
            after = last.0;
            let opener = keys.clone();
            let classified = tokio::task::spawn_blocking(move || {
                rows.into_iter()
                    .filter(|(_, raw, _)| raw.source != messenger_core::EventSource::Archive)
                    .map(|(_, raw, received_at)| {
                        let inbound = classify(&raw, Some(&opener));
                        (raw, inbound, received_at)
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| MessengerError::Other(format!("backfill: {e}")))?;
            let mut snapshot: BTreeMap<usize, (i64, RawEvent, i64)> = BTreeMap::new();
            for (raw, inbound, received_at) in classified {
                let received_at = Timestamp(received_at);
                match inbound {
                    Inbound::Dm(inbound) if raw.kind == messenger_archive::KIND_GIFT_WRAP => {
                        hooks.record(ArchivedEvent::dm(inbound, received_at)).await?;
                        handed += 1;
                    }
                    Inbound::Group(g) if raw.kind == messenger_ingress::classify::KIND_GROUP_MESSAGE => {
                        hooks.record(ArchivedEvent::wire(raw, ArchiveScope::Group { id: g.group_id }, received_at)).await?;
                        handed += 1;
                    }
                    Inbound::Meta(MetaInbound::Relations { page, created_at, .. }) => {
                        let at = created_at.secs();
                        let newer = recorded.get(&page).is_none_or(|r| at > *r) && snapshot.get(&page).is_none_or(|(s, _, _)| at > *s);
                        if newer {
                            snapshot.insert(page, (at, raw, received_at.secs()));
                        }
                    }
                    _ => {}
                }
            }
            for (page, (at, raw, received_at)) in snapshot {
                hooks.record(ArchivedEvent::wire(raw, ArchiveScope::Meta, Timestamp(received_at))).await?;
                recorded.insert(page, at);
                handed += 1;
            }
            settings::set(&self.store, KEY_BACKFILL_AFTER, &format!("{me}:{after}")).await?;
            tokio::task::yield_now().await;
        }
        Ok(handed)
    }
}

/// The group of a kind-9 event as the relay has it (its `h` tag).
fn group_of(raw: &RawEvent) -> Option<&str> {
    raw.json["tags"].as_array()?.iter().find_map(|t| {
        let t = t.as_array()?;
        (t.first()?.as_str()? == "h").then(|| t.get(1)?.as_str()).flatten()
    })
}

#[async_trait]
impl KeptSink for ArchiveBridge {
    async fn kept(&self, raw: &RawEvent, what: Kept, received_at: Timestamp) {
        let Some(hooks) = self.hooks() else { return };
        let event = match what {
            Kept::Dm(inbound) => ArchivedEvent::dm(inbound, received_at),
            Kept::Group { group_id } => ArchivedEvent::wire(raw.clone(), ArchiveScope::Group { id: group_id }, received_at),
            Kept::Relations => ArchivedEvent::wire(raw.clone(), ArchiveScope::Meta, received_at),
        };
        if let Err(e) = hooks.record(event).await {
            eprintln!("messenger archive: an event was not recorded: {e}");
        }
    }
}

#[async_trait]
impl Replayer for ArchiveBridge {
    /// Into the running session's ingress, as history. A wire event of a
    /// group this device does not hold yet (its welcome has not come) and
    /// an opened message of a private server this device does not have
    /// are refused (`group_unknown`, `server_unknown`) rather than taken:
    /// taken, the handler would drop them and the ingress mark them seen,
    /// and nothing would bring them again. The archive keeps the segment
    /// for a later pass. A personal message neither to nor from me and a
    /// snapshot of relations not signed by me are another identity's (a
    /// storage two identities share) and refused for good (`not_mine`).
    /// Without a session: `NotLoggedIn`.
    async fn replay(&self, event: ArchivedEvent) -> Result<Replayed> {
        let (ingress, me) = self.ingress()?;
        let not_mine = || MessengerError::Invalid("not_mine".into());
        let processed = match event.body {
            Body::Wire { mut raw } => {
                raw.source = messenger_core::EventSource::Archive;
                if raw.kind == messenger_ingress::classify::KIND_GROUP_MESSAGE {
                    let Some(group) = group_of(&raw) else { return Ok(Replayed::Ignored) };
                    let mine = messenger_store::groups::get(&self.store, group).await?.is_some_and(|row| {
                        matches!(row.membership.as_str(), messenger_groups::service::MEMBERSHIP_JOINED | messenger_groups::service::MEMBERSHIP_JOINING)
                    });
                    if !mine {
                        return Err(MessengerError::Invalid(REFUSED_GROUP_UNKNOWN.into()));
                    }
                } else if raw.kind == messenger_ingress::classify::KIND_RELATIONS && raw.pubkey != me {
                    return Err(not_mine());
                }
                ingress.process(raw).await
            }
            Body::Dm { mut inbound } => {
                inbound.envelope.source = messenger_core::EventSource::Archive;
                if inbound.sender != me && !inbound.recipients.contains(&me) {
                    return Err(not_mine());
                }
                if let Some(server) = &inbound.srv {
                    if messenger_store::servers::get(&self.store, server).await?.is_none() {
                        return Err(MessengerError::Invalid(REFUSED_SERVER_UNKNOWN.into()));
                    }
                }
                ingress.process_opened(inbound).await
            }
        };
        Ok(match processed {
            Processed::Applied => Replayed::Applied,
            Processed::Duplicate => Replayed::Duplicate,
            Processed::Ignored => Replayed::Ignored,
            Processed::Failed => return Err(MessengerError::Other("the handler refused the event".into())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::device;
    use messenger_archive::{ArchiveSink, KIND_GIFT_WRAP};
    use messenger_core::outbound::WireEvent;
    use messenger_core::traits::UiEvent;
    use messenger_core::{EventId, EventSource, Outbound, PubKey, RelayUrl, SecretStore, Timestamp};
    use messenger_testkit::open_dm;
    use nostr::nips::nip17::PrivateDirectMessageBuilder;
    use nostr::prelude::{Event, EventBuilder, FinalizeEvent as _, Keys, Kind, Tag, ToBech32};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use tokio::sync::broadcast;

    /// An archive that remembers what it was given and answers one mark;
    /// it cannot take the record number `fail_at` (the store busy).
    #[derive(Default)]
    struct Recorder {
        recorded: Mutex<Vec<ArchivedEvent>>,
        mark: Mutex<Option<Timestamp>>,
        fail_at: Mutex<Option<usize>>,
    }

    impl Recorder {
        fn ids(&self) -> Vec<String> {
            self.recorded.lock().unwrap().iter().map(|e| e.id.as_hex().to_string()).collect()
        }
    }

    #[async_trait]
    impl ArchiveSink for Recorder {
        async fn record(&self, event: ArchivedEvent) -> Result<()> {
            let mut recorded = self.recorded.lock().unwrap();
            if self.fail_at.lock().unwrap().is_some_and(|n| recorded.len() + 1 == n) {
                return Err(MessengerError::Storage("database is locked".into()));
            }
            recorded.push(event);
            Ok(())
        }
    }

    #[async_trait]
    impl ArchiveMarks for Recorder {
        async fn high_water(&self, _: &ArchiveScope) -> Option<Timestamp> {
            *self.mark.lock().unwrap()
        }
    }

    fn raw_of(event: &Event) -> RawEvent {
        RawEvent {
            id: EventId::parse(&event.id.to_hex()).unwrap(),
            kind: event.kind.as_u16(),
            pubkey: PubKey::parse(&event.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(event.created_at.as_secs() as i64),
            json: serde_json::to_value(event).unwrap(),
            source: EventSource::Relay { url: RelayUrl::parse("wss://relay.example").unwrap() },
        }
    }

    /// A wrap from `who` to `me`, and the archive's record of it opened.
    fn wrapped(who: &Keys, me: &Keys, text: &str) -> (Event, ArchivedEvent) {
        let wrap = PrivateDirectMessageBuilder::new(me.public_key(), text).finalize(who).unwrap();
        let wire = WireEvent { id: EventId::parse(&wrap.id.to_hex()).unwrap(), json: serde_json::to_value(&wrap).unwrap() };
        let inbound = open_dm(me, &wire, false).expect("addressed to me");
        (wrap, ArchivedEvent::dm(inbound, Timestamp(1_800_000_000)))
    }

    async fn until(mut cond: impl AsyncFnMut() -> bool) {
        for _ in 0..100 {
            if cond().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("did not happen in time");
    }

    fn drain(ui: &mut broadcast::Receiver<UiEvent>) -> Vec<UiEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = ui.try_recv() {
            out.push(ev);
        }
        out
    }

    /// A personal message replayed from the archive lands in its chat as
    /// history: no notification, no unread on a fresh database; a row
    /// stands in for its wrap, on the home side of the local set; the
    /// same event again is a duplicate; and the archive is not told of it.
    #[tokio::test]
    async fn a_replayed_dm_lands_as_history_once() {
        let (_dir, rt, _secrets, me) = device(None).await;
        let alice = Keys::generate();
        let recorder = Arc::new(Recorder::default());
        rt.set_archive(Some(recorder.clone()));
        let mut ui = rt.ui_events();
        let (wrap, archived) = wrapped(&alice, &me, "from the archive");
        let wire_id = archived.id.clone();

        assert_eq!(rt.replayer().replay(archived.clone()).await.unwrap(), Replayed::Applied);
        let chat = messenger_store::chats::dm_chat_id(&alice.public_key().to_hex(), None);
        let msgs = rt.dm().messages(&chat, None, 10).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("from the archive"));
        assert_eq!(rt.dm().chat(&chat).await.unwrap().unwrap().unread, 0, "history is not unread on a fresh database");
        let events = drain(&mut ui);
        let landed = events.iter().find(|e| e.name == messenger_dm::UI_EVENT_DM_MESSAGE).expect("told");
        assert_eq!(landed.payload["historical"], true);
        assert!(!events.iter().any(|e| e.name == "notify"), "no notification of history");

        assert!(events_raw::contains(rt.store(), &wire_id).await.unwrap(), "a row stands for the wrap");
        let local = crate::session::dm_local_set(rt.store(), None, 0).await;
        assert!(local.iter().any(|i| i.id == wire_id), "the home set's reconciliation lists it");
        assert_eq!(rt.replayer().replay(archived).await.unwrap(), Replayed::Duplicate);
        assert_eq!(rt.dm().messages(&chat, None, 10).await.unwrap().len(), 1);
        assert!(recorder.ids().is_empty(), "the archive is not told of what came from it");

        // The wrap itself, by a relay later: seen, nothing twice.
        rt.relays.pool().await.inject(raw_of(&wrap)).await.unwrap();
        until(async || rt.session.lock().await.as_ref().unwrap().stats().1 >= 1).await;
        assert_eq!(rt.dm().messages(&chat, None, 10).await.unwrap().len(), 1);
        rt.shutdown().await;
    }

    /// The archive hears of the opened wrap of a personal message, not
    /// of a profile; a message of a private server this device does not
    /// have is refused rather than dropped as seen.
    #[tokio::test]
    async fn the_archive_hears_the_archived_kinds_only() {
        let (_dir, rt, _secrets, me) = device(None).await;
        let alice = Keys::generate();
        let recorder = Arc::new(Recorder::default());
        rt.set_archive(Some(recorder.clone()));
        let pool = rt.relays.pool().await;

        let (wrap, _) = wrapped(&alice, &me, "live");
        pool.inject(raw_of(&wrap)).await.unwrap();
        until(async || !recorder.ids().is_empty()).await;
        let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"alice"}"#).finalize(&alice).unwrap();
        pool.inject(raw_of(&profile)).await.unwrap();
        until(async || events_raw::contains(rt.store(), &raw_of(&profile).id).await.unwrap()).await;
        let recorded = recorder.recorded.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "the profile is not archived");
        assert_eq!(recorded[0].id.as_hex(), wrap.id.to_hex());
        assert_eq!(recorded[0].kind, KIND_GIFT_WRAP);
        assert_eq!(recorded[0].scope, ArchiveScope::Dm);
        match &recorded[0].body {
            Body::Dm { inbound } => {
                assert_eq!(inbound.content, "live");
                assert_eq!(inbound.envelope.source, EventSource::Archive);
            }
            Body::Wire { .. } => panic!("an opened wrap, not the wire"),
        }

        let (_, mut of_server) = wrapped(&alice, &me, "private");
        if let Body::Dm { inbound } = &mut of_server.body {
            inbound.srv = Some("nosuchserver".into());
        }
        let refused = rt.replayer().replay(of_server.clone()).await;
        assert!(matches!(refused, Err(MessengerError::Invalid(ref e)) if e == "server_unknown"), "{refused:?}");
        assert!(!events_raw::contains(rt.store(), &of_server.id).await.unwrap(), "not marked seen");

        // Another identity's archive (a storage two identities share): a
        // message neither to nor from me, a snapshot not signed by me.
        let other = Keys::generate();
        let (_, theirs) = wrapped(&alice, &other, "to someone else");
        let refused = rt.replayer().replay(theirs.clone()).await;
        assert!(matches!(refused, Err(MessengerError::Invalid(ref e)) if e == "not_mine"), "{refused:?}");
        assert!(!events_raw::contains(rt.store(), &theirs.id).await.unwrap());
        let snapshot = EventBuilder::new(Kind::Custom(messenger_ingress::classify::KIND_RELATIONS), "").finalize(&other).unwrap();
        let theirs = ArchivedEvent::wire(raw_of(&snapshot), ArchiveScope::Meta, Timestamp(1));
        let refused = rt.replayer().replay(theirs).await;
        assert!(matches!(refused, Err(MessengerError::Invalid(ref e)) if e == "not_mine"), "{refused:?}");
        rt.shutdown().await;
    }

    /// A group of a private server replayed from the archive is taken as
    /// history: the device that kept its events heard them on the
    /// group's own server (`servers::group_heard_on`).
    #[tokio::test]
    async fn a_group_of_a_private_server_is_replayed_from_the_archive() {
        let (_dir_a, a, _secrets_a, keys) = device(None).await;
        let nsec = keys.secret_key().to_bech32().unwrap();
        let server = crate::private::PrivateServerInput { relay_url: "wss://mine.example".into(), name: Some("Mine".into()), ..Default::default() };
        let sid = a.server_put_private(&server).await.unwrap();
        let g = a.group_create(messenger_groups::GroupKind::Private, "Work", "", true, Some(&sid)).await.unwrap();
        a.group_send_text(&g.id, "on our server", None).await.unwrap();
        let mut events: Vec<RawEvent> = Vec::new();
        for row in messenger_store::outbox::due(a.store(), i64::MAX / 4, 0).await.unwrap() {
            if let Ok(Outbound::PublishScoped { event, .. }) = row.outbound() {
                let ev: Event = serde_json::from_value(event.json).unwrap();
                if ev.kind.as_u16() == messenger_ingress::classify::KIND_GROUP_MESSAGE {
                    events.push(raw_of(&ev));
                }
            }
        }
        assert!(events.len() >= 2, "{}", events.len());
        let row = messenger_store::groups::get(a.store(), &g.id).await.unwrap().unwrap();
        assert_eq!(messenger_store::servers::of_group(a.store(), &g.id).await.unwrap().as_deref(), Some(sid.as_str()));
        a.shutdown().await;

        let (_dir_b, b, _secrets_b, _) = device(Some(&nsec)).await;
        b.set_archive(Some(Arc::new(Recorder::default())));
        assert_eq!(b.server_put_private(&server).await.unwrap(), sid);
        messenger_store::groups::upsert(b.store(), &row).await.unwrap();
        for raw in &events {
            let archived = ArchivedEvent::wire(raw.clone(), ArchiveScope::Group { id: g.id.clone() }, Timestamp(1));
            assert_eq!(b.replayer().replay(archived).await.unwrap(), Replayed::Applied, "heard on its own server");
        }
        assert!(events_raw::contains(b.store(), &events[0].id).await.unwrap(), "seen");
        b.shutdown().await;
    }

    /// The events of a group as the relay has them: refused while the
    /// group is not here; parked without its key; opened, as history,
    /// when the key comes into the lock box from the archive.
    #[tokio::test]
    async fn a_group_event_waits_for_its_key_from_the_archive() {
        let (_dir_a, a, secrets_a, keys) = device(None).await;
        let nsec = keys.secret_key().to_bech32().unwrap();
        let g = a.group_create(messenger_groups::GroupKind::Private, "Family", "", true, None).await.unwrap();
        a.group_send_text(&g.id, "hello from the past", None).await.unwrap();
        let mut events: Vec<RawEvent> = Vec::new();
        for row in messenger_store::outbox::due(a.store(), i64::MAX / 4, 0).await.unwrap() {
            if let Ok(Outbound::PublishScoped { event, .. }) = row.outbound() {
                let ev: Event = serde_json::from_value(event.json).unwrap();
                if ev.kind.as_u16() == messenger_ingress::classify::KIND_GROUP_MESSAGE {
                    events.push(raw_of(&ev));
                }
            }
        }
        assert!(events.len() >= 2, "the creation and the message: {}", events.len());
        let row = messenger_store::groups::get(a.store(), &g.id).await.unwrap().unwrap();
        let kid = group_key_id(&events[0]);
        let name = format!("group.{}.key.{kid}", g.id);
        let key = secrets_a.get(&name).await.unwrap().expect("the key of my group");
        a.shutdown().await;

        let (_dir_b, b, secrets_b, _) = device(Some(&nsec)).await;
        b.set_archive(Some(Arc::new(Recorder::default())));
        let first = ArchivedEvent::wire(events[0].clone(), ArchiveScope::Group { id: g.id.clone() }, Timestamp(1));
        let refused = b.replayer().replay(first.clone()).await;
        assert!(matches!(refused, Err(MessengerError::Invalid(ref e)) if e == "group_unknown"), "{refused:?}");
        assert!(!events_raw::contains(b.store(), &first.id).await.unwrap(), "not marked seen");

        // As the welcome to myself would have left it.
        messenger_store::groups::upsert(b.store(), &row).await.unwrap();
        for raw in &events {
            let archived = ArchivedEvent::wire(raw.clone(), ArchiveScope::Group { id: g.id.clone() }, Timestamp(1));
            assert_eq!(b.replayer().replay(archived).await.unwrap(), Replayed::Applied);
        }
        assert!(!messenger_store::groups::pending_in(b.store(), &g.id, &kid).await.unwrap().is_empty(), "parked for the key");
        let chat = format!("group:{}", g.id);
        assert!(b.dm().messages(&chat, None, 50).await.unwrap().is_empty());
        let again = ArchivedEvent::wire(events[0].clone(), ArchiveScope::Group { id: g.id.clone() }, Timestamp(1));
        assert_eq!(b.replayer().replay(again).await.unwrap(), Replayed::Duplicate);

        secrets_b.put(&name, &key).await.unwrap();
        b.group_secret_arrived(&name).await.unwrap();
        assert!(messenger_store::groups::pending_in(b.store(), &g.id, &kid).await.unwrap().is_empty(), "opened");
        let texts: Vec<String> = b.dm().messages(&chat, None, 50).await.unwrap().into_iter().filter_map(|m| m.text).collect();
        assert!(texts.iter().any(|t| t == "hello from the past"), "{texts:?}");
        b.shutdown().await;
    }

    fn group_key_id(raw: &RawEvent) -> String {
        raw.json["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|t| (t[0] == "k").then(|| t[1].as_str().unwrap().to_string()))
            .expect("a k tag")
    }

    /// A device with a past feeds it to the archive once: the wraps
    /// opened, the profile left out, a row the archive itself left
    /// skipped; a second run hands over nothing.
    #[tokio::test]
    async fn the_backfill_opens_the_past_once() {
        let (_dir, rt, _secrets, me) = device(None).await;
        let alice = Keys::generate();
        let pool = rt.relays.pool().await;
        let (one, _) = wrapped(&alice, &me, "one");
        let (two, _) = wrapped(&alice, &me, "two");
        let profile = EventBuilder::new(Kind::Metadata, "{}").finalize(&alice).unwrap();
        for ev in [&one, &two, &profile] {
            pool.inject(raw_of(ev)).await.unwrap();
        }
        until(async || events_raw::count(rt.store()).await.unwrap() == 3).await;

        assert!(matches!(rt.archive_backfill(), Err(MessengerError::Other(_))), "no archive yet");
        let recorder = Arc::new(Recorder::default());
        rt.set_archive(Some(recorder.clone()));
        let (_, replayed) = wrapped(&alice, &me, "from the archive");
        assert_eq!(rt.replayer().replay(replayed).await.unwrap(), Replayed::Applied);

        assert_eq!(rt.archive_backfill().unwrap().await.unwrap().unwrap(), 2);
        let mut ids = recorder.ids();
        ids.sort();
        let mut want = vec![one.id.to_hex(), two.id.to_hex()];
        want.sort();
        assert_eq!(ids, want, "the wraps, not the profile nor the row of the archive");
        assert!(recorder.recorded.lock().unwrap().iter().all(|e| matches!(&e.body, Body::Dm { inbound } if inbound.envelope.source == EventSource::Archive)));
        assert_eq!(rt.archive_backfill().unwrap().await.unwrap().unwrap(), 0, "the walk is done: nothing after its last row");
        assert_eq!(recorder.ids().len(), 2);
        // A wrap kept since the walk: the next walk starts after the last
        // row and hands over that one alone.
        let (three, _) = wrapped(&alice, &me, "three");
        pool.inject(raw_of(&three)).await.unwrap();
        until(async || recorder.ids().len() == 3).await;
        let after = settings::get(rt.store(), KEY_BACKFILL_AFTER).await.unwrap().unwrap();
        assert!(after.starts_with(&format!("{}:", me.public_key().to_hex())), "{after}");
        // A walk cut short: pushed back to the start, it goes over the
        // same rows again; the archive keeps an id once, so no harm.
        settings::set(rt.store(), KEY_BACKFILL_AFTER, &format!("{}:0", me.public_key().to_hex())).await.unwrap();
        assert_eq!(rt.archive_backfill().unwrap().await.unwrap().unwrap(), 3, "walked again from the start");
        assert_eq!(recorder.ids().len(), 6, "handed over again");
        rt.shutdown().await;
    }

    /// The pieces of the snapshot of my relations go to the archive with
    /// the page of the store they came on (the newest per page of the
    /// snapshot), so a walk cut short — the archive could not take a
    /// record — loses none of them; the walk resumes after the last page
    /// it saved and hands no piece over twice.
    #[tokio::test]
    async fn the_backfill_hands_the_snapshot_over_page_by_page() {
        let (_dir, rt, _secrets, me) = device(None).await;
        let alice = Keys::generate();
        let pool = rt.relays.pool().await;
        let piece = |at: u64| {
            EventBuilder::new(Kind::from(messenger_core::relations::KIND_RELATIONS), "{}")
                .tag(Tag::identifier(messenger_core::relations::relations_d(1)))
                .custom_created_at(nostr::types::Timestamp::from_secs(at))
                .finalize(&me)
                .unwrap()
        };
        let older = piece(1_700_000_000);
        let (wrap, _) = wrapped(&alice, &me, "one");
        let newer = piece(1_700_000_100);
        for ev in [&older, &wrap, &newer] {
            pool.inject(raw_of(ev)).await.unwrap();
        }
        until(async || events_raw::count(rt.store()).await.unwrap() == 3).await;

        // A row a page: the first page's piece is taken, the wrap of the
        // second is not, and the walk breaks off there.
        let recorder = Arc::new(Recorder { fail_at: Mutex::new(Some(2)), ..Default::default() });
        rt.set_archive(Some(recorder.clone()));
        let err = rt.archive.backfill_pages(me.clone(), 1).await.unwrap_err();
        assert!(matches!(err, MessengerError::Storage(_)), "{err}");
        assert_eq!(recorder.ids(), [older.id.to_hex()], "the piece went with its page");
        let after = settings::get(rt.store(), KEY_BACKFILL_AFTER).await.unwrap().unwrap();
        assert!(after.ends_with(":1"), "the first page is saved, the second is not: {after}");

        // Resumed: the wrap and the newer piece, the older not again.
        *recorder.fail_at.lock().unwrap() = None;
        assert_eq!(rt.archive.backfill_pages(me.clone(), 1).await.unwrap(), 2);
        assert_eq!(recorder.ids(), [older.id.to_hex(), wrap.id.to_hex(), newer.id.to_hex()]);
        rt.shutdown().await;
    }

    /// The runtime wakes the handler of the archive when a parked segment's
    /// wait may be over: a private server came (or went), my membership of
    /// a group changed (its welcome landed). Nobody is woken once the
    /// handler is gone.
    #[tokio::test]
    async fn a_server_or_a_group_that_comes_wakes_the_archive() {
        let (_dir, rt, _secrets, _me) = device(None).await;
        let woke = Arc::new(AtomicUsize::new(0));
        let n = woke.clone();
        rt.set_archive_waker(Some(Arc::new(move || {
            n.fetch_add(1, Ordering::SeqCst);
        })));
        rt.server_put_private(&crate::private::PrivateServerInput { relay_url: "wss://private.example".into(), name: Some("S1".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(woke.load(Ordering::SeqCst), 1, "a server came");
        let joined = || messenger_groups::service::Outcome { resubscribe: true, ..Default::default() };
        rt.group_driver.apply(joined()).await.unwrap();
        assert_eq!(woke.load(Ordering::SeqCst), 2, "a membership changed");
        rt.set_archive_waker(None);
        rt.group_driver.apply(joined()).await.unwrap();
        assert_eq!(woke.load(Ordering::SeqCst), 2, "nobody to wake");
        rt.shutdown().await;
    }

    /// Where a catch-up without a cursor starts: the archive's mark less
    /// the margin, never after now, the beginning without a mark or an
    /// archive.
    #[tokio::test]
    async fn the_catch_up_starts_where_the_archive_ends() {
        let store = Store::open_in_memory().await.unwrap();
        let bridge = ArchiveBridge::new(store);
        let now = Timestamp(10_000);
        assert_eq!(bridge.since_without_cursor(&ArchiveScope::Dm, 100, now).await, 0, "no archive");
        let recorder = Arc::new(Recorder::default());
        bridge.install(Some(recorder.clone()));
        assert_eq!(bridge.since_without_cursor(&ArchiveScope::Dm, 100, now).await, 0, "nothing archived");
        *recorder.mark.lock().unwrap() = Some(Timestamp(5_000));
        assert_eq!(bridge.since_without_cursor(&ArchiveScope::Dm, 100, now).await, 4_900);
        assert_eq!(bridge.since_without_cursor(&ArchiveScope::Group { id: "g".into() }, 6_000, now).await, 0, "never before the beginning");
        // A mark of 2099: the catch-up starts from now, not from 2099.
        *recorder.mark.lock().unwrap() = Some(Timestamp(4_070_908_800));
        assert_eq!(bridge.since_without_cursor(&ArchiveScope::Dm, 100, now).await, 9_900);
    }

    /// A fresh database waits for the archive to restore, at most
    /// `RESTORE_WAIT`; not at all without an archive, nor once restored.
    #[tokio::test]
    async fn a_fresh_database_waits_for_the_restore() {
        let store = Store::open_in_memory().await.unwrap();
        let bridge = Arc::new(ArchiveBridge::new(store));
        // The clock stands still once the store is open: its pool keeps
        // real timers.
        tokio::time::pause();
        tokio::time::timeout(Duration::from_millis(10), bridge.wait_restored()).await.expect("no archive: no wait");
        bridge.install(Some(Arc::new(Recorder::default())));
        assert!(tokio::time::timeout(Duration::from_secs(1), bridge.wait_restored()).await.is_err(), "waits");
        let waiter = tokio::spawn({
            let bridge = bridge.clone();
            async move { bridge.wait_restored().await }
        });
        tokio::task::yield_now().await;
        bridge.restored();
        tokio::time::timeout(Duration::from_millis(10), waiter).await.expect("released").unwrap();
        tokio::time::timeout(Duration::from_millis(10), bridge.wait_restored()).await.expect("restored: no wait");
        // Installed anew: not restored yet, but the wait ends by itself,
        // and the session waits once: the next catch-up goes at once.
        bridge.install(Some(Arc::new(Recorder::default())));
        let started = tokio::time::Instant::now();
        bridge.wait_restored().await;
        assert!(started.elapsed() >= RESTORE_WAIT);
        tokio::time::timeout(Duration::from_millis(10), bridge.wait_restored()).await.expect("waited once already: no wait");
    }
}

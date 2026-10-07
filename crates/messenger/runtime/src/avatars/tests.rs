// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::*;
use messenger_core::Timestamp;
use messenger_media::download::BlobFetcher;
use messenger_media::{MediaServerInput, MemoryBackend};
use messenger_testkit::MemorySecretStore;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

const BASE: &str = "https://mem.example/a";
const DAY: i64 = 24 * 3600;
const T0: i64 = 1_800_000_000;

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

impl TestClock {
    fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

/// Downloads of chat media are not what these tests are about.
struct NoFetch;

#[async_trait]
impl BlobFetcher for NoFetch {
    async fn fetch(&self, _url: &str, _max: u64) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// My servers are the memory backend; everything else is a map.
#[derive(Default)]
struct MemNet {
    backend: Option<MemoryBackend>,
    foreign: Mutex<HashMap<String, Vec<u8>>>,
    /// `own <url>` / `foreign <url>`, in order.
    calls: Mutex<Vec<String>>,
    /// Addresses asked past the caches.
    fresh: Mutex<Vec<String>>,
    delay: Option<Duration>,
    active: AtomicUsize,
    most: AtomicUsize,
}

impl MemNet {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn serve(&self, url: &str, body: Vec<u8>) {
        self.foreign.lock().unwrap().insert(url.into(), body);
    }
}

#[async_trait]
impl AvatarNet for MemNet {
    async fn own(&self, url: &str, _max: usize, fresh: bool) -> Result<Option<Vec<u8>>> {
        self.calls.lock().unwrap().push(format!("own {url}"));
        if fresh {
            self.fresh.lock().unwrap().push(url.into());
        }
        let path = url.split('?').next().unwrap();
        let backend = self.backend.as_ref().ok_or_else(|| MessengerError::Transport("no server".into()))?;
        match path.strip_prefix(&format!("{}/", backend.base)) {
            Some(sha) => Ok(backend.get(sha)),
            None => Err(MessengerError::Transport("unknown host".into())),
        }
    }

    async fn foreign(&self, url: &str, _max: usize) -> Result<Vec<u8>> {
        self.calls.lock().unwrap().push(format!("foreign {url}"));
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.foreign.lock().unwrap().get(url).cloned().ok_or_else(|| MessengerError::Transport("down".into()))
    }
}

struct Rig {
    av: Avatars,
    backend: MemoryBackend,
    net: Arc<MemNet>,
    clock: Arc<TestClock>,
    keys: Keys,
    store: Store,
    profiles: ProfileService,
    outbox: Outbox,
    ui: broadcast::Sender<UiEvent>,
    dir: tempfile::TempDir,
}

async fn rig_on(backend: MemoryBackend, keys: Keys, net: MemNet) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().await.unwrap();
    let media = MediaService::new(store.clone(), Arc::new(MemorySecretStore::unlocked()), dir.path())
        .unwrap()
        .with_backend(Arc::new(backend.clone()), Arc::new(NoFetch));
    rig_with_media(store, media, backend, keys, net, dir)
}

fn rig_with_media(store: Store, media: MediaService, backend: MemoryBackend, keys: Keys, net: MemNet, dir: tempfile::TempDir) -> Rig {
    let clock = Arc::new(TestClock(AtomicI64::new(T0)));
    let profiles = ProfileService::new(store.clone());
    // As after a relay told the session my kind 0; `session_started` undoes it.
    profiles.note_own_heard();
    let outbox = Outbox::new(store.clone(), clock.clone());
    let (ui, _) = broadcast::channel(64);
    let net = Arc::new(MemNet { backend: Some(backend.clone()), ..net });
    let av = Avatars::new(
        store.clone(),
        media,
        profiles.clone(),
        outbox.clone(),
        ui.clone(),
        clock.clone(),
        net.clone(),
        dir.path().join("avatars"),
    );
    Rig { av, backend, net, clock, keys, store, profiles, outbox, ui, dir }
}

async fn rig() -> Rig {
    rig_on(MemoryBackend::new(BASE), Keys::generate(), MemNet::default()).await
}

impl Rig {
    fn me(&self) -> PubKey {
        PubKey::parse(&self.keys.public_key().to_hex()).unwrap()
    }

    async fn picture(&self) -> Option<String> {
        self.profiles.get(&self.me()).await.unwrap().and_then(|p| p.picture)
    }

    async fn row(&self) -> Option<own_avatar::OwnAvatar> {
        own_avatar::get(&self.store).await.unwrap()
    }

    fn own_file(&self, sha: &str) -> PathBuf {
        self.dir.path().join("avatars").join("own").join(format!("{sha}.jpg"))
    }

    /// Pick `bytes` and make all of it my avatar.
    async fn set(&self, bytes: Vec<u8>) -> own_avatar::OwnAvatar {
        let p = self.av.prepare(bytes).await.unwrap();
        self.av.set(&self.keys, &p.token, CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }).await.unwrap();
        self.row().await.unwrap()
    }

    async fn ready(&self, rx: &mut broadcast::Receiver<UiEvent>, url: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let ev = rx.recv().await.unwrap();
                if ev.name == UI_EVENT_AVATAR_READY && ev.payload["url"] == url {
                    return;
                }
            }
        })
        .await
        .expect("avatar.ready");
    }
}

/// An uncompressed 24-bit BMP, `w` by `h`, every pixel `rgb`.
fn bmp(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(row * h).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..h {
        let mut line: Vec<u8> = (0..w).flat_map(|_| [rgb[2], rgb[1], rgb[0]]).collect();
        line.resize(row as usize, 0);
        out.extend_from_slice(&line);
    }
    out
}

fn data_bytes(data_url: &str) -> Vec<u8> {
    use base64::Engine as _;
    let b64 = data_url.strip_prefix("data:image/jpeg;base64,").expect("a JPEG data url");
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{what}");
}

fn code<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(MessengerError::Invalid(c)) => c,
        other => panic!("{other:?}"),
    }
}

// ─── Small pieces ───────────────────────────────────────────────────────────

#[test]
fn hashes_in_addresses() {
    let sha = "ab".repeat(32);
    assert_eq!(hash_of(&format!("https://x.example/{sha}")), Some(sha.clone()));
    assert_eq!(hash_of(&format!("https://x.example/a/{sha}.jpg?x=1#y")), Some(sha.clone()));
    assert_eq!(hash_of(&format!("https://x.example/{}", sha.to_uppercase())), Some(sha.clone()), "lowercased");
    assert_eq!(hash_of(&format!("https://x.example/{sha}.tar.gz")), None);
    assert_eq!(hash_of(&format!("https://x.example/{sha}/x")), None);
    assert_eq!(hash_of(&format!("https://x.example/{}", &sha[1..])), None);
    assert_eq!(hash_of("https://x.example/avatar.png"), None);
    assert_eq!(hash_of(""), None);

    let bases = vec![("a".to_string(), "https://a.example/b/".to_string()), ("c".to_string(), "http://10.0.0.2:3000".to_string())];
    assert_eq!(server_of(&format!("https://a.example/b/{sha}"), &bases), Some("a"));
    assert_eq!(server_of(&format!("http://10.0.0.2:3000/{sha}"), &bases), Some("c"));
    assert_eq!(server_of(&format!("https://a.example/b/{sha}.jpg"), &bases), None, "exactly a blob of the server");
    assert_eq!(server_of(&format!("https://a.example/bb/{sha}"), &bases), None);
    assert!(under_my_server("https://a.example/b/x.png", &bases));
    assert!(!under_my_server("https://a.example/bx.png", &bases));

    assert!(unique("https://a.example/x").starts_with("https://a.example/x?k="));
    assert!(unique("https://a.example/x?y=1").starts_with("https://a.example/x?y=1&k="));
    assert_ne!(unique("https://a.example/x"), unique("https://a.example/x"));
}

#[test]
fn failed_addresses_wait_longer_but_never_give_up() {
    let row = |attempts, failed_at| avatar_cache::AvatarCacheRow { url: "u".into(), attempts, failed_at, ..Default::default() };
    assert!(may_try(None, T0));
    assert!(may_try(Some(&row(0, 0)), T0));
    assert!(!may_try(Some(&row(1, T0 - 10)), T0), "an hour first");
    assert!(may_try(Some(&row(1, T0 - RETRY_AFTER_SECS)), T0));
    assert!(!may_try(Some(&row(2, T0 - RETRY_AFTER_SECS)), T0), "two hours after the second");
    assert!(may_try(Some(&row(2, T0 - 2 * RETRY_AFTER_SECS)), T0));
    assert_eq!(retry_after(4), 8 * RETRY_AFTER_SECS);
    assert_eq!(retry_after(6), MAX_RETRY_SECS, "a day at most");
    assert_eq!(retry_after(i64::MAX), MAX_RETRY_SECS);
    assert!(!may_try(Some(&row(1000, T0 - MAX_RETRY_SECS + 1)), T0));
    assert!(may_try(Some(&row(1000, T0 - MAX_RETRY_SECS)), T0), "never given up for good");
}

// ─── The plan ───────────────────────────────────────────────────────────────

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

fn mine(c: char, at: i64) -> own_avatar::OwnAvatar {
    own_avatar::OwnAvatar {
        sha256: sha(c),
        url: format!("https://m.example/{}", sha(c)),
        server_id: Some("m".into()),
        copies_json: "[]".into(),
        set_at: at,
        checked_at: at,
        touched_at: at,
    }
}

fn facts(own: Option<own_avatar::OwnAvatar>, picture: Option<Option<String>>) -> Facts {
    Facts {
        own,
        picture,
        bases: vec![("m".into(), "https://m.example".into()), ("n".into(), "https://n.example".into())],
        known: vec![("m".into(), "https://m.example".into()), ("n".into(), "https://n.example".into())],
        own_files: vec![sha('a')],
        fresh: true,
        now: T0,
    }
}

#[test]
fn plan_waits_for_my_kind_0() {
    assert_eq!(plan(&facts(Some(mine('a', T0)), None)), vec![]);
    assert_eq!(plan(&facts(None, None)), vec![]);
    assert_eq!(plan(&facts(None, Some(None))), vec![]);
}

#[test]
fn plan_checks_and_touches_when_due() {
    let url = format!("https://m.example/{}", sha('a'));
    assert_eq!(plan(&facts(Some(mine('a', T0)), Some(Some(url.clone())))), vec![], "all fresh");
    let f = facts(Some(mine('a', T0 - CHECK_EVERY_SECS)), Some(Some(url.clone())));
    assert_eq!(plan(&f), vec![Step::Check { server_id: "m".into(), sha: sha('a') }]);
    let f = facts(Some(mine('a', T0 - TOUCH_EVERY_SECS)), Some(Some(url.clone())));
    assert_eq!(
        plan(&f),
        vec![Step::Check { server_id: "m".into(), sha: sha('a') }, Step::Touch { url: url.clone(), server_id: "m".into(), sha: sha('a') }]
    );
    // The file is fetched first: everything after needs it.
    let mut f = facts(Some(mine('a', T0 - CHECK_EVERY_SECS)), Some(Some(url.clone())));
    f.own_files.clear();
    assert_eq!(
        plan(&f),
        vec![Step::Download { url: url.clone(), sha: sha('a') }, Step::Check { server_id: "m".into(), sha: sha('a') }]
    );
}

#[test]
fn plan_moves_a_picture_off_a_server_no_longer_used() {
    let url = format!("https://m.example/{}", sha('a'));
    let mut f = facts(Some(mine('a', T0)), Some(Some(url.clone())));
    f.bases.remove(0);
    f.known.remove(0);
    assert_eq!(plan(&f), vec![Step::Rehost { sha: sha('a') }]);
    f.fresh = false;
    assert_eq!(plan(&f), vec![], "not before a relay told my kind 0");
    f.fresh = true;
    f.bases.clear();
    f.known.clear();
    assert_eq!(plan(&f), vec![], "nowhere to go: it stays");
    // …and is kept alive where it is.
    f.now += TOUCH_EVERY_SECS;
    assert_eq!(plan(&f), vec![Step::KeepAlive { url, sha: sha('a') }]);
}

/// A server still enabled whose secret cannot be read just now (a keyring
/// locked at autostart) is no reason to move.
#[test]
fn plan_waits_for_a_server_in_use_that_cannot_be_written_now() {
    let url = format!("https://m.example/{}", sha('a'));
    let mut f = facts(Some(mine('a', T0 - TOUCH_EVERY_SECS)), Some(Some(url)));
    f.bases.remove(0);
    assert_eq!(plan(&f), vec![]);
    f.bases.clear();
    assert_eq!(plan(&f), vec![]);
}

#[test]
fn plan_follows_what_another_device_did() {
    // Removed there.
    assert_eq!(plan(&facts(Some(mine('a', T0)), Some(None))), vec![Step::Forget { sha: sha('a'), keep_file: false }]);
    // Another picture set there.
    let other = format!("https://n.example/{}", sha('b'));
    assert_eq!(
        plan(&facts(Some(mine('a', T0)), Some(Some(other.clone())))),
        vec![
            Step::Forget { sha: sha('a'), keep_file: false },
            Step::Adopt { url: other.clone(), sha: sha('b'), server_id: "n".into() },
            Step::Download { url: other.clone(), sha: sha('b') },
        ]
    );
    // The same picture moved to another server: the file stays.
    let moved = format!("https://n.example/{}", sha('a'));
    assert_eq!(
        plan(&facts(Some(mine('a', T0)), Some(Some(moved.clone())))),
        vec![Step::Forget { sha: sha('a'), keep_file: true }, Step::Adopt { url: moved, sha: sha('a'), server_id: "n".into() }]
    );
    // A new device of mine.
    assert_eq!(
        plan(&facts(None, Some(Some(other.clone())))),
        vec![Step::Adopt { url: other.clone(), sha: sha('b'), server_id: "n".into() }, Step::Download { url: other, sha: sha('b') }]
    );
}

#[test]
fn plan_leaves_pictures_of_other_clients_alone() {
    for url in [format!("https://elsewhere.example/{}", sha('b')), "https://m.example/me.png".to_string()] {
        assert_eq!(plan(&facts(None, Some(Some(url.clone())))), vec![], "{url}");
        assert_eq!(plan(&facts(Some(mine('a', T0)), Some(Some(url.clone())))), vec![Step::Forget { sha: sha('a'), keep_file: false }]);
    }
}

// ─── Mine ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn set_puts_a_clean_jpeg_on_my_server_and_names_it_in_kind_0() {
    let r = rig().await;
    let p = r.av.prepare(bmp(300, 200, [200, 30, 30])).await.unwrap();
    assert_eq!((p.width, p.height), (300, 200));
    assert!(p.preview.starts_with("data:image/jpeg;base64,"));
    assert_eq!(p.token.len(), 32);

    let rect = CropRect { x: 0.1, y: 0.0, w: 0.6, h: 1.0 };
    r.av.set(&r.keys, &p.token, rect).await.unwrap();
    let row = r.row().await.unwrap();
    let url = format!("{BASE}/{}", row.sha256);
    assert_eq!(row.url, url);
    assert_eq!(row.server_id.as_deref(), Some(messenger_media::service::FIXED_SERVER_ID));
    assert_eq!((row.set_at, row.checked_at, row.touched_at), (T0, T0, T0));
    let copies: Vec<PublicBlob> = serde_json::from_str(&row.copies_json).unwrap();
    assert_eq!(copies.len(), 1, "one server, one copy");

    let jpeg = r.backend.get(&row.sha256).expect("on the server");
    assert_eq!(avatar::sha256_hex(&jpeg), row.sha256);
    assert_eq!(&jpeg[..2], &[0xff, 0xd8], "a JPEG");
    assert_eq!(r.backend.content_type(&row.sha256).as_deref(), Some(JPEG));
    assert_eq!(std::fs::read(r.own_file(&row.sha256)).unwrap(), jpeg, "kept on this device");
    let side = avatar::decode(&jpeg).unwrap();
    assert_eq!((side.width(), side.height()), (avatar::OWN_SIDE, avatar::OWN_SIDE));

    assert_eq!(r.picture().await.as_deref(), Some(url.as_str()));
    assert_eq!(r.outbox.pending().await.unwrap(), 1, "kind 0 queued");
    assert_eq!(code(r.av.set(&r.keys, &p.token, rect).await), "avatar_expired", "a token serves once");
}

#[tokio::test]
async fn a_new_avatar_takes_the_old_one_down() {
    let r = rig().await;
    let old = r.set(bmp(64, 64, [0, 0, 200])).await;
    let new = r.set(bmp(64, 64, [0, 200, 0])).await;
    assert_ne!(old.sha256, new.sha256);
    assert!(!r.own_file(&old.sha256).exists());
    assert!(r.own_file(&new.sha256).exists());
    let backend = r.backend.clone();
    let gone = old.sha256.clone();
    eventually("the old blob is deleted", move || backend.get(&gone).is_none()).await;
    assert!(r.backend.get(&new.sha256).is_some());

    // The same picture again: nothing is taken down.
    let again = r.set(bmp(64, 64, [0, 200, 0])).await;
    assert_eq!(again.sha256, new.sha256);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.backend.get(&new.sha256).is_some());
    assert!(r.own_file(&new.sha256).exists());
}

#[tokio::test]
async fn prepare_refusals_and_expiry() {
    let r = rig().await;
    assert_eq!(code(r.av.prepare(b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec()).await), "avatar_unsupported");
    assert_eq!(code(r.av.prepare(vec![0xff, 0xd8, 0xff, 0x00, 1, 2, 3]).await), "avatar_corrupt");
    assert_eq!(code(r.av.prepare(vec![0; avatar::MAX_INPUT_BYTES + 1]).await), "avatar_too_large");
    assert!(r.av.prepare_file(&r.dir.path().join("missing.png")).await.is_err());
    let path = r.dir.path().join("a.bmp");
    std::fs::write(&path, bmp(40, 30, [1, 2, 3])).unwrap();
    let first = r.av.prepare_file(&path).await.unwrap();
    assert_eq!((first.width, first.height), (40, 30));

    // One at a time: a second pick lets the first go.
    let second = r.av.prepare(bmp(50, 50, [9, 9, 9])).await.unwrap();
    let all = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    assert_eq!(code(r.av.set(&r.keys, &first.token, all).await), "avatar_expired");
    assert_eq!(code(r.av.set(&r.keys, &second.token, CropRect { x: 0.5, y: 0.5, w: 0.9, h: 0.9 }).await), "avatar_bad_crop");
    // A bad crop keeps the picture for another try.
    r.av.prepared.lock().unwrap().as_mut().unwrap().at = Instant::now().checked_sub(PREPARED_TTL).unwrap();
    assert_eq!(code(r.av.set(&r.keys, &second.token, all).await), "avatar_expired", "ten minutes");
    assert!(r.backend.is_empty());
    assert_eq!(r.picture().await, None);
}

#[tokio::test]
async fn set_without_a_server_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().await.unwrap();
    let media = MediaService::new(store.clone(), Arc::new(MemorySecretStore::unlocked()), dir.path()).unwrap();
    let r = rig_with_media(store, media, MemoryBackend::new(BASE), Keys::generate(), MemNet::default(), dir);
    let p = r.av.prepare(bmp(32, 32, [5, 5, 5])).await.unwrap();
    let all = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let e = r.av.set(&r.keys, &p.token, all).await.unwrap_err();
    assert!(e.to_string().contains("err.media_no_server"), "{e}");
    assert_eq!(r.row().await, None);
    assert_eq!(r.picture().await, None);
    assert_eq!(std::fs::read_dir(r.dir.path().join("avatars/own")).unwrap().count(), 0, "no file left behind");
    assert!(r.av.prepared(&p.token).is_ok(), "the picture waits for another try");
}

#[tokio::test]
async fn remove_takes_the_picture_away() {
    let r = rig().await;
    let row = r.set(bmp(64, 64, [100, 100, 0])).await;
    r.av.remove(&r.keys).await.unwrap();
    assert_eq!(r.picture().await, None);
    assert_eq!(r.row().await, None);
    assert!(!r.own_file(&row.sha256).exists());
    let backend = r.backend.clone();
    eventually("the blob is deleted", move || backend.is_empty()).await;
    // Nothing to remove is not an error.
    r.av.remove(&r.keys).await.unwrap();
}

#[tokio::test]
async fn remove_on_a_device_that_did_not_take_the_avatar_on_yet() {
    let a = rig().await;
    let row = a.set(bmp(64, 64, [1, 100, 0])).await;
    let b = rig_on(a.backend.clone(), a.keys.clone(), MemNet::default()).await;
    let content = serde_json::json!({ "name": "me", "picture": row.url }).to_string();
    b.profiles.apply_event(&b.me(), Timestamp(T0), &content).await.unwrap();
    b.av.remove(&b.keys).await.unwrap();
    assert_eq!(b.picture().await, None);
    let backend = a.backend.clone();
    eventually("the blob is deleted", move || backend.is_empty()).await;
}

// ─── The keeper ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_keeper_puts_back_what_the_server_lost_and_keeps_it_alive() {
    let r = rig().await;
    let row = r.set(bmp(64, 64, [7, 7, 7])).await;
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![], "nothing due");

    // A day later the server has lost it.
    r.clock.advance(DAY);
    r.backend.blobs.lock().unwrap().clear();
    let steps = r.av.keep(&r.keys).await.unwrap();
    assert_eq!(steps, vec![Step::Check { server_id: "fixed".into(), sha: row.sha256.clone() }]);
    assert!(r.backend.get(&row.sha256).is_some(), "put back");
    assert_eq!(r.row().await.unwrap().checked_at, T0 + DAY);
    assert_eq!(r.picture().await.as_deref(), Some(row.url.as_str()), "the address is the same");
    assert!(r.net.calls().is_empty(), "a check is no download");

    // Five days after the upload: fetched past the caches.
    r.clock.advance(4 * DAY);
    let steps = r.av.keep(&r.keys).await.unwrap();
    assert!(steps.contains(&Step::Touch { url: row.url.clone(), server_id: "fixed".into(), sha: row.sha256.clone() }), "{steps:?}");
    let fresh = r.net.fresh.lock().unwrap().clone();
    assert_eq!(fresh.len(), 1);
    assert!(fresh[0].starts_with(&format!("{}?k=", row.url)), "{fresh:?}");
    assert_eq!(r.row().await.unwrap().touched_at, T0 + 5 * DAY);

    // Gone again at the next touch: put back as well.
    r.clock.advance(5 * DAY);
    r.backend.blobs.lock().unwrap().clear();
    let puts = *r.backend.put_calls.lock().unwrap();
    r.av.keep(&r.keys).await.unwrap();
    assert!(r.backend.get(&row.sha256).is_some());
    assert!(*r.backend.put_calls.lock().unwrap() > puts);
    assert_eq!(r.row().await.unwrap().touched_at, T0 + 10 * DAY);
}

#[tokio::test]
async fn the_keeper_moves_the_picture_to_a_server_i_use() {
    let r = rig().await;
    let row = r.set(bmp(64, 64, [3, 30, 90])).await;
    // Named on a server this device no longer uses.
    let old_url = format!("https://old.example/{}", row.sha256);
    own_avatar::set(&r.store, &own_avatar::OwnAvatar { url: old_url.clone(), server_id: Some("old".into()), ..row.clone() })
        .await
        .unwrap();
    r.profiles.build_picture(&r.keys, Picture::Set(&old_url)).await.unwrap();
    r.backend.blobs.lock().unwrap().clear();
    let mut rx = r.ui.subscribe();

    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![Step::Rehost { sha: row.sha256.clone() }]);
    let moved = r.row().await.unwrap();
    assert_eq!(moved.url, row.url, "back on the server in use, the same hash");
    assert_eq!(moved.sha256, row.sha256);
    assert!(r.backend.get(&row.sha256).is_some());
    assert_eq!(r.picture().await.as_deref(), Some(row.url.as_str()), "kind 0 names it");
    let ev = rx.try_recv().unwrap();
    assert_eq!((ev.name.as_str(), ev.payload["pubkey"].as_str()), (UI_EVENT_PROFILE_UPDATED, Some(r.keys.public_key().to_hex().as_str())));
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![]);
}

#[tokio::test]
async fn another_device_takes_the_avatar_on_and_follows_its_removal() {
    let a = rig().await;
    let row = a.set(bmp(80, 60, [250, 120, 0])).await;
    let jpeg = a.backend.get(&row.sha256).unwrap();

    // My second device: the same servers, the same key, my kind 0 from a relay.
    let b = rig_on(a.backend.clone(), a.keys.clone(), MemNet::default()).await;
    assert_eq!(b.av.keep(&b.keys).await.unwrap(), vec![], "no kind 0 of mine yet");
    let content = serde_json::json!({ "name": "me", "picture": row.url }).to_string();
    b.profiles.apply_event(&b.me(), Timestamp(T0 + 5), &content).await.unwrap();
    let mut rx = b.ui.subscribe();
    let steps = b.av.keep(&b.keys).await.unwrap();
    assert_eq!(
        steps,
        vec![
            Step::Adopt { url: row.url.clone(), sha: row.sha256.clone(), server_id: "fixed".into() },
            Step::Download { url: row.url.clone(), sha: row.sha256.clone() },
        ]
    );
    assert_eq!(std::fs::read(b.own_file(&row.sha256)).unwrap(), jpeg);
    let held = b.row().await.unwrap();
    assert_eq!((held.url.as_str(), held.set_at), (row.url.as_str(), T0 + 5));
    b.ready(&mut rx, &row.url).await;
    // Shown from the file, without the network.
    let calls = b.net.calls().len();
    assert_eq!(data_bytes(&b.av.cached(&row.url, Some(&b.keys), true).await.unwrap().unwrap()), jpeg);
    assert_eq!(b.net.calls().len(), calls);
    assert_eq!(b.av.keep(&b.keys).await.unwrap(), vec![]);

    // Removed on the first device.
    b.profiles.apply_event(&b.me(), Timestamp(T0 + 10), r#"{"name":"me"}"#).await.unwrap();
    assert_eq!(b.av.keep(&b.keys).await.unwrap(), vec![Step::Forget { sha: row.sha256.clone(), keep_file: false }]);
    assert_eq!(b.row().await, None);
    assert!(!b.own_file(&row.sha256).exists());
}

#[tokio::test]
async fn a_download_that_does_not_match_its_hash_is_refused() {
    let a = rig().await;
    let row = a.set(bmp(64, 64, [10, 20, 30])).await;
    let b = rig_on(a.backend.clone(), a.keys.clone(), MemNet::default()).await;
    let content = serde_json::json!({ "picture": row.url }).to_string();
    b.profiles.apply_event(&b.me(), Timestamp(T0 + 5), &content).await.unwrap();
    // What the server hands out is something else.
    a.backend.blobs.lock().unwrap().insert(row.sha256.clone(), b"not it".to_vec());
    let e = b.av.keep(&b.keys).await.unwrap_err();
    assert!(e.to_string().contains("avatar_unavailable"), "{e}");
    assert!(!b.own_file(&row.sha256).exists());
}

/// Two Blossom servers: the avatar goes on both; when the first is turned
/// off, the keeper names the copy on the second.
#[tokio::test]
async fn over_blossom_two_copies_and_a_move() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let (s1, s2) = (MockServer::start().await, MockServer::start().await);
    for s in [&s1, &s2] {
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(s).await;
        Mock::given(method("HEAD")).respond_with(ResponseTemplate::new(200)).mount(s).await;
        Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(200)).mount(s).await;
    }
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().await.unwrap();
    let media = MediaService::new(store.clone(), Arc::new(MemorySecretStore::unlocked()), dir.path()).unwrap();
    for (id, s, prio) in [("one", &s1, 10), ("two", &s2, 20)] {
        let input = MediaServerInput { id: Some(id.into()), kind: "blossom".into(), url: s.uri(), priority: Some(prio), ..Default::default() };
        media.put_server(input).await.unwrap();
    }
    let r = rig_with_media(store, media.clone(), MemoryBackend::new(BASE), Keys::generate(), MemNet::default(), dir);
    let row = r.set(bmp(64, 64, [42, 42, 42])).await;
    assert_eq!(row.url, format!("{}/{}", s1.uri(), row.sha256));
    let copies: Vec<PublicBlob> = serde_json::from_str(&row.copies_json).unwrap();
    assert_eq!(copies.iter().map(|c| c.server_id.as_str()).collect::<Vec<_>>(), vec!["one", "two"]);

    media.set_server_enabled("one", false).await.unwrap();
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![Step::Rehost { sha: row.sha256.clone() }]);
    let url2 = format!("{}/{}", s2.uri(), row.sha256);
    assert_eq!(r.row().await.unwrap().url, url2);
    assert_eq!(r.picture().await.as_deref(), Some(url2.as_str()));
    let puts = s2.received_requests().await.unwrap().iter().filter(|q| q.method.as_str() == "PUT").count();
    assert_eq!(puts, 2, "once with the first copy, once on the move");
    // Taken off the server it moved from, and only there.
    let mut deleted = 0;
    for _ in 0..200 {
        deleted = s1.received_requests().await.unwrap().iter().filter(|q| q.method.as_str() == "DELETE").count();
        if deleted > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(deleted, 1);
    assert_eq!(s2.received_requests().await.unwrap().iter().filter(|q| q.method.as_str() == "DELETE").count(), 0);

    // A day later: asked by HEAD, still there.
    r.clock.advance(DAY);
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![Step::Check { server_id: "two".into(), sha: row.sha256.clone() }]);
    let heads = s2.received_requests().await.unwrap().iter().filter(|q| q.method.as_str() == "HEAD").count();
    assert_eq!(heads, 1);
}

// ─── Others ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_picture_of_someone_else_is_fetched_once_and_kept_small() {
    let r = rig().await;
    let picture = bmp(600, 400, [0, 120, 255]);
    let sha = avatar::sha256_hex(&picture);
    let url = format!("https://cdn.example/{sha}.png");
    r.net.serve(&url, picture);
    let mut rx = r.ui.subscribe();
    assert_eq!(r.av.cached(&url, None, true).await.unwrap(), None, "fetched in the background");
    r.ready(&mut rx, &url).await;
    let jpeg = data_bytes(&r.av.cached(&url, None, true).await.unwrap().unwrap());
    let thumb = avatar::decode(&jpeg).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (avatar::CACHE_SIDE, avatar::CACHE_SIDE));
    assert!(r.dir.path().join("avatars").join(format!("{sha}.jpg")).exists(), "named by the hash it carries");
    assert_eq!(r.net.calls(), vec![format!("foreign {url}")], "asked once");
    let row = avatar_cache::get(&r.store, &url).await.unwrap().unwrap();
    assert_eq!((row.sha256.as_deref(), row.fetched_at), (Some(sha.as_str()), T0));

    // An address without a hash is named by the address, and looked at
    // again a week later; the old picture is shown meanwhile.
    let plain = "https://cdn.example/me.png";
    r.net.serve(plain, bmp(50, 50, [1, 1, 1]));
    assert_eq!(r.av.cached(plain, None, true).await.unwrap(), None);
    r.ready(&mut rx, plain).await;
    assert!(r.dir.path().join("avatars").join(format!("{}.jpg", avatar::sha256_hex(plain.as_bytes()))).exists());
    r.clock.advance(REFRESH_SECS);
    assert!(r.av.cached(plain, None, true).await.unwrap().is_some());
    r.ready(&mut rx, plain).await;
    assert_eq!(r.net.calls().iter().filter(|c| c.ends_with(plain)).count(), 2);
    // An address with a hash never changes.
    assert!(r.av.cached(&url, None, true).await.unwrap().is_some());
    assert_eq!(r.net.calls().iter().filter(|c| c.ends_with(&url)).count(), 1);
}

#[tokio::test]
async fn a_wrong_or_missing_picture_is_looked_for_on_my_servers() {
    let r = rig().await;
    let picture = bmp(64, 64, [9, 90, 9]);
    let sha = avatar::sha256_hex(&picture);
    r.backend.blobs.lock().unwrap().insert(sha.clone(), picture);
    let url = format!("https://gone.example/{sha}");
    r.net.serve(&url, b"something else".to_vec());
    let mut rx = r.ui.subscribe();
    assert_eq!(r.av.cached(&url, Some(&r.keys), true).await.unwrap(), None);
    r.ready(&mut rx, &url).await;
    assert!(r.av.cached(&url, Some(&r.keys), true).await.unwrap().is_some());
    assert_eq!(r.net.calls(), vec![format!("foreign {url}"), format!("own {BASE}/{sha}")]);

    // On my own server: the way of the media, never the guarded fetcher.
    let other = bmp(64, 64, [1, 2, 250]);
    let sha2 = avatar::sha256_hex(&other);
    r.backend.blobs.lock().unwrap().insert(sha2.clone(), other);
    let mine = format!("{BASE}/{sha2}");
    assert_eq!(r.av.cached(&mine, Some(&r.keys), true).await.unwrap(), None);
    r.ready(&mut rx, &mine).await;
    assert_eq!(r.net.calls().last().unwrap(), &format!("own {mine}"));
    assert!(!r.net.calls().contains(&format!("foreign {mine}")));
}

#[tokio::test]
async fn failures_are_remembered() {
    let r = rig().await;
    let url = "https://down.example/a.png";
    let mut rx = r.ui.subscribe();
    for attempt in 1..=8 {
        assert_eq!(r.av.cached(url, None, true).await.unwrap(), None);
        attempts_reach(&r.store, url, attempt).await;
        // Asked again at once: not tried.
        r.av.cached(url, None, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(r.net.calls().len() as i64, attempt);
        r.clock.advance(retry_after(attempt) - 1);
        r.av.cached(url, None, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(r.net.calls().len() as i64, attempt, "not before its time");
        r.clock.advance(1);
    }
    assert!(rx.try_recv().is_err(), "no avatar.ready for a failure");

    // The network is back, however often it failed before.
    r.net.serve(url, bmp(40, 40, [5, 50, 5]));
    r.clock.advance(365 * DAY);
    assert_eq!(r.av.cached(url, None, true).await.unwrap(), None);
    r.ready(&mut rx, url).await;
    assert!(r.av.cached(url, None, true).await.unwrap().is_some());

    // Garbage is a failure too, and nothing is kept.
    let junk = "https://junk.example/a.png";
    r.net.serve(junk, b"<html>".to_vec());
    r.av.cached(junk, None, true).await.unwrap();
    attempts_reach(&r.store, junk, 1).await;
    assert!(!r.dir.path().join("avatars").join(format!("{}.jpg", avatar::sha256_hex(junk.as_bytes()))).exists());
}

/// Waits until the failures of `url` are `n`.
async fn attempts_reach(store: &Store, url: &str, n: i64) {
    for _ in 0..200 {
        if avatar_cache::get(store, url).await.unwrap().map(|row| row.attempts) == Some(n) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{url}: not {n} failures");
}

#[tokio::test]
async fn fetches_are_deduplicated_and_limited() {
    let net = MemNet { delay: Some(Duration::from_millis(80)), ..Default::default() };
    let r = rig_on(MemoryBackend::new(BASE), Keys::generate(), net).await;
    let urls: Vec<String> = (0..7).map(|i| format!("https://cdn.example/{i}.png")).collect();
    for (i, u) in urls.iter().enumerate() {
        r.net.serve(u, bmp(20, 20, [i as u8 * 30, 0, 0]));
    }
    let mut rx = r.ui.subscribe();
    for u in &urls {
        for _ in 0..3 {
            assert_eq!(r.av.cached(u, None, true).await.unwrap(), None);
        }
    }
    // In any order.
    let mut seen = HashSet::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while seen.len() < urls.len() {
            let ev = rx.recv().await.unwrap();
            if ev.name == UI_EVENT_AVATAR_READY {
                seen.insert(ev.payload["url"].as_str().unwrap().to_string());
            }
        }
    })
    .await
    .expect("every avatar.ready");
    assert_eq!(r.net.calls().len(), urls.len(), "one fetch per address");
    let most = r.net.most.load(Ordering::SeqCst);
    assert!((1..=MAX_FETCHES).contains(&most), "{most} at once");
}

#[tokio::test]
async fn nothing_is_fetched_when_it_may_not_be_or_cannot_be() {
    let r = rig().await;
    assert_eq!(r.av.cached("https://cdn.example/a.png", None, false).await.unwrap(), None, "silent mode");
    for bad in ["javascript:alert(1)", "data:image/png;base64,AAAA", "file:///etc/passwd", "https://a.example/a b", ""] {
        assert_eq!(r.av.cached(bad, None, true).await.unwrap(), None, "{bad:?}");
    }
    let long = format!("https://a.example/{}", "x".repeat(messenger_preview::target::MAX_URL_LEN));
    assert_eq!(r.av.cached(&long, None, true).await.unwrap(), None);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(r.net.calls().is_empty());
}

// ─── After the review ───────────────────────────────────────────────────────

/// My avatar as `set` makes it, from `bmp` bytes.
fn own_jpeg(rgb: [u8; 3]) -> Vec<u8> {
    let all = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    avatar::crop_square(&avatar::decode(&bmp(64, 64, rgb)).unwrap(), all, avatar::OWN_SIDE).unwrap()
}

fn requests(reqs: &[wiremock::Request], verb: &str) -> usize {
    reqs.iter().filter(|q| q.method.as_str() == verb).count()
}

#[test]
fn the_size_of_a_jpeg_is_read_from_its_header() {
    assert_eq!(jpeg_size(&own_jpeg([1, 2, 3])), Some((avatar::OWN_SIDE, avatar::OWN_SIDE)));
    let wide = avatar::preview_jpeg(&avatar::decode(&bmp(90, 30, [1, 2, 3])).unwrap(), 1024);
    assert_eq!(jpeg_size(&wide), Some((90, 30)));
    for junk in [&b""[..], b"\xff\xd8", b"\xff\xd8\xff\xc0\x00", b"<svg/>", &bmp(4, 4, [0, 0, 0])] {
        assert_eq!(jpeg_size(junk), None);
    }
}

/// What kind 0 names on my server but this app did not make (an SVG, a
/// picture of another size) is not taken for my avatar, and never reaches
/// the page as it came.
#[tokio::test]
async fn a_picture_another_client_put_on_my_server_is_not_taken_on() {
    let r = rig().await;
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#.to_vec();
    let small = avatar::square_thumb(&bmp(64, 64, [1, 2, 3]), avatar::CACHE_SIDE).unwrap();
    for (i, body) in [svg, small].into_iter().enumerate() {
        let sha = avatar::sha256_hex(&body);
        r.backend.blobs.lock().unwrap().insert(sha.clone(), body);
        let url = format!("{BASE}/{sha}");
        let content = serde_json::json!({ "name": "me", "picture": url }).to_string();
        r.profiles.apply_event(&r.me(), Timestamp(4_000_000_000 + i as i64), &content).await.unwrap();
        assert_eq!(code(r.av.keep(&r.keys).await), "avatar_not_ours");
        assert!(!r.own_file(&sha).exists());
        assert_eq!(r.row().await, None, "not taken on");
        let shown = r.av.cached(&url, Some(&r.keys), true).await.unwrap();
        assert!(shown.is_none(), "no file of mine to show it from");
    }
    let svg_url = format!("{BASE}/{}", avatar::sha256_hex(br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#));
    attempts_reach(&r.store, &svg_url, 1).await;
    assert_eq!(r.av.cached(&svg_url, Some(&r.keys), true).await.unwrap(), None, "and as anyone's picture it is no picture");
}

/// Away for a day, my second device still holds the avatar my first one
/// removed: it does not put it back before a relay tells it my kind 0.
#[tokio::test]
async fn a_device_back_from_away_does_not_bring_back_a_removed_avatar() {
    let a = rig().await;
    let row = a.set(bmp(64, 64, [90, 9, 9])).await;
    let b = rig_on(a.backend.clone(), a.keys.clone(), MemNet::default()).await;
    let content = serde_json::json!({ "name": "me", "picture": row.url }).to_string();
    b.profiles.apply_event(&b.me(), Timestamp(T0 + 5), &content).await.unwrap();
    assert_eq!(b.av.keep(&b.keys).await.unwrap().len(), 2, "taken on");
    assert!(b.own_file(&row.sha256).exists());

    a.av.remove(&a.keys).await.unwrap();
    let backend = a.backend.clone();
    eventually("the blob is deleted", move || backend.is_empty()).await;

    // A new session a day later; the cache still names the picture.
    b.av.session_started();
    b.clock.advance(DAY);
    assert_eq!(b.av.keep(&b.keys).await.unwrap(), vec![Step::Check { server_id: "fixed".into(), sha: row.sha256.clone() }]);
    assert!(b.backend.is_empty(), "not put back on an old kind 0");
    assert_eq!(b.row().await.unwrap().checked_at, T0, "asked again on the next run");

    // The relay tells the removal.
    b.profiles.apply_event(&b.me(), Timestamp(T0 + 10), r#"{"name":"me"}"#).await.unwrap();
    b.profiles.note_own_heard();
    assert_eq!(b.av.keep(&b.keys).await.unwrap(), vec![Step::Forget { sha: row.sha256.clone(), keep_file: false }]);
    assert!(b.backend.is_empty());
}

/// Put back by this device just before another one removed it: taken down
/// again when the removal comes.
#[tokio::test]
async fn a_picture_put_back_here_goes_down_again_when_kind_0_drops_it() {
    let a = rig().await;
    let row = a.set(bmp(64, 64, [9, 90, 9])).await;
    let b = rig_on(a.backend.clone(), a.keys.clone(), MemNet::default()).await;
    let content = serde_json::json!({ "name": "me", "picture": row.url }).to_string();
    b.profiles.apply_event(&b.me(), Timestamp(T0 + 5), &content).await.unwrap();
    b.av.keep(&b.keys).await.unwrap();

    a.backend.blobs.lock().unwrap().clear();
    b.clock.advance(DAY);
    b.av.keep(&b.keys).await.unwrap();
    assert!(b.backend.get(&row.sha256).is_some(), "put back");

    b.profiles.apply_event(&b.me(), Timestamp(T0 + 10), r#"{"name":"me"}"#).await.unwrap();
    assert_eq!(b.av.keep(&b.keys).await.unwrap(), vec![Step::Forget { sha: row.sha256.clone(), keep_file: false }]);
    let backend = b.backend.clone();
    eventually("taken down again", move || backend.is_empty()).await;
}

/// The S3 server of my avatar cannot be written to while its secret is
/// out of reach: the picture stays where it is, named as it is.
#[tokio::test]
async fn a_server_that_cannot_be_written_now_is_not_moved_away_from() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(&s).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().await.unwrap();
    let media = MediaService::new(store.clone(), Arc::new(MemorySecretStore::unlocked()), dir.path()).unwrap();
    let s3 = MediaServerInput {
        id: Some("s3".into()),
        kind: "s3".into(),
        url: "https://s3.example".into(),
        bucket: Some("avatars".into()),
        access_key: Some("AK".into()),
        priority: Some(1),
        ..Default::default()
    };
    media.put_server(s3).await.unwrap();
    let blossom = MediaServerInput { id: Some("b".into()), kind: "blossom".into(), url: s.uri(), priority: Some(20), ..Default::default() };
    media.put_server(blossom).await.unwrap();
    let r = rig_with_media(store, media, MemoryBackend::new(BASE), Keys::generate(), MemNet::default(), dir);

    let jpeg = own_jpeg([1, 1, 1]);
    let sha = avatar::sha256_hex(&jpeg);
    std::fs::create_dir_all(r.own_file(&sha).parent().unwrap()).unwrap();
    std::fs::write(r.own_file(&sha), &jpeg).unwrap();
    let url = format!("https://s3.example/avatars/{sha}");
    let row = own_avatar::OwnAvatar { sha256: sha, url: url.clone(), server_id: Some("s3".into()), copies_json: "[]".into(), set_at: T0, checked_at: T0, touched_at: T0 };
    own_avatar::set(&r.store, &row).await.unwrap();
    r.profiles.build_picture(&r.keys, Picture::Set(&url)).await.unwrap();

    r.clock.advance(DAY);
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![]);
    assert_eq!(requests(&s.received_requests().await.unwrap(), "PUT"), 0, "nothing moved");
    assert_eq!(r.picture().await.as_deref(), Some(url.as_str()));
    assert_eq!(r.outbox.pending().await.unwrap(), 0, "nothing published");
}

/// A move whose kind 0 cannot be made leaves the row as it was, so the
/// next run moves it again instead of forgetting it.
#[tokio::test]
async fn a_move_that_cannot_be_published_is_tried_again() {
    let r = rig().await;
    let row = r.set(bmp(64, 64, [4, 40, 4])).await;
    let old_url = format!("https://old.example/{}", row.sha256);
    own_avatar::set(&r.store, &own_avatar::OwnAvatar { url: old_url.clone(), server_id: Some("old".into()), ..row.clone() })
        .await
        .unwrap();
    // Another client filled my kind 0 to the brim: the new address, two
    // bytes longer, does not fit.
    let at = r.profiles.get(&r.me()).await.unwrap().unwrap().event_created_at + 1;
    let base = serde_json::json!({ "picture": old_url, "x_other": "" }).to_string().len();
    let full = serde_json::json!({ "picture": old_url, "x_other": "x".repeat(messenger_contacts::profile::MAX_CONTENT_BYTES - base) }).to_string();
    assert_eq!(full.len(), messenger_contacts::profile::MAX_CONTENT_BYTES);
    assert!(r.profiles.apply_event(&r.me(), Timestamp(at), &full).await.unwrap());

    assert_eq!(code(r.av.keep(&r.keys).await), "profile_too_large");
    assert_eq!(r.row().await.unwrap().url, old_url, "the row waits for its kind 0");
    assert_eq!(r.picture().await.as_deref(), Some(old_url.as_str()));

    // Room again: the move is made.
    let small = serde_json::json!({ "picture": old_url }).to_string();
    r.profiles.apply_event(&r.me(), Timestamp(at + 1), &small).await.unwrap();
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![Step::Rehost { sha: row.sha256.clone() }]);
    assert_eq!(r.picture().await.as_deref(), Some(row.url.as_str()));
    assert_eq!(r.row().await.unwrap().url, row.url);
}

/// With no server of mine left to take it, my avatar is still fetched now
/// and then where it is, so that server keeps it.
#[tokio::test]
async fn without_a_server_the_avatar_is_kept_alive_where_it_is() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(&s).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().await.unwrap();
    let media = MediaService::new(store.clone(), Arc::new(MemorySecretStore::unlocked()), dir.path()).unwrap();
    let blossom = MediaServerInput { id: Some("b".into()), kind: "blossom".into(), url: s.uri(), ..Default::default() };
    media.put_server(blossom).await.unwrap();
    let r = rig_with_media(store, media.clone(), MemoryBackend::new(BASE), Keys::generate(), MemNet::default(), dir);
    let row = r.set(bmp(64, 64, [8, 8, 80])).await;
    media.set_server_enabled("b", false).await.unwrap();

    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![], "not due");
    r.clock.advance(TOUCH_EVERY_SECS);
    assert_eq!(r.av.keep(&r.keys).await.unwrap(), vec![Step::KeepAlive { url: row.url.clone(), sha: row.sha256.clone() }]);
    let fresh = r.net.fresh.lock().unwrap().clone();
    assert!(fresh.len() == 1 && fresh[0].starts_with(&format!("{}?k=", row.url)), "{fresh:?}");
    assert_eq!(requests(&s.received_requests().await.unwrap(), "PUT"), 1, "nothing put on a server turned off");
}

/// The pictures of others shown lately stay; those nobody looked at for a
/// month go, files and all.
#[tokio::test]
async fn pictures_nobody_looks_at_are_let_go() {
    let r = rig().await;
    let mut rx = r.ui.subscribe();
    let mut urls = Vec::new();
    for rgb in [[1, 1, 1], [2, 2, 2]] {
        let body = bmp(30, 30, rgb);
        let url = format!("https://cdn.example/{}.png", avatar::sha256_hex(&body));
        r.net.serve(&url, body);
        assert_eq!(r.av.cached(&url, None, true).await.unwrap(), None);
        r.ready(&mut rx, &url).await;
        urls.push(url);
    }
    let file = |url: &str| r.dir.path().join("avatars").join(format!("{}.jpg", hash_of(url).unwrap()));
    assert!(file(&urls[0]).exists() && file(&urls[1]).exists());

    r.clock.advance(20 * DAY);
    assert!(r.av.cached(&urls[1], None, true).await.unwrap().is_some(), "shown");
    r.clock.advance(15 * DAY);
    assert_eq!(r.av.sweep().await.unwrap(), 1);
    assert!(!file(&urls[0]).exists(), "a month unseen: gone");
    assert_eq!(avatar_cache::get(&r.store, &urls[0]).await.unwrap(), None);
    assert!(file(&urls[1]).exists());
    assert!(r.av.cached(&urls[1], None, true).await.unwrap().is_some());
}

// ─── HTTP ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn http_own_goes_past_caches_and_reads_within_limits() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    messenger_transport::ensure_crypto_provider();
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/here")).and(header("cache-control", "no-cache")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"fresh".to_vec())).mount(&s).await;
    Mock::given(method("GET")).and(path("/here")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"cached".to_vec())).mount(&s).await;
    Mock::given(method("GET")).and(path("/gone")).respond_with(ResponseTemplate::new(404)).mount(&s).await;
    Mock::given(method("GET")).and(path("/broken")).respond_with(ResponseTemplate::new(503)).mount(&s).await;
    Mock::given(method("GET")).and(path("/big")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 5000])).mount(&s).await;
    let net = HttpAvatarNet::new().unwrap();
    let here = format!("{}/here", s.uri());
    assert_eq!(net.own(&unique(&here), 100, true).await.unwrap().as_deref(), Some(&b"fresh"[..]));
    assert_eq!(net.own(&here, 100, false).await.unwrap().as_deref(), Some(&b"cached"[..]));
    assert_eq!(net.own(&format!("{}/gone", s.uri()), 100, false).await.unwrap(), None);
    assert!(net.own(&format!("{}/broken", s.uri()), 100, false).await.is_err());
    assert_eq!(code(net.own(&format!("{}/big", s.uri()), 1000, false).await), "avatar_too_large");
    let fresh = s.received_requests().await.unwrap().into_iter().find(|q| q.headers.get("cache-control").is_some()).unwrap();
    assert!(fresh.url.query().unwrap_or_default().starts_with("k="), "a query no cache has seen");
}

#[tokio::test]
async fn http_foreign_is_guarded() {
    use messenger_preview::{Fetched, Fetcher, Target};
    struct Fake(bool);
    #[async_trait]
    impl Fetcher for Fake {
        async fn get(&self, target: &Target, accept: &str, max: usize) -> Result<Fetched> {
            assert_eq!((accept, max), ("image/*", 77));
            Ok(Fetched { url: target.url().into(), content_type: "image/png".into(), body: b"png".to_vec(), truncated: self.0 })
        }
    }
    messenger_transport::ensure_crypto_provider();
    let net = HttpAvatarNet::with_foreign(Arc::new(Fake(false))).unwrap();
    assert_eq!(net.foreign("https://cdn.example/a.png", 77).await.unwrap(), b"png");
    let net = HttpAvatarNet::with_foreign(Arc::new(Fake(true))).unwrap();
    assert_eq!(code(net.foreign("https://cdn.example/a.png", 77).await), "avatar_too_large");
    // The real one refuses before any request.
    let net = HttpAvatarNet::new().unwrap();
    assert_eq!(code(net.foreign("http://cdn.example/a.png", 77).await), "preview_not_https");
    assert_eq!(code(net.foreign("https://127.0.0.1/a.png", 77).await), "preview_private_host");
    assert_eq!(code(net.foreign("https://192.168.1.1/a.png", 77).await), "preview_private_host");
}

#[tokio::test]
async fn the_session_keeper_wakes_when_kicked() {
    let r = rig().await;
    let row = r.set(bmp(64, 64, [3, 3, 90])).await;
    let old_url = format!("https://old.example/{}", row.sha256);
    own_avatar::set(&r.store, &own_avatar::OwnAvatar { url: old_url.clone(), ..row.clone() }).await.unwrap();
    r.profiles.build_picture(&r.keys, Picture::Set(&old_url)).await.unwrap();
    let keeper = tokio::spawn(keeper_loop(r.av.clone(), r.keys.clone(), || false));
    r.av.kick();
    for _ in 0..200 {
        if r.row().await.unwrap().url == row.url {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    keeper.abort();
    assert_eq!(r.picture().await.as_deref(), Some(row.url.as_str()), "moved by the keeper after the kick");
}

#[tokio::test]
async fn through_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = messenger_core::MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(cfg.clone(), Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().set_silent(true).await.unwrap();
    crate::servers::use_veydan_offline(&rt).await;
    assert!(matches!(rt.avatar_remove().await, Err(MessengerError::NotLoggedIn)));
    rt.identity().create("pw").await.unwrap();
    rt.refresh_signer().await.unwrap();

    let path = dir.path().join("me.bmp");
    std::fs::write(&path, bmp(30, 40, [1, 2, 3])).unwrap();
    let p = rt.avatar_prepare(&path).await.unwrap();
    assert_eq!((p.width, p.height), (30, 40));
    assert!(rt.avatar_prepare_bytes(b"nope".to_vec()).await.is_err());
    assert_eq!(code(rt.avatar_set(&p.token, CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }).await), "avatar_expired", "the bad pick let it go");

    let me = rt.avatar_remove().await.unwrap();
    assert_eq!(me.picture, None);
    assert_eq!(rt.avatar_cached("https://cdn.example/a.png").await.unwrap(), None, "silent: nothing fetched");
    assert!(cfg.avatars_dir().starts_with(cfg.data_dir()));

    let spans = rt.bio_parse("**hi** https://example.com");
    assert!(matches!(&spans[0], Span::Text { text, style } if text == "hi" && style.bold), "{spans:?}");
    assert!(spans.iter().any(|s| matches!(s, Span::Link { url, .. } if url == "https://example.com")), "{spans:?}");
    let platforms = rt.social_platforms();
    assert!(platforms.iter().any(|p| p.id == "telegram"));
    assert!(platforms.iter().any(|p| p.id == "other"));
    rt.shutdown().await;
}

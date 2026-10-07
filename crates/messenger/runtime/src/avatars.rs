// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Avatars: mine, set from a picture the user picked and kept alive on my
//! media servers, and those of others, fetched here and handed to the page
//! as `data:` urls. The webview never loads a picture from the network.
//!
//! Mine: the picked file is decoded once and held in memory under a token
//! while the user crops it (`prepare`); the crop is encoded again as a
//! 512px JPEG, kept as `avatars/own/<sha256>.jpg` on every device of mine,
//! put open (not encrypted) on up to two of my media servers, and named in
//! kind 0 as `<public base>/<sha256>` (`set`). Nothing else writes
//! `picture`.
//!
//! The keeper (`keep`, run by the session at its start, once a day, when
//! the media servers change and when my kind 0 changes) makes kind 0 and
//! this device agree: it takes on a picture set on another device of mine
//! and fetches its file, forgets one that was removed, moves the picture to
//! a server I use when its server is no longer one of them, puts it back
//! when its server lost it, and fetches it every five days so that a
//! server that forgets what nobody reads keeps it. What it does on a run is
//! decided from the state alone (`plan`). It puts nothing back and
//! publishes nothing until a relay has told this session my kind 0, so a
//! picture removed or replaced on another device of mine while this one
//! was away is not brought back; a server enabled but not usable just now
//! (its secret not readable yet) is waited for, not moved away from. A
//! file it fetches is taken only when it is a picture this app made.
//!
//! Others: `cached` answers from `avatars/<name>.jpg` at once, or queues a
//! fetch and answers `None`; `avatar.ready {url}` follows when the picture
//! is here. A fetch is made once per address at a time, three at most,
//! and an address that failed is tried again after an hour, then twice as
//! long after each failure, a day at most. Pictures nobody looked at for a
//! month are forgotten (`sweep`). A picture on one of my media servers is
//! fetched the way the media go (bridges included); any other one only
//! over https, from addresses on the open internet (`messenger-preview`).
//! An address that ends in a SHA-256 must give those bytes, and when it
//! does not answer the blob is looked for on my servers. Whatever comes is
//! decoded within limits and encoded again as a 256px JPEG
//! (`messenger-avatar`).

use crate::MessengerRuntime;
use async_trait::async_trait;
use messenger_avatar::{self as avatar, AvatarError, CropRect, Decoded};
use messenger_contacts::social::{self, SocialPlatform};
use messenger_contacts::{Picture, ProfileService, ProfileView, UI_EVENT_PROFILE_UPDATED};
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, MessengerError, Outbound, PubKey, Result};
use messenger_ingress::Outbox;
use messenger_media::{MediaService, PublicBlob};
use messenger_richtext::Span;
use messenger_store::{avatar_cache, own_avatar, settings, Store};
use nostr::key::Keys;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Notify, Semaphore};
use ts_rs::TS;

/// Emitted with `{url}` when the picture of that address is here.
pub const UI_EVENT_AVATAR_READY: &str = "avatar.ready";

/// The largest picked file read for an avatar.
pub const MAX_PICK_BYTES: usize = avatar::MAX_INPUT_BYTES;
/// Copies of my avatar, each on another server.
pub const OWN_COPIES: usize = 2;
/// How long a picked picture waits for its crop.
pub const PREPARED_TTL: Duration = Duration::from_secs(10 * 60);
/// The longer side of the picture the crop UI shows.
pub const PREVIEW_SIDE: u32 = 1024;
/// The largest picture fetched, of anybody.
pub const MAX_FETCH_BYTES: usize = 2 * 1024 * 1024;
/// Fetches of others' avatars at once.
pub const MAX_FETCHES: usize = 3;
/// An address that failed is tried again after this long, twice as long
/// after every failure in a row…
pub const RETRY_AFTER_SECS: i64 = 3600;
/// …but never waits longer than this: a phone that was offline a few
/// times does not give up on a picture for good.
pub const MAX_RETRY_SECS: i64 = 24 * 3600;
/// The picture of an address nobody looked at for this long is forgotten…
pub const CACHE_KEEP_SECS: i64 = 30 * 24 * 3600;
/// …and of the others, those beyond this many shown most lately.
pub const MAX_CACHED: i64 = 2000;
/// A picture shown is noted at most this often.
const USED_EVERY_SECS: i64 = 24 * 3600;
/// A picture whose address names no hash may change under it: it is
/// fetched again after this long (the old one is shown meanwhile).
pub const REFRESH_SECS: i64 = 7 * 24 * 3600;
/// The keeper looks whether my avatar is still on its server…
pub const CHECK_EVERY_SECS: i64 = 24 * 3600;
/// …and fetches it to keep it alive (Veydan's servers keep a blob for 14
/// days after it was last read).
pub const TOUCH_EVERY_SECS: i64 = 5 * 24 * 3600;
/// The keeper's first run waits for the relays and my kind 0.
const KEEPER_FIRST_RUN: Duration = Duration::from_secs(20);
const KEEPER_EVERY: Duration = Duration::from_secs(24 * 3600);
/// Changes come in bursts (a server added, then enabled).
const KEEPER_SETTLE: Duration = Duration::from_secs(3);

const JPEG: &str = "image/jpeg";
/// The hash of my avatar this device last put back on a server that lost
/// it (settings): when kind 0 drops that avatar, it is taken down again.
const KEY_PUT_BACK: &str = "avatar.put_back";

/// A picked picture, ready to be cropped.
#[derive(Clone, Debug, Serialize, TS)]
pub struct AvatarPreview {
    /// Names the picture for `avatar_set`; good for ten minutes, and only
    /// until another picture is picked.
    pub token: String,
    /// The whole picture, upright, as a `data:` url; the crop is given as
    /// fractions of it.
    pub preview: String,
    /// Of the picture itself, upright.
    pub width: u32,
    pub height: u32,
}

/// The network as avatars use it. The real one is HTTP; tests answer from
/// memory.
#[async_trait]
pub trait AvatarNet: Send + Sync {
    /// A blob on one of my media servers, the way the media go. `Ok(None)`:
    /// the server does not have it. `fresh`: past every cache on the way.
    async fn own(&self, url: &str, max_bytes: usize, fresh: bool) -> Result<Option<Vec<u8>>>;
    /// A picture anywhere else: https and the open internet only.
    async fn foreign(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>>;
}

/// HTTP: the client of the media for my servers, the guarded fetcher of
/// link previews for everything else.
pub struct HttpAvatarNet {
    own: reqwest::Client,
    foreign: Arc<dyn messenger_preview::Fetcher>,
}

impl HttpAvatarNet {
    pub fn new() -> Result<Self> {
        Self::with_foreign(Arc::new(messenger_preview::ReqwestFetcher::new()?))
    }

    pub fn with_foreign(foreign: Arc<dyn messenger_preview::Fetcher>) -> Result<Self> {
        let own = messenger_http::client(Duration::from_secs(10), Duration::from_secs(30))?;
        Ok(Self { own, foreign })
    }

    async fn own_once(&self, url: &str, max_bytes: usize, fresh: bool) -> Result<Option<Vec<u8>>> {
        let mut request = self.own.get(url);
        if fresh {
            request = request.header("cache-control", "no-cache").header("pragma", "no-cache");
        }
        let mut resp = request.send().await.map_err(transport)?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(MessengerError::Transport(format!("http {}", resp.status().as_u16())));
        }
        if resp.content_length().is_some_and(|n| n > max_bytes as u64) {
            return Err(MessengerError::Invalid(AvatarError::TooLarge.code().into()));
        }
        let mut out = Vec::new();
        while let Some(part) = resp.chunk().await.map_err(transport)? {
            if out.len() + part.len() > max_bytes {
                return Err(MessengerError::Invalid(AvatarError::TooLarge.code().into()));
            }
            out.extend_from_slice(&part);
        }
        Ok(Some(out))
    }
}

fn transport(e: reqwest::Error) -> MessengerError {
    MessengerError::Transport(e.to_string())
}

#[async_trait]
impl AvatarNet for HttpAvatarNet {
    async fn own(&self, url: &str, max_bytes: usize, fresh: bool) -> Result<Option<Vec<u8>>> {
        let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        messenger_http::following_route(&host, || self.own_once(url, max_bytes, fresh))
            .await
            .map_err(|e| MessengerError::Transport(e.to_string()))?
    }

    async fn foreign(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let target = messenger_preview::Target::parse(url)?;
        let got = self.foreign.get(&target, "image/*", max_bytes).await?;
        if got.truncated {
            return Err(MessengerError::Invalid(AvatarError::TooLarge.code().into()));
        }
        Ok(got.body)
    }
}

/// The picture being cropped.
struct Prepared {
    token: String,
    image: Arc<Decoded>,
    at: Instant,
}

/// Avatars: mine and those of others. Cheap to clone; clones share
/// everything.
#[derive(Clone)]
pub struct Avatars {
    store: Store,
    media: MediaService,
    profiles: ProfileService,
    outbox: Outbox,
    ui: broadcast::Sender<UiEvent>,
    clock: Arc<dyn Clock>,
    net: Arc<dyn AvatarNet>,
    /// `avatars/`: the cache of others; `own/` inside it holds mine.
    dir: PathBuf,
    prepared: Arc<Mutex<Option<Prepared>>>,
    /// Addresses being fetched.
    inflight: Arc<Mutex<HashSet<String>>>,
    slots: Arc<Semaphore>,
    /// Wakes the keeper.
    kick: Arc<Notify>,
    /// One change of my avatar at a time: set, remove, the keeper.
    own_lock: Arc<tokio::sync::Mutex<()>>,
    /// `ProfileService::own_heard` when the session started: a larger one
    /// now says my kind 0 came from a relay since (`Facts::fresh`).
    heard_from: Arc<AtomicU64>,
}

fn invalid(e: AvatarError) -> MessengerError {
    MessengerError::Invalid(e.code().into())
}

fn joined(e: tokio::task::JoinError) -> MessengerError {
    MessengerError::Io(e.to_string())
}

/// 64 lowercase hex characters.
fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The SHA-256 an address carries as its last segment (`…/<sha256>` or
/// `…/<sha256>.<ext>`, as Blossom serves it), lowercase.
pub fn hash_of(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let stem = match last.split_once('.') {
        Some((stem, ext)) if !ext.is_empty() && ext.len() <= 5 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) => stem,
        Some(_) => return None,
        None => last,
    };
    let stem = stem.to_ascii_lowercase();
    is_sha(&stem).then_some(stem)
}

/// The server of mine `url` is a blob of: `<base>/<sha256>` exactly.
fn server_of<'a>(url: &str, bases: &'a [(String, String)]) -> Option<&'a str> {
    bases.iter().find_map(|(id, base)| {
        let rest = url.strip_prefix(base.trim_end_matches('/'))?.strip_prefix('/')?;
        is_sha(rest).then_some(id.as_str())
    })
}

/// Whether `url` is anywhere under one of my servers.
fn under_my_server(url: &str, bases: &[(String, String)]) -> bool {
    bases.iter().any(|(_, base)| url.strip_prefix(base.trim_end_matches('/')).is_some_and(|r| r.starts_with('/')))
}

/// An address the UI may ask for: https (or under one of my servers),
/// within the length of a link, with nothing that is not printable.
fn acceptable(url: &str) -> bool {
    url.len() <= messenger_preview::target::MAX_URL_LEN
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && (url.starts_with("https://") || url.starts_with("http://"))
}

/// How long an address waits after `attempts` failures in a row.
pub fn retry_after(attempts: i64) -> i64 {
    let doublings = (attempts - 1).clamp(0, 16) as u32;
    RETRY_AFTER_SECS.saturating_mul(1 << doublings).min(MAX_RETRY_SECS)
}

/// Whether a fetch of an address may be made now.
pub fn may_try(row: Option<&avatar_cache::AvatarCacheRow>, now: i64) -> bool {
    match row {
        Some(r) if r.attempts > 0 => now - r.failed_at >= retry_after(r.attempts),
        _ => true,
    }
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    // A token only has to differ from the one before it; the clock does
    // when the system has no randomness to give.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        bytes.iter_mut().zip(nanos.to_le_bytes().iter().cycle()).for_each(|(b, n)| *b = *n);
    }
    hex::encode(bytes)
}

/// `url` with a query no cache on the way has seen.
fn unique(url: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}k={}", random_hex(8))
}

/// Write `bytes` to `path` whole or not at all.
async fn write_whole(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| MessengerError::Io("no folder".into()))?;
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!(".{}.tmp", random_hex(8)));
    tokio::fs::write(&tmp, bytes).await?;
    if let Err(e) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e.into());
    }
    Ok(())
}

/// Width and height of a JPEG from its frame header, without decoding it.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        // A start of frame: every 0xC_ but the tables (C4, C8, CC).
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let f = bytes.get(i + 5..i + 9)?;
            return Some((u32::from(u16::from_be_bytes([f[2], f[3]])), u32::from(u16::from_be_bytes([f[0], f[1]]))));
        }
        if len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Whether `bytes` are a picture as `set` makes them: a JPEG, `OWN_SIDE`
/// square, that decodes. Whatever else kind 0 names was put there by
/// another client and is never taken for my avatar.
async fn is_own_encode(bytes: &[u8]) -> bool {
    if jpeg_size(bytes) != Some((avatar::OWN_SIDE, avatar::OWN_SIDE)) {
        return false;
    }
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        avatar::decode(&bytes).is_ok_and(|d| (d.width(), d.height()) == (avatar::OWN_SIDE, avatar::OWN_SIDE))
    })
    .await
    .unwrap_or(false)
}

// ─── The keeper's plan ──────────────────────────────────────────────────────

/// What the keeper knows when it starts a run.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// My avatar as this device holds it.
    pub own: Option<own_avatar::OwnAvatar>,
    /// `picture` of my kind 0; `None` while no kind 0 of mine is known.
    pub picture: Option<Option<String>>,
    /// `(server id, public base)` of my enabled media servers that can be
    /// written to now.
    pub bases: Vec<(String, String)>,
    /// The same of every enabled one, also those that cannot be written to
    /// now (an S3 secret not readable yet): a server still in use.
    pub known: Vec<(String, String)>,
    /// Hashes of the files in `avatars/own/`.
    pub own_files: Vec<String>,
    /// A kind 0 of mine came from a relay this session: `picture` is not
    /// one another device of mine has since changed. Until then nothing is
    /// put back and nothing published.
    pub fresh: bool,
    pub now: i64,
}

/// One thing the keeper does, in the order of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Kind 0 no longer names the avatar this device holds: forget it, and
    /// its file unless the next one is the same picture.
    Forget { sha: String, keep_file: bool },
    /// Kind 0 names a picture on one of my servers, set on another device
    /// of mine: hold it here too.
    Adopt { url: String, sha: String, server_id: String },
    /// Fetch the file of my avatar, missing on this device.
    Download { url: String, sha: String },
    /// Its server is no longer one I use: put it on those I use and name
    /// the new address in kind 0.
    Rehost { sha: String },
    /// Ask its server whether it still has it; put it back if not.
    Check { server_id: String, sha: String },
    /// Fetch it past the caches to keep it alive; put it back if gone.
    Touch { url: String, server_id: String, sha: String },
    /// No server of mine can take it now: fetch it past the caches all the
    /// same, so the server it is on keeps it meanwhile.
    KeepAlive { url: String, sha: String },
}

/// What the keeper does on a run with `facts`.
pub fn plan(facts: &Facts) -> Vec<Step> {
    let Some(picture) = &facts.picture else { return Vec::new() };
    let mut steps = Vec::new();
    // (url, sha, checked_at, touched_at) of the avatar to look after.
    let mut current = None;
    match &facts.own {
        Some(row) if picture.as_deref() == Some(row.url.as_str()) => {
            current = Some((row.url.clone(), row.sha256.clone(), row.checked_at, row.touched_at));
        }
        Some(row) => {
            let next = picture.as_deref().and_then(hash_of);
            steps.push(Step::Forget { sha: row.sha256.clone(), keep_file: next.as_deref() == Some(row.sha256.as_str()) });
        }
        None => {}
    }
    if current.is_none() {
        let Some(url) = picture else { return steps };
        // Only a picture on one of my servers is one I set: anything else
        // was named by another client and is left as it is.
        let Some(server_id) = server_of(url, &facts.bases) else { return steps };
        let Some(sha) = hash_of(url) else { return steps };
        steps.push(Step::Adopt { url: url.clone(), sha: sha.clone(), server_id: server_id.into() });
        current = Some((url.clone(), sha, facts.now, facts.now));
    }
    let (url, sha, checked_at, touched_at) = current.expect("set above");
    if !facts.own_files.contains(&sha) {
        steps.push(Step::Download { url: url.clone(), sha: sha.clone() });
    }
    match server_of(&url, &facts.bases) {
        // Still in use, only not now: the next run sees.
        None if server_of(&url, &facts.known).is_some() => {}
        None if facts.bases.is_empty() => {
            if facts.now - touched_at >= TOUCH_EVERY_SECS {
                steps.push(Step::KeepAlive { url, sha });
            }
        }
        None if facts.fresh => steps.push(Step::Rehost { sha }),
        None => {}
        Some(server_id) => {
            if facts.now - checked_at >= CHECK_EVERY_SECS {
                steps.push(Step::Check { server_id: server_id.into(), sha: sha.clone() });
            }
            if facts.now - touched_at >= TOUCH_EVERY_SECS {
                steps.push(Step::Touch { url, server_id: server_id.into(), sha });
            }
        }
    }
    steps
}

impl Avatars {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Store,
        media: MediaService,
        profiles: ProfileService,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
        clock: Arc<dyn Clock>,
        net: Arc<dyn AvatarNet>,
        dir: PathBuf,
    ) -> Self {
        Self {
            store,
            media,
            profiles,
            outbox,
            ui,
            clock,
            net,
            dir,
            prepared: Arc::default(),
            inflight: Arc::default(),
            slots: Arc::new(Semaphore::new(MAX_FETCHES)),
            kick: Arc::new(Notify::new()),
            own_lock: Arc::default(),
            heard_from: Arc::default(),
        }
    }

    /// A session starts: what my kind 0 said before it may be old, until
    /// a relay tells it again.
    pub fn session_started(&self) {
        self.heard_from.store(self.profiles.own_heard(), Ordering::SeqCst);
    }

    fn fresh(&self) -> bool {
        self.profiles.own_heard() > self.heard_from.load(Ordering::SeqCst)
    }

    fn now(&self) -> i64 {
        self.clock.now().secs()
    }

    fn own_path(&self, sha: &str) -> PathBuf {
        self.dir.join("own").join(format!("{sha}.jpg"))
    }

    fn cache_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.jpg"))
    }

    fn emit(&self, name: &str, payload: serde_json::Value) {
        let _ = self.ui.send(UiEvent { name: name.into(), payload });
    }

    fn profile_updated(&self, keys: &Keys) {
        self.emit(UI_EVENT_PROFILE_UPDATED, serde_json::json!({ "pubkey": keys.public_key().to_hex() }));
    }

    /// Wake the keeper: the media servers changed.
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    // ─── Mine ───────────────────────────────────────────────────────────────

    /// Decode a picked file and hold it for the crop.
    pub async fn prepare_file(&self, path: &Path) -> Result<AvatarPreview> {
        let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
        if !meta.is_file() {
            return Err(MessengerError::Io("err.file_not_found".into()));
        }
        if meta.len() > avatar::MAX_INPUT_BYTES as u64 {
            return Err(invalid(AvatarError::TooLarge));
        }
        self.prepare(tokio::fs::read(path).await?).await
    }

    /// Decode a picked picture and hold it for the crop: one at a time,
    /// for ten minutes at most.
    pub async fn prepare(&self, bytes: Vec<u8>) -> Result<AvatarPreview> {
        if bytes.len() > avatar::MAX_INPUT_BYTES {
            return Err(invalid(AvatarError::TooLarge));
        }
        // The picture the user picked before is let go first.
        self.prepared.lock().unwrap_or_else(|p| p.into_inner()).take();
        let (image, preview) = tokio::task::spawn_blocking(move || -> std::result::Result<_, AvatarError> {
            let image = avatar::decode(&bytes)?;
            let preview = avatar::preview_jpeg(&image, PREVIEW_SIDE);
            Ok((image, preview))
        })
        .await
        .map_err(joined)?
        .map_err(invalid)?;
        let token = random_hex(16);
        let view = AvatarPreview { token: token.clone(), preview: avatar::data_url(&preview), width: image.width(), height: image.height() };
        *self.prepared.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Prepared { token: token.clone(), image: Arc::new(image), at: Instant::now() });
        // Let go of the pixels when nobody crops them.
        let held = Arc::downgrade(&self.prepared);
        tokio::spawn(async move {
            tokio::time::sleep(PREPARED_TTL).await;
            if let Some(held) = held.upgrade() {
                let mut slot = held.lock().unwrap_or_else(|p| p.into_inner());
                if slot.as_ref().is_some_and(|p| p.token == token) {
                    slot.take();
                }
            }
        });
        Ok(view)
    }

    fn prepared(&self, token: &str) -> Result<Arc<Decoded>> {
        let slot = self.prepared.lock().unwrap_or_else(|p| p.into_inner());
        match slot.as_ref() {
            Some(p) if p.token == token && p.at.elapsed() < PREPARED_TTL => Ok(p.image.clone()),
            _ => Err(MessengerError::Invalid("avatar_expired".into())),
        }
    }

    /// Crop the prepared picture, put it on my servers and name it in my
    /// kind 0. The picture it replaces is taken off its servers.
    pub async fn set(&self, keys: &Keys, token: &str, rect: CropRect) -> Result<()> {
        let image = self.prepared(token)?;
        let jpeg = tokio::task::spawn_blocking(move || avatar::crop_square(&image, rect, avatar::OWN_SIDE))
            .await
            .map_err(joined)?
            .map_err(invalid)?;
        let sha = avatar::sha256_hex(&jpeg);
        let _one = self.own_lock.lock().await;
        let old = own_avatar::get(&self.store).await?;
        let path = self.own_path(&sha);
        write_whole(&path, &jpeg).await?;
        let blobs = match self.media.upload_public(keys, jpeg, JPEG, OWN_COPIES).await {
            Ok(b) => b,
            Err(e) => {
                if old.as_ref().is_none_or(|o| o.sha256 != sha) {
                    let _ = tokio::fs::remove_file(&path).await;
                }
                return Err(e);
            }
        };
        let now = self.now();
        let first = blobs[0].clone();
        own_avatar::set(
            &self.store,
            &own_avatar::OwnAvatar {
                sha256: sha.clone(),
                url: first.url.clone(),
                server_id: Some(first.server_id.clone()),
                copies_json: serde_json::to_string(&blobs)?,
                set_at: now,
                checked_at: now,
                touched_at: now,
            },
        )
        .await?;
        let event = self.profiles.build_picture(keys, Picture::Set(&first.url)).await?;
        self.outbox.enqueue(Outbound::PublishOwn { event }).await?;
        self.outbox.kick();
        {
            let mut slot = self.prepared.lock().unwrap_or_else(|p| p.into_inner());
            if slot.as_ref().is_some_and(|p| p.token == token) {
                slot.take();
            }
        }
        if let Some(old) = old.filter(|o| o.sha256 != sha) {
            let _ = tokio::fs::remove_file(self.own_path(&old.sha256)).await;
            self.take_down(keys, &old.sha256, copies_of(&old));
        }
        self.profile_updated(keys);
        Ok(())
    }

    /// No avatar: kind 0 without `picture`, and the picture off its servers.
    pub async fn remove(&self, keys: &Keys) -> Result<()> {
        let _one = self.own_lock.lock().await;
        let old = own_avatar::get(&self.store).await?;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        let named = self.profiles.get(&me).await?.and_then(|p| p.picture);
        let event = self.profiles.build_picture(keys, Picture::Remove).await?;
        self.outbox.enqueue(Outbound::PublishOwn { event }).await?;
        self.outbox.kick();
        own_avatar::clear(&self.store).await?;
        match old {
            Some(old) => {
                let _ = tokio::fs::remove_file(self.own_path(&old.sha256)).await;
                self.take_down(keys, &old.sha256, copies_of(&old));
            }
            // Set on another device of mine and not yet taken on here.
            None => {
                if let Some(url) = named {
                    let bases = self.media.enabled_public_bases(keys).await.unwrap_or_default();
                    if let (Some(server_id), Some(sha)) = (server_of(&url, &bases), hash_of(&url)) {
                        let _ = tokio::fs::remove_file(self.own_path(&sha)).await;
                        self.take_down(keys, &sha, vec![PublicBlob { server_id: server_id.into(), url: url.clone() }]);
                    }
                }
            }
        }
        self.profile_updated(keys);
        Ok(())
    }

    /// Delete the blob from where it was put, in the background; a server
    /// that cannot be reached keeps it.
    fn take_down(&self, keys: &Keys, sha: &str, copies: Vec<PublicBlob>) {
        let (media, keys, sha) = (self.media.clone(), keys.clone(), sha.to_string());
        tokio::spawn(async move {
            for copy in copies {
                if let Err(e) = media.public_delete(&keys, &copy.server_id, &sha).await {
                    eprintln!("messenger avatar: the old picture stays on {}: {e}", copy.server_id);
                }
            }
        });
    }

    // ─── The keeper ─────────────────────────────────────────────────────────

    async fn own_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(mut dir) = tokio::fs::read_dir(self.dir.join("own")).await else { return out };
        while let Ok(Some(entry)) = dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(sha) = name.strip_suffix(".jpg").filter(|s| is_sha(s)) {
                out.push(sha.to_string());
            }
        }
        out
    }

    /// One run of the keeper; see `plan`. A step that fails stops the run
    /// when the next ones need it (the file), and is otherwise tried again
    /// on the next run.
    pub async fn keep(&self, keys: &Keys) -> Result<Vec<Step>> {
        let _one = self.own_lock.lock().await;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        // Read before the profile: a kind 0 that comes after it counts on
        // the next run only.
        let fresh = self.fresh();
        let profile = self.profiles.get(&me).await?;
        let facts = Facts {
            own: own_avatar::get(&self.store).await?,
            picture: profile.as_ref().map(|p| p.picture.clone()),
            bases: self.media.enabled_public_bases(keys).await?,
            known: self.media.enabled_server_bases().await?,
            own_files: self.own_files().await,
            fresh,
            now: self.now(),
        };
        let steps = plan(&facts);
        for step in &steps {
            self.run_step(keys, step, &facts, profile.as_ref()).await?;
        }
        Ok(steps)
    }

    /// Whether my kind 0 still names what it did when `facts` were taken:
    /// one that came meanwhile from another device of mine wins over what
    /// the run was about to do.
    async fn still_named(&self, keys: &Keys, facts: &Facts) -> Result<bool> {
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        Ok(self.profiles.get(&me).await?.map(|p| p.picture) == facts.picture)
    }

    async fn run_step(&self, keys: &Keys, step: &Step, facts: &Facts, profile: Option<&ProfileView>) -> Result<()> {
        let now = facts.now;
        match step {
            Step::Forget { sha, keep_file } => {
                own_avatar::clear(&self.store).await?;
                if !keep_file {
                    let _ = tokio::fs::remove_file(self.own_path(sha)).await;
                    // Put back here after another device of mine took it
                    // down: down again.
                    if settings::get(&self.store, KEY_PUT_BACK).await?.as_deref() == Some(sha.as_str()) {
                        if let Some(row) = facts.own.as_ref().filter(|r| r.sha256 == *sha) {
                            self.take_down(keys, sha, copies_of(row));
                        }
                        settings::delete(&self.store, KEY_PUT_BACK).await?;
                    }
                }
            }
            Step::Adopt { url, sha, server_id } => {
                let copy = PublicBlob { server_id: server_id.clone(), url: url.clone() };
                own_avatar::set(
                    &self.store,
                    &own_avatar::OwnAvatar {
                        sha256: sha.clone(),
                        url: url.clone(),
                        server_id: Some(server_id.clone()),
                        copies_json: serde_json::to_string(&[copy])?,
                        set_at: profile.map_or(now, |p| p.event_created_at),
                        checked_at: now,
                        touched_at: now,
                    },
                )
                .await?;
            }
            Step::Download { url, sha } => {
                let bytes = self.fetch_blob(url, sha, &facts.bases).await?;
                if !is_own_encode(&bytes).await {
                    // Not a picture this app made: another client named it,
                    // and it is not taken on (nor handed to the page).
                    if own_avatar::get(&self.store).await?.is_some_and(|r| r.sha256 == *sha) {
                        own_avatar::clear(&self.store).await?;
                    }
                    return Err(MessengerError::Invalid("avatar_not_ours".into()));
                }
                write_whole(&self.own_path(sha), &bytes).await?;
                self.emit(UI_EVENT_AVATAR_READY, serde_json::json!({ "url": url }));
            }
            Step::Rehost { sha } => {
                if !facts.fresh || !self.still_named(keys, facts).await? {
                    return Ok(());
                }
                let Some(mut row) = own_avatar::get(&self.store).await?.filter(|r| r.sha256 == *sha) else { return Ok(()) };
                let old = copies_of(&row);
                let bytes = tokio::fs::read(self.own_path(sha)).await?;
                let blobs = self.media.upload_public(keys, bytes, JPEG, OWN_COPIES).await?;
                if !self.still_named(keys, facts).await? {
                    return Ok(());
                }
                // Kind 0 first: the row names the new address only once it
                // is on its way, so a run that stops here is done again.
                let event = self.profiles.build_picture(keys, Picture::Set(&blobs[0].url)).await?;
                self.outbox.enqueue(Outbound::PublishOwn { event }).await?;
                self.outbox.kick();
                row.url = blobs[0].url.clone();
                row.server_id = Some(blobs[0].server_id.clone());
                row.copies_json = serde_json::to_string(&blobs)?;
                row.checked_at = now;
                row.touched_at = now;
                own_avatar::set(&self.store, &row).await?;
                // Off the servers it moved from.
                let left: Vec<PublicBlob> = old.into_iter().filter(|c| blobs.iter().all(|b| b.server_id != c.server_id)).collect();
                if !left.is_empty() {
                    self.take_down(keys, sha, left);
                }
                self.profile_updated(keys);
            }
            Step::Check { server_id, sha } => match self.media.public_exists(keys, server_id, sha).await {
                Ok(true) => own_avatar::mark_checked(&self.store, now).await?,
                Ok(false) => {
                    if self.put_back(keys, server_id, sha, facts).await? {
                        own_avatar::mark_checked(&self.store, now).await?;
                    }
                }
                Err(e) => eprintln!("messenger avatar: {server_id} not asked: {e}"),
            },
            Step::Touch { url, server_id, sha } => match self.net.own(&unique(url), MAX_FETCH_BYTES, true).await {
                Ok(Some(body)) if avatar::sha256_hex(&body) == *sha => own_avatar::mark_touched(&self.store, now).await?,
                Ok(_) => {
                    if self.put_back(keys, server_id, sha, facts).await? {
                        own_avatar::mark_touched(&self.store, now).await?;
                    }
                }
                Err(e) => eprintln!("messenger avatar: {url} not fetched: {e}"),
            },
            Step::KeepAlive { url, sha } => match self.net.own(&unique(url), MAX_FETCH_BYTES, true).await {
                Ok(Some(body)) if avatar::sha256_hex(&body) == *sha => own_avatar::mark_touched(&self.store, now).await?,
                Ok(_) => eprintln!("messenger avatar: {url} is gone and no server of mine can take it"),
                Err(e) => eprintln!("messenger avatar: {url} not fetched: {e}"),
            },
        }
        Ok(())
    }

    /// Put my avatar back on a server that lost it, only when my kind 0 is
    /// known to be current and still names it: a picture removed on
    /// another device of mine is not brought back. `true` when it was put.
    async fn put_back(&self, keys: &Keys, server_id: &str, sha: &str, facts: &Facts) -> Result<bool> {
        if !facts.fresh || !self.still_named(keys, facts).await? {
            return Ok(false);
        }
        let bytes = tokio::fs::read(self.own_path(sha)).await?;
        self.media.put_public(keys, server_id, bytes, JPEG).await?;
        settings::set(&self.store, KEY_PUT_BACK, sha).await?;
        Ok(true)
    }

    /// The bytes of `sha`: from `url`, else from any of my servers. Only
    /// bytes of that hash are taken.
    async fn fetch_blob(&self, url: &str, sha: &str, bases: &[(String, String)]) -> Result<Vec<u8>> {
        let mut tried = Vec::new();
        let first = if under_my_server(url, bases) {
            self.net.own(url, MAX_FETCH_BYTES, false).await
        } else {
            self.net.foreign(url, MAX_FETCH_BYTES).await.map(Some)
        };
        match first {
            Ok(Some(b)) if avatar::sha256_hex(&b) == sha => return Ok(b),
            Ok(Some(_)) => tried.push(format!("{url}: not the picture")),
            Ok(None) => tried.push(format!("{url}: not there")),
            Err(e) => tried.push(format!("{url}: {e}")),
        }
        for (_, base) in bases {
            let other = messenger_media::MediaDescriptor::chunk_url(base, sha);
            if other == url {
                continue;
            }
            match self.net.own(&other, MAX_FETCH_BYTES, false).await {
                Ok(Some(b)) if avatar::sha256_hex(&b) == sha => return Ok(b),
                Ok(_) => {}
                Err(e) => tried.push(format!("{other}: {e}")),
            }
        }
        Err(MessengerError::Transport(format!("avatar_unavailable: {}", tried.join("; "))))
    }

    // ─── Others ─────────────────────────────────────────────────────────────

    /// The picture of `url` as a `data:` url when it is here. Otherwise
    /// `None`, and unless `may_fetch` is false a fetch is queued;
    /// `avatar.ready {url}` follows when it is done. `keys` (the session)
    /// lets my own servers be recognised and asked.
    pub async fn cached(&self, url: &str, keys: Option<&Keys>, may_fetch: bool) -> Result<Option<String>> {
        let url = url.trim();
        if !acceptable(url) {
            return Ok(None);
        }
        // My own avatar, wherever it is named: the file, no network.
        if let Some(sha) = hash_of(url) {
            if let Ok(bytes) = tokio::fs::read(self.own_path(&sha)).await {
                return Ok(Some(avatar::data_url(&bytes)));
            }
        }
        let row = avatar_cache::get(&self.store, url).await?;
        let now = self.now();
        let mut found = None;
        if let Some(name) = row.as_ref().and_then(|r| r.sha256.as_deref()).filter(|n| is_sha(n)) {
            if let Ok(bytes) = tokio::fs::read(self.cache_path(name)).await {
                found = Some(avatar::data_url(&bytes));
            }
        }
        if let Some(r) = row.as_ref().filter(|r| found.is_some() && now - r.used_at >= USED_EVERY_SECS) {
            avatar_cache::mark_used(&self.store, &r.url, now).await?;
        }
        let stale = match (&found, &row) {
            (None, _) => true,
            (Some(_), Some(r)) => hash_of(url).is_none() && now - r.fetched_at >= REFRESH_SECS,
            (Some(_), None) => false,
        };
        if stale && may_fetch && may_try(row.as_ref(), now) {
            self.queue(url, keys.cloned());
        }
        Ok(found)
    }

    /// The picture of `url` as this device holds it, never from the
    /// network: my own file when the address names it, else the cache of
    /// others. For a contact card.
    pub async fn local_bytes(&self, url: &str) -> Option<Vec<u8>> {
        let url = url.trim();
        if let Some(sha) = hash_of(url) {
            if let Ok(bytes) = tokio::fs::read(self.own_path(&sha)).await {
                return Some(bytes);
            }
        }
        let row = avatar_cache::get(&self.store, url).await.ok()??;
        let name = row.sha256.filter(|n| is_sha(n))?;
        tokio::fs::read(self.cache_path(&name)).await.ok()
    }

    fn queue(&self, url: &str, keys: Option<Keys>) {
        if !self.inflight.lock().unwrap_or_else(|p| p.into_inner()).insert(url.to_string()) {
            return;
        }
        let (this, url) = (self.clone(), url.to_string());
        tokio::spawn(async move {
            let ok = match this.slots.clone().acquire_owned().await {
                Ok(_slot) => this.fetch_one(&url, keys.as_ref()).await,
                Err(_) => Err(MessengerError::Io("closed".into())),
            };
            // The failure is written before the address is free again, so a
            // request in between does not try it once more.
            if let Err(e) = &ok {
                let _ = avatar_cache::put_failed(&this.store, &url, this.now()).await;
                eprintln!("messenger avatar: {url}: {e}");
            }
            this.inflight.lock().unwrap_or_else(|p| p.into_inner()).remove(&url);
            if ok.is_ok() {
                this.emit(UI_EVENT_AVATAR_READY, serde_json::json!({ "url": url }));
            }
        });
    }

    /// Forget the pictures of others nobody looked at for
    /// `CACHE_KEEP_SECS`, and those beyond the `MAX_CACHED` shown most
    /// lately, files and all. Returns how many files went.
    pub async fn sweep(&self) -> Result<usize> {
        let free = avatar_cache::sweep(&self.store, self.now() - CACHE_KEEP_SECS, MAX_CACHED).await?;
        for sha in free.iter().filter(|s| is_sha(s)) {
            let _ = tokio::fs::remove_file(self.cache_path(sha)).await;
        }
        Ok(free.len())
    }

    /// Fetch the picture of `url`, make it a thumbnail and keep it.
    pub async fn fetch_one(&self, url: &str, keys: Option<&Keys>) -> Result<()> {
        let bases = match keys {
            Some(k) => self.media.enabled_public_bases(k).await.unwrap_or_default(),
            None => Vec::new(),
        };
        let hash = hash_of(url);
        let body = match &hash {
            Some(sha) => self.fetch_blob(url, sha, &bases).await?,
            None if under_my_server(url, &bases) => self
                .net
                .own(url, MAX_FETCH_BYTES, false)
                .await?
                .ok_or_else(|| MessengerError::Transport("avatar_unavailable".into()))?,
            None => self.net.foreign(url, MAX_FETCH_BYTES).await?,
        };
        let thumb = tokio::task::spawn_blocking(move || avatar::square_thumb(&body, avatar::CACHE_SIDE))
            .await
            .map_err(joined)?
            .map_err(invalid)?;
        let name = hash.unwrap_or_else(|| avatar::sha256_hex(url.as_bytes()));
        write_whole(&self.cache_path(&name), &thumb).await?;
        avatar_cache::put_fetched(&self.store, url, &name, self.now()).await?;
        Ok(())
    }
}

/// Where the avatar was put, as the row says; the server it is named on
/// when the list cannot be read.
fn copies_of(row: &own_avatar::OwnAvatar) -> Vec<PublicBlob> {
    match serde_json::from_str::<Vec<PublicBlob>>(&row.copies_json) {
        Ok(list) if !list.is_empty() => list,
        _ => row.server_id.iter().map(|s| PublicBlob { server_id: s.clone(), url: row.url.clone() }).collect(),
    }
}

/// The keeper as the session runs it: at the start, once a day, when it is
/// kicked and when my kind 0 changes; never while `silent` says the app
/// keeps off the network.
pub(crate) async fn keeper_loop(avatars: Avatars, keys: Keys, silent: impl Fn() -> bool + Send + 'static) {
    let me = keys.public_key().to_hex();
    let mut events = avatars.ui.subscribe();
    let mut wait = KEEPER_FIRST_RUN;
    loop {
        let timer = tokio::time::sleep(wait);
        tokio::pin!(timer);
        loop {
            tokio::select! {
                _ = &mut timer => break,
                _ = avatars.kick.notified() => {
                    tokio::time::sleep(KEEPER_SETTLE).await;
                    break;
                }
                ev = events.recv() => match ev {
                    Ok(ev) if ev.name == UI_EVENT_PROFILE_UPDATED && ev.payload["pubkey"].as_str() == Some(me.as_str()) => {
                        tokio::time::sleep(KEEPER_SETTLE).await;
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                    _ => {}
                },
            }
        }
        if let Err(e) = avatars.sweep().await {
            eprintln!("messenger avatar: cache: {e}");
        }
        if silent() {
            wait = KEEPER_EVERY;
            continue;
        }
        if let Err(e) = avatars.keep(&keys).await {
            eprintln!("messenger avatar: keeper: {e}");
        }
        // Events of the run itself are no reason for another.
        events = events.resubscribe();
        wait = KEEPER_EVERY;
    }
}

// ─── The runtime ────────────────────────────────────────────────────────────

impl MessengerRuntime {
    pub fn avatars(&self) -> &Avatars {
        &self.avatars
    }

    /// A picked file (a path this device may read) as a picture to crop.
    pub async fn avatar_prepare(&self, path: &Path) -> Result<AvatarPreview> {
        self.avatars.prepare_file(path).await
    }

    /// The bytes of a picked picture as a picture to crop.
    pub async fn avatar_prepare_bytes(&self, bytes: Vec<u8>) -> Result<AvatarPreview> {
        self.avatars.prepare(bytes).await
    }

    /// Make the part `rect` of the prepared picture my avatar. Errors:
    /// `avatar_expired`, `avatar_bad_crop`, `err.media_no_server` and those
    /// of the servers.
    pub async fn avatar_set(&self, token: &str, rect: CropRect) -> Result<ProfileView> {
        let keys = self.session_keys().await?;
        self.avatars.set(&keys, token, rect).await?;
        self.own_profile_now(&keys).await
    }

    pub async fn avatar_remove(&self) -> Result<ProfileView> {
        let keys = self.session_keys().await?;
        self.avatars.remove(&keys).await?;
        self.own_profile_now(&keys).await
    }

    /// The avatar at `url` as a `data:` url, or `None` while it is fetched
    /// (`avatar.ready` follows). Nothing is fetched in silent mode.
    pub async fn avatar_cached(&self, url: &str) -> Result<Option<String>> {
        let keys = self.session_keys().await.ok();
        let may_fetch = !self.relays.is_silent().await.unwrap_or(false);
        self.avatars.cached(url, keys.as_ref(), may_fetch).await
    }

    async fn own_profile_now(&self, keys: &Keys) -> Result<ProfileView> {
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        self.profiles.get(&me).await?.ok_or_else(|| MessengerError::Storage("own profile missing after publish".into()))
    }

    /// The spans of a bio as it will show: the live preview of the editor.
    pub fn bio_parse(&self, markup: &str) -> Vec<Span> {
        messenger_richtext::parse(markup)
    }

    /// The platforms a link to a profile elsewhere may be on.
    pub fn social_platforms(&self) -> Vec<SocialPlatform> {
        social::platforms()
    }
}

#[cfg(test)]
mod tests;

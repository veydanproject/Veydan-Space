// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/media.rs at 1f2ef31d, then changed for channels.

//! Attachments of posts and comments, and the cover of a channel
//! (internal/communities-spec.md §10.3, internal/communities-wire.md §9).
//! As in a group, while a file is uploading the channel shows a
//! placeholder row, and once the blob is stored the placeholder gives way
//! to the real message; the file key travels inside the message, under
//! the key of the epoch.
//!
//! A file goes up and comes down through the messenger's engine
//! (messenger-media `upload`, `download`), the one of chats and groups:
//! several chunks at once, the retries of a chunk, the resume from what
//! the hosts hold (its state kept with the row after every chunk), the
//! automatic attempts of the file, a pause and a cancel heard at once,
//! and the progress with its stage, speed and time left (the messenger's
//! `transfer.progress`, inside `channel.media`). The messenger's media
//! service is not used: it uploads to the media servers of the user's
//! account and keeps its transfers in `messenger.db`. The chunks of a
//! channel go through the channel's own seam (`BlobHosts`, the keeper's
//! since 1.2) to every host of `storage[]` (`media_hosts`), in the
//! messenger's formats, so that a page shows and opens them as it does
//! those of chats. One host (`MIN_HOSTS`) must hold every chunk before
//! the message goes out, as for an epoch.
//!
//! Who may put blobs on the hosts (§9): the keeper's key and the roles
//! from author up; a reader comments in words, and with a voice message
//! or a video circle recorded in the app (2026-10-23).

use crate::blobs::{fetch_verified_up_to, sha256_hex};
use crate::db::{self, avatar::ChannelAvatar, content::UploadRow};
use crate::keeper::MIN_HOSTS;
use crate::media_hosts::{hosts_of, HostsBackend, HostsFetcher};
use crate::op::OpBody;
use crate::roles::Role;
use crate::service::{ChannelService, Outcome};
use crate::state::{ChannelState, Content, MAX_PICTURE_CHARS, SCOPE_EVERYONE};
use crate::wire::{self, Links};
use messenger_core::{MessengerError, PubKey, Result, TextFormat};
use messenger_media::control::{Control, TransferCtx, CANCEL, PAUSE, WORKERS};
use messenger_media::descriptor::{mime_for, safe_name, valid_batch, valid_thumb, MediaDescriptor, MediaKind, MAX_SEND_BYTES};
use messenger_media::download::{self, DownloadOutcome};
use messenger_media::prepare::{of_poster, picture_size, smaller_photo, thumb_of_file};
pub use messenger_media::prepare::Poster;
use messenger_media::progress::{Live, ProgressMeter, TransferStage};
use messenger_media::service::{retryable, short_reason};
use messenger_media::upload::{self, UploadOutcome, UploadParams, UploadState};
pub use messenger_media::progress::{Progress, ProgressSink};
use nostr::key::Keys;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A file of mine started, went on, was stored or failed, or a file of a
/// message came down: `{community_id, id, status, done, total, error?,
/// progress?}`, `progress` the messenger's `transfer.progress` of it.
pub const UI_EVENT_CHANNEL_MEDIA: &str = "channel.media";
/// The largest cover (wire §9).
pub const MAX_COVER_BYTES: usize = 1024 * 1024;
/// The largest picture a cover is made from: what the messenger takes for
/// the picture of a group (`messenger_avatar::MAX_INPUT_BYTES`).
pub const MAX_COVER_INPUT_BYTES: usize = messenger_avatar::MAX_INPUT_BYTES;
/// The side of the square a picked picture is shown as in the tray above
/// the composer, before it is sent (`picked_preview`).
pub const PICKED_PREVIEW_SIDE: u32 = 256;
/// What a refusal of a blob host starts with (`channel_upload_refused`):
/// the host and its answer follow, for the page to say.
pub const UPLOAD_REFUSED: &str = "channel: upload refused";

/// The error of an upload no host took enough of: the answer of the last
/// host that answered, when one refused it (its key, its plan, its rules:
/// sending again changes nothing, the page says what it said), else a
/// network error (worth another try).
/// A cover on the hosts, not set yet (`ChannelService::upload_cover`).
#[derive(Clone, Debug)]
pub struct NewCover {
    /// Of the JPEG, lowercase hex.
    pub sha: String,
    /// On the first host of `storage` that took it.
    pub url: String,
    /// The hosts that took it, in the order of `storage`.
    pub copies: Vec<String>,
    /// The JPEG itself: the page shows it from here before any host is asked.
    pub jpeg: Vec<u8>,
}

/// The photo of a section on the hosts, not named by a meta yet
/// (`ChannelService::upload_scope_photo`).
#[derive(Clone, Debug)]
pub struct NewScopePhoto {
    /// What its meta names: the blob and the key it is sealed under.
    pub photo: wire::ScopePhoto,
    /// The hosts that took it, in the order of `storage`.
    pub copies: Vec<String>,
    /// The JPEG itself: the page shows it from here before any host is asked.
    pub jpeg: Vec<u8>,
}

/// The largest blob of the photo of a section: a cover sealed
/// (`wire::seal_bytes`, base64 of nonce, ciphertext and tag).
pub const MAX_SCOPE_PHOTO_BLOB: usize = MAX_COVER_BYTES / 3 * 4 + 64;

/// The photo of a section, width by height: the 3:2 of its tile, cut from
/// the middle of the picture (the owner's word of 2026-10-25, «да, делаем»).
pub const SCOPE_PHOTO_SIZE: (u32, u32) = (960, 640);

/// A file I picked as the tray above the composer shows it before it is
/// sent: a picture as a square JPEG of `PICKED_PREVIEW_SIDE` of its
/// center, made from the pixels alone, as a `data:` URL. `None` for a
/// file that is not a picture (told by its first bytes, never by a name:
/// a phone's URI may have none), one larger than a cover is made from
/// (`MAX_COVER_INPUT_BYTES`, of which no more is read), or one that does
/// not decode. It reads and decodes: called off the runtime's threads.
pub fn picked_preview(file: impl std::io::Read) -> std::io::Result<Option<String>> {
    use std::io::Read as _;
    let mut file = file.take(MAX_COVER_INPUT_BYTES as u64 + 1);
    let mut bytes = Vec::new();
    (&mut file).take(64).read_to_end(&mut bytes)?;
    if messenger_avatar::picture_ext(&bytes).is_none() {
        return Ok(None);
    }
    file.read_to_end(&mut bytes)?;
    if bytes.len() > MAX_COVER_INPUT_BYTES {
        return Ok(None);
    }
    let Ok(img) = messenger_avatar::decode(&bytes) else { return Ok(None) };
    let whole = messenger_avatar::CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    Ok(messenger_avatar::crop_square(&img, whole, PICKED_PREVIEW_SIDE).ok().map(|jpeg| messenger_avatar::data_url(&jpeg)))
}

fn not_taken(last: Option<(String, MessengerError)>, what: &str) -> MessengerError {
    match last {
        Some((host, MessengerError::Transport(m)))
            if !(m.starts_with("err.network") || m.starts_with("err.timeout") || m.starts_with("err.server") || m.starts_with("err.rate_limited")) =>
        {
            MessengerError::Invalid(format!("{UPLOAD_REFUSED}: {host}: {m}"))
        }
        _ => MessengerError::Transport(format!("err.network: the hosts of the channel did not take the {what}")),
    }
}

/// A file no host took for good: what the last host that refused it said
/// (its key, its plan, its rules: sending again changes nothing), as
/// `not_taken` says it of a cover; any other failure as the engine told it.
fn refusal_of(backend: &HostsBackend, e: MessengerError) -> MessengerError {
    match backend.refused() {
        Some((host, refused)) if !retryable(&e) => MessengerError::Invalid(format!("{UPLOAD_REFUSED}: {host}: {}: {}", refused.code(), refused.message)),
        _ => e,
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The state an upload kept with its row, when it has one.
fn parse_state(json: &str) -> Option<UploadState> {
    serde_json::from_str::<UploadState>(json).ok().filter(|s| !s.key.is_empty())
}

/// Waits before automatic attempt `attempt` of a file, `delay` long, told
/// as the messenger tells it: "retry now" (`Control::wake`) ends the wait,
/// a pause or a cancel ends it and is returned.
async fn wait_retry(control: &Control, meter: &ProgressMeter<'_>, attempt: usize, delay: Duration, e: &MessengerError) -> Option<u8> {
    meter.retry(u32::try_from(attempt).unwrap_or(u32::MAX), Some(now_ms() + i64::try_from(delay.as_millis()).unwrap_or(i64::MAX)));
    meter.status("waiting_retry", Some(&short_reason(e)));
    let mut wakes = control.wakes();
    tokio::select! {
        _ = tokio::time::sleep(delay) => None,
        _ = wakes.changed() => None,
        stop = control.stopped() => Some(stop),
    }
}

/// The error of an upload given up while it ran (`media_cancel`).
pub const UPLOAD_CANCELLED: &str = "channel_upload_cancelled";
/// The error of an upload paused while it ran (`media_pause`): its row
/// waits, `media_retry` goes on from the chunks the hosts hold.
pub const UPLOAD_PAUSED: &str = "channel_upload_paused";
/// The error of a download paused or given up (`media_pause`,
/// `media_fetch_cancel`).
pub const FETCH_STOPPED: &str = "channel_fetch_stopped";
/// The largest file a page is handed whole as a `data:` URL: the
/// messenger's bound (messenger-runtime `MAX_INLINE_BYTES`), so a file
/// shows inline in a channel exactly when it would in a chat.
pub const MAX_INLINE_BYTES: u64 = 24 * 1024 * 1024;

/// A file picked to go into a channel (`media_placeholder_file`).
pub struct NewFile<'a> {
    pub path: &'a Path,
    /// The name it goes under; none: the name of `path` (a copy the app
    /// made of a phone's file has a name of its own).
    pub name: Option<&'a str>,
    pub caption: Option<&'a str>,
    /// The section of a post; a comment goes into its post's.
    pub scope_id: Option<&'a str>,
    /// The post a comment goes under, and the comment it answers.
    pub post_id: Option<&'a str>,
    pub parent: Option<&'a str>,
    /// The files sent by one action (an album).
    pub batch: Option<&'a str>,
    /// A photo goes as it is, not made smaller.
    pub original: bool,
    /// The frame of a video the page took.
    pub poster: Option<Poster>,
    /// How the caption is written (Markdown or plain), as a chat's is:
    /// `fmt` on the envelope of the file.
    pub fmt: TextFormat,
    /// A voice message or a video circle recorded in the app: its kind,
    /// length and outline go with it (`media_placeholder_recording`).
    pub recorded: Option<Recorded>,
}

impl<'a> NewFile<'a> {
    /// The file at `path` alone, as it is picked.
    pub fn at(path: &'a Path) -> Self {
        Self { path, name: None, caption: None, scope_id: None, post_id: None, parent: None, batch: None, original: false, poster: None, fmt: TextFormat::Plain, recorded: None }
    }
}

/// What a recording carries besides its bytes, as a chat's does
/// (messenger-runtime `dm_send_recording`): its kind (`voice`, `circle`),
/// its length, and the outline of a voice message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub kind: MediaKind,
    pub duration_ms: Option<u64>,
    pub waveform: Option<Vec<u8>>,
}

/// The largest recording, as a chat takes it (messenger-runtime
/// `MAX_RECORDING_BYTES`).
pub const MAX_RECORDING_BYTES: u64 = 64 * 1024 * 1024;

/// A voice message or a video circle as the composer of the page hands it
/// over (`channel_media_record`), the bytes as base64, as a chat's
/// (`messenger_dm_send_recording`): mobile IPC has no raw bodies.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct NewRecording {
    /// `voice` | `circle`
    pub kind: String,
    /// As the recorder reported it; codec parameters are dropped.
    pub mime: String,
    pub duration_ms: Option<u64>,
    pub waveform: Option<Vec<u8>>,
    pub data_base64: String,
}

/// Where a recording goes: a post into a section (`scope_id`), or a
/// comment under `post_id` (`parent`: the comment it answers).
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordingPlace<'a> {
    pub scope_id: Option<&'a str>,
    pub post_id: Option<&'a str>,
    pub parent: Option<&'a str>,
}

/// The kind of a recording and the extension its file goes with, from
/// what the recorder reported; `None` for anything a chat would refuse
/// (messenger-runtime `dm_send_recording`).
fn recording_file(kind: &str, mime: &str) -> Option<(MediaKind, &'static str)> {
    let base = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match (MediaKind::parse(kind)?, base.as_str()) {
        (MediaKind::Voice, "audio/webm") => Some((MediaKind::Voice, "weba")),
        (MediaKind::Voice, "audio/ogg") => Some((MediaKind::Voice, "ogg")),
        (MediaKind::Voice, "audio/mp4") => Some((MediaKind::Voice, "m4a")),
        (MediaKind::Circle, "video/webm") => Some((MediaKind::Circle, "webm")),
        (MediaKind::Circle, "video/mp4") => Some((MediaKind::Circle, "mp4")),
        _ => None,
    }
}

/// May `who` put the file of `file` on the hosts: an author and above any
/// file; anyone not banned the recording of a comment (whether they may
/// write that comment is the rule of comments, checked on its own).
fn may_upload_file(state: &ChannelState, who: &PubKey, comment: bool, recording: bool) -> bool {
    may_upload(state, who) || (comment && recording && !state.is_banned(who))
}

/// What a file of mine carries besides its chunks, kept with its row
/// (`cm_uploads.meta_json`) until the message goes: the descriptor takes
/// it then, as a chat's placeholder gives it to its message.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UploadMeta {
    /// The name it goes under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A photo goes as it is.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub original: bool,
    /// How large a picture or a video is shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<(u32, u32)>,
    /// The preview of a picture or of the frame of a video (base64 JPEG).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// The caption is written in Markdown (`fmt: md` on the envelope).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub md: bool,
    /// A recording: `voice` or `circle` (none: the kind of its type).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The length of a recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The outline of a voice message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveform: Option<Vec<u8>>,
}

impl UploadMeta {
    /// The kind a recording goes as (`voice`, `circle`); `None` for a file
    /// picked, which goes as the kind of its type.
    pub fn recorded_kind(json: &str) -> Option<String> {
        Self::parse(json).kind.filter(|k| matches!(MediaKind::parse(k), Some(MediaKind::Voice | MediaKind::Circle)))
    }

    fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// How long a file stored once goes again as its chunks (`reused`): the
/// free hosts keep a file 14 days from its upload, and a new message of it
/// keeps a week at least.
pub const REUSE_SECS: i64 = 7 * 24 * 3600;

/// The hex SHA-256 of the file at `path`, read a piece at a time.
fn file_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

/// The name of the file at `path`.
fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A progress sink of a closure: `Sink(|p: Progress| …)`.
pub struct Sink<F>(pub F);

impl<F: Fn(Progress) + Send + Sync> ProgressSink for Sink<F> {
    fn progress(&self, p: Progress) {
        (self.0)(p)
    }
}

/// How a run of a file ended.
enum Ending<T> {
    Done(T),
    /// `PAUSE` or `CANCEL` of its control.
    Stopped(u8),
    Failed(MessengerError),
}

/// The run of a file is known by its id while it lives (`transfers`).
struct Registered<'a> {
    map: &'a Mutex<std::collections::HashMap<String, Control>>,
    id: String,
    control: Control,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(&self.id).is_some_and(|c| c.same(&self.control)) {
            map.remove(&self.id);
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// What does not change in the events of a file, and where it starts.
fn base_progress(id: &str, community_id: &str, direction: &str, name: &str, done: u64, total: u64) -> Progress {
    Progress {
        transfer_id: id.to_string(),
        message_id: Some(id.to_string()),
        chat_id: Some(community_id.to_string()),
        direction: direction.into(),
        status: "queued".into(),
        done_bytes: done,
        total_bytes: total,
        failure_reason: None,
        local_path: None,
        stage: TransferStage::Queued,
        chunks_done: 0,
        chunks_total: 0,
        chunk_size: 0,
        rate_bps: 0,
        eta_secs: None,
        retry_at_ms: None,
        attempt: 0,
        file_name: name.to_string(),
        mime: mime_for(name).to_string(),
    }
}

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel media: {what}"))
}

fn placeholder_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("local:c{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

/// May `who` put blobs on the hosts of the channel (wire §9)?
pub fn may_upload(state: &ChannelState, who: &PubKey) -> bool {
    !state.is_banned(who) && state.role_of(who).is_some_and(|r| r >= Role::Author)
}

impl ChannelService {
    fn upload_event(out: &mut Outcome, u: &UploadRow) {
        out.event(
            UI_EVENT_CHANNEL_MEDIA,
            serde_json::json!({ "community_id": u.community_id, "id": u.id, "status": u.status, "done": u.done, "total": u.total }),
        );
    }

    /// A row for a file about to go up: as a post, or as a comment under
    /// `post_id` (`parent`: a reply in a branch). Refused when I may not
    /// write that now, or may not put blobs on the hosts.
    pub async fn media_placeholder(&self, keys: &Keys, community_id: &str, post_id: Option<&str>, parent: Option<&str>, path: &Path, caption: Option<&str>) -> Result<(UploadRow, Outcome)> {
        self.media_placeholder_in(keys, community_id, None, post_id, parent, path, caption, None).await
    }

    /// `media_placeholder` of a post into a section (`scope_id`; none:
    /// `everyone`); a comment goes into the scope of its post whatever is
    /// named. `batch`: the files sent by one action (an album), the
    /// same for each of them, as the messenger's descriptor carries it.
    #[allow(clippy::too_many_arguments)]
    pub async fn media_placeholder_in(
        &self,
        keys: &Keys,
        community_id: &str,
        scope_id: Option<&str>,
        post_id: Option<&str>,
        parent: Option<&str>,
        path: &Path,
        caption: Option<&str>,
        batch: Option<&str>,
    ) -> Result<(UploadRow, Outcome)> {
        let file = NewFile { scope_id, post_id, parent, caption, batch, ..NewFile::at(path) };
        self.media_placeholder_file(keys, community_id, file).await
    }

    /// `media_placeholder_in` of a file with what it carries, as a chat's
    /// does: the name it goes under, how large a picture is shown, the
    /// frame of a video the page took (its preview and size), and whether
    /// a photo goes as it is (`original`) or is made smaller before it
    /// goes (`media_send`).
    pub async fn media_placeholder_file(&self, keys: &Keys, community_id: &str, file: NewFile<'_>) -> Result<(UploadRow, Outcome)> {
        let NewFile { path, name, caption, scope_id, post_id, parent, batch, original, poster, fmt, recorded } = file;
        if batch.is_some_and(|b| !valid_batch(b)) {
            return Err(invalid("not a batch of files"));
        }
        if recorded.as_ref().is_some_and(|r| !matches!(r.kind, MediaKind::Voice | MediaKind::Circle)) {
            return Err(invalid("a recording is a voice message or a circle"));
        }
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        let post_scope = match post_id {
            Some(p) => Some(db::content::get(&self.env.db, p).await?.map_or_else(|| SCOPE_EVERYONE.to_string(), |r| r.scope_id)),
            None => scope_id.map(str::to_string),
        };
        let scope_id = Self::scope_to_write(&state, &me, post_scope.as_deref())?;
        self.may_write(keys, &state, if post_id.is_some() { Content::Comment } else { Content::Post }).await?;
        if !may_upload_file(&state, &me, post_id.is_some(), recorded.is_some()) {
            return Err(invalid("not permitted to upload"));
        }
        let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
        let most = if recorded.is_some() { MAX_RECORDING_BYTES } else { MAX_SEND_BYTES };
        if !meta.is_file() || meta.len() == 0 || meta.len() > most {
            return Err(invalid("not a file that can be sent"));
        }
        let name = safe_name(&name.map(str::to_string).unwrap_or_else(|| file_name(path)));
        let markdown = fmt == TextFormat::Markdown && caption.is_some_and(|c| !c.trim().is_empty());
        let mut carried = UploadMeta { name: Some(name.clone()), original, md: markdown, ..UploadMeta::default() };
        if let Some(r) = recorded {
            // As a chat's recording: its kind, its length, an outline of 64 values at most.
            carried.kind = Some(r.kind.as_str().to_string());
            carried.duration_ms = r.duration_ms.filter(|d| *d <= 24 * 3600 * 1000);
            carried.waveform = r.waveform.map(|w| w.into_iter().take(messenger_media::descriptor::MAX_WAVEFORM).collect());
        }
        match MediaKind::from_mime(mime_for(&name)) {
            // A picture keeps its place on the other side before it is there.
            MediaKind::Image => carried.dim = picture_size(path).await,
            // A video shows the frame the page took, and keeps its place.
            MediaKind::Video => {
                if let Some(p) = poster {
                    let (thumb, dim) = tokio::task::spawn_blocking(move || of_poster(&p)).await.unwrap_or_default();
                    (carried.thumb, carried.dim) = (thumb, dim);
                }
            }
            _ => {}
        }
        let row = UploadRow {
            id: placeholder_id(),
            community_id: community_id.to_string(),
            post_id: post_id.unwrap_or_default().to_string(),
            parent_id: parent.map(str::to_string),
            path: path.to_string_lossy().into_owned(),
            caption: caption.map(str::trim).filter(|c| !c.is_empty()).map(str::to_string),
            status: db::content::UPLOAD_UPLOADING.into(),
            error: None,
            done: 0,
            total: i64::try_from(meta.len()).unwrap_or(i64::MAX),
            created_at: self.now(),
            scope_id,
            batch: batch.map(str::to_string),
            state_json: "{}".into(),
            meta_json: carried.to_json(),
        };
        db::content::put_upload(&self.env.db, &row).await?;
        let mut out = Outcome::default();
        Self::upload_event(&mut out, &row);
        Ok((row, out))
    }

    /// A voice message or a video circle recorded in the app, as a post or
    /// a comment (`place`), as a chat's recording goes
    /// (messenger-runtime `dm_send_recording`): its bytes become a file of
    /// `outgoing` (`voice.weba`, `circle.webm` …), whose row goes up as
    /// any file's (`media_send`) and whose message is of the kind
    /// `voice` or `circle`, with its length and outline. A recording is
    /// refused when it is empty, too large (`err.file_too_large`) or of a
    /// type a chat would refuse; a reader may record a comment.
    pub async fn media_placeholder_recording(
        &self,
        keys: &Keys,
        community_id: &str,
        rec: NewRecording,
        place: RecordingPlace<'_>,
        outgoing: &Path,
    ) -> Result<(UploadRow, Outcome)> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let (kind, ext) = recording_file(&rec.kind, &rec.mime).ok_or_else(|| invalid("unsupported recording type"))?;
        // A base64 of more than the bound is refused before it is decoded.
        if rec.data_base64.len() as u64 > MAX_RECORDING_BYTES.div_ceil(3) * 4 {
            return Err(invalid("err.file_too_large"));
        }
        let bytes = B64.decode(rec.data_base64.as_bytes()).map_err(|_| invalid("the recording is not base64"))?;
        if bytes.is_empty() {
            return Err(invalid("the recording is empty"));
        }
        if bytes.len() as u64 > MAX_RECORDING_BYTES {
            return Err(invalid("err.file_too_large"));
        }
        let name = format!("{}.{ext}", kind.as_str());
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        tokio::fs::create_dir_all(outgoing).await?;
        // In the folder of copies, as a file a phone hands over (a page shows its own name).
        let path = outgoing.join(format!("{nanos}-{name}"));
        tokio::fs::write(&path, &bytes).await?;
        let file = NewFile {
            name: Some(&name),
            scope_id: place.scope_id,
            post_id: place.post_id,
            parent: place.parent,
            recorded: Some(Recorded { kind, duration_ms: rec.duration_ms, waveform: rec.waveform }),
            ..NewFile::at(&path)
        };
        let placed = self.media_placeholder_file(keys, community_id, file).await;
        if placed.is_err() {
            let _ = tokio::fs::remove_file(&path).await;
        }
        placed
    }

    /// The files of mine on their way into a channel.
    pub async fn uploads(&self, community_id: &str) -> Result<Vec<UploadRow>> {
        db::content::uploads(&self.env.db, community_id).await
    }

    /// Gives up a file on its way: the row goes, a run of it stops at once
    /// (its requests dropped) and never sends the message, and what the
    /// hosts hold of it is removed in the background.
    pub async fn media_cancel(&self, upload_id: &str) -> Result<()> {
        let running = lock(&self.transfers).get(upload_id).cloned();
        let row = db::content::upload(&self.env.db, upload_id).await?;
        db::content::delete_upload(&self.env.db, upload_id).await?;
        match running {
            // The run removes what it stored, by the state it holds.
            Some(c) => {
                c.ask(CANCEL);
            }
            None => {
                if let (Some(state), Some(keys)) = (row.and_then(|r| parse_state(&r.state_json)), self.signer()) {
                    self.remove_chunks(&keys, &state);
                }
            }
        }
        Ok(())
    }

    /// Pauses a file of mine on its way, or the file of a message coming
    /// down, by its id: its requests are dropped at once, the chunks done
    /// stay. Whether a run of it heard it.
    pub fn media_pause(&self, id: &str) -> bool {
        lock(&self.transfers).get(id).is_some_and(|c| c.ask(PAUSE))
    }

    /// Gives up the file of a message coming down: what it fetched goes.
    /// Whether a run of it heard it.
    pub fn media_fetch_cancel(&self, message_id: &str) -> bool {
        lock(&self.transfers).get(message_id).is_some_and(|c| c.ask(CANCEL))
    }

    /// "Retry now" of a file waiting for its next automatic attempt, up or
    /// down, by its id: the wait ends. Whether a run of it is there.
    pub fn media_wake(&self, id: &str) -> bool {
        lock(&self.transfers).get(id).map(|c| c.wake()).is_some()
    }

    /// A failed or paused file put back on its way: the row is `uploading`
    /// again, and the caller runs `media_send` for it, which goes on from
    /// the chunks the hosts hold.
    pub async fn media_retry(&self, upload_id: &str) -> Result<(UploadRow, Outcome)> {
        let mut row = db::content::upload(&self.env.db, upload_id).await?.ok_or_else(|| invalid("the upload was removed"))?;
        if row.status != db::content::UPLOAD_FAILED && row.status != db::content::UPLOAD_PAUSED {
            return Err(invalid("the upload has not stopped"));
        }
        if lock(&self.transfers).contains_key(upload_id) {
            return Err(invalid("the file is on its way already"));
        }
        row.status = db::content::UPLOAD_UPLOADING.into();
        row.error = None;
        db::content::set_upload_status(&self.env.db, upload_id, &row.status, None).await?;
        let mut out = Outcome::default();
        Self::upload_event(&mut out, &row);
        Ok((row, out))
    }

    /// At a start: the uploads the end of the last run interrupted, to be
    /// sent again (`media_send` goes on from the chunks the hosts hold).
    pub async fn interrupted_uploads(&self) -> Result<Vec<UploadRow>> {
        db::content::uploading(&self.env.db).await
    }

    /// The waits before the automatic attempts of a file instead of the
    /// messenger's (tests).
    pub fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = Arc::new(delays);
        self
    }

    /// A run of `id` known from now on, unless one is already.
    fn register(&self, id: &str) -> Option<Registered<'_>> {
        let mut map = lock(&self.transfers);
        if map.contains_key(id) {
            return None;
        }
        let control = Control::new();
        map.insert(id.to_string(), control.clone());
        Some(Registered { map: &self.transfers, id: id.to_string(), control })
    }

    /// What `d` was stored as, kept for the same file sent again
    /// (`reused`); `at`: when its chunks went up.
    async fn know_file(&self, d: &MediaDescriptor, at: i64) {
        let Ok(size) = i64::try_from(d.size) else { return };
        if let Err(e) = db::content::put_known_file(&self.env.db, &d.sha256, size, &d.to_envelope().encode(), at).await {
            tracing::warn!(error = %e, "a stored file was not noted");
        }
    }

    /// The file of `row` as the hosts hold it already: one this device
    /// sent or fetched before (`cm_known_files`, by its plaintext hash and
    /// size), stored within `REUSE_SECS` on hosts of this channel, each of
    /// which still has every chunk. Looked at only when a file of that
    /// size is known: the file is read whole for its hash. `None`: it goes
    /// up as a new file.
    async fn reused(&self, keys: &Keys, row: &UploadRow, storage: &[String], meter: &ProgressMeter<'_>) -> Option<MediaDescriptor> {
        let size = tokio::fs::metadata(&row.path).await.ok()?.len();
        let known = db::content::known_files(&self.env.db, i64::try_from(size).ok()?, self.now() - REUSE_SECS).await.ok()?;
        if known.is_empty() {
            return None;
        }
        meter.live().set_stage(TransferStage::Checking);
        meter.poll();
        let path = PathBuf::from(&row.path);
        let sha = tokio::task::spawn_blocking(move || file_sha256(&path)).await.ok()??;
        for (known_sha, json) in known {
            if known_sha != sha {
                continue;
            }
            let Ok(d) = messenger_core::Envelope::parse(&json).and_then(|e| MediaDescriptor::from_envelope(&e)) else { continue };
            if d.servers.is_empty() || !d.servers.iter().all(|h| storage.contains(h)) {
                continue;
            }
            let hosts = HostsBackend::new(&self.env.blobs, keys, &d.servers, d.servers.len());
            if upload::all_stored(&hosts, &d).await {
                return Some(d);
            }
        }
        None
    }

    /// A photo made smaller before its first chunk
    /// (`prepare::smaller_photo`), unless it goes as it is: written into
    /// `dir` as `<n>-<name>.jpg`, the row and what it carries follow it,
    /// and a copy the app made of the first one (in `dir` too) goes. The
    /// page hears the preparing stage meanwhile; a few photos at once
    /// (`PHOTO_SLOTS`). Whatever fails, the file goes as it is.
    async fn smaller(&self, mut row: UploadRow, carried: &mut UploadMeta, dir: &Path, sink: &dyn ProgressSink) -> Result<UploadRow> {
        let name = carried.name.clone().unwrap_or_else(|| file_name(Path::new(&row.path)));
        if carried.original || row.done > 0 || parse_state(&row.state_json).is_some() || MediaKind::from_mime(mime_for(&name)) != MediaKind::Image {
            return Ok(row);
        }
        let mut told = base_progress(&row.id, &row.community_id, "up", &name, 0, u64::try_from(row.total).unwrap_or(0));
        (told.status, told.stage) = ("running".into(), TransferStage::Preparing);
        sink.progress(told);
        let Ok(_slot) = self.photo_slots.acquire().await else { return Ok(row) };
        let first = PathBuf::from(&row.path);
        let from = first.clone();
        let Some((jpeg, dim)) = tokio::task::spawn_blocking(move || smaller_photo(&from)).await.ok().flatten() else { return Ok(row) };
        let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let made = safe_name(&format!("{stem}.jpg"));
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = dir.join(format!("{nanos}-{made}"));
        if tokio::fs::create_dir_all(dir).await.is_err() || tokio::fs::write(&path, &jpeg).await.is_err() {
            return Ok(row);
        }
        (carried.name, carried.dim) = (Some(made), Some(dim));
        row.path = path.to_string_lossy().into_owned();
        row.total = i64::try_from(jpeg.len()).unwrap_or(i64::MAX);
        row.meta_json = carried.to_json();
        db::content::set_upload_file(&self.env.db, &row.id, &row.path, row.total, &row.meta_json).await?;
        if first.parent() == Some(dir) {
            let _ = tokio::fs::remove_file(&first).await;
        }
        Ok(row)
    }

    /// Uploads the file of a placeholder through the messenger's engine:
    /// to every host of the channel, from the chunks they hold already
    /// (after a pause, a failure or the end of the app), with the
    /// automatic attempts of the messenger; once enough hosts hold every
    /// chunk, the message goes out and the placeholder goes. Before its
    /// first chunk a photo is made smaller into `outgoing` unless it goes
    /// as it is (`NewFile::original`), as a chat's is; a picture's preview
    /// is made while it goes. `sink` hears the progress as a chat's
    /// transfer tells it.
    pub async fn media_send(&self, keys: &Keys, upload_id: &str, outgoing: Option<&Path>, sink: &dyn ProgressSink) -> Result<(String, Outcome)> {
        let row = db::content::upload(&self.env.db, upload_id).await?.ok_or_else(|| invalid("the upload was removed"))?;
        let state = self.state(&row.community_id).await?;
        let mut carried = UploadMeta::parse(&row.meta_json);
        // A recording keeps its kind (`voice`, `circle`); a file picked goes as the kind of its type.
        let recorded = carried.kind.as_deref().and_then(MediaKind::parse).filter(|k| matches!(k, MediaKind::Voice | MediaKind::Circle));
        if !may_upload_file(&state, &Self::me(keys)?, !row.post_id.is_empty(), recorded.is_some()) {
            return Err(invalid("not permitted to upload"));
        }
        // A host of the kit that needs the key of its server takes the file only
        // with it (wire §9.1); none left: nothing goes up.
        let hosts = self.env.writable(keys, &state.storage, Some(&row.community_id)).await;
        if hosts.is_empty() && !state.storage.is_empty() {
            return Err(MessengerError::Transport(format!("{}: no host of the channel takes a file of mine", crate::upload_keys::NEEDS_KEY)));
        }
        let reg = self.register(upload_id).ok_or_else(|| invalid("the file is on its way already"))?;
        let row = match outgoing {
            Some(dir) => self.smaller(row, &mut carried, dir, sink).await?,
            None => row,
        };
        let name = safe_name(&carried.name.clone().unwrap_or_else(|| file_name(Path::new(&row.path))));
        let kind = recorded.unwrap_or_else(|| MediaKind::from_mime(mime_for(&name)));
        // A picture's preview is made from the file that goes, beside it.
        let thumb = (kind == MediaKind::Image && carried.thumb.is_none()).then(|| {
            let path = PathBuf::from(&row.path);
            tokio::task::spawn_blocking(move || thumb_of_file(&path))
        });
        let base = base_progress(&row.id, &row.community_id, "up", &name, u64::try_from(row.done).unwrap_or(0), u64::try_from(row.total).unwrap_or(0));
        let meter = ProgressMeter::new(sink, Arc::new(Live::default()), base);
        // A host that needs no key of its server takes the file with the key
        // of the channel for its owner's Cloud when I have one (wire §9.2):
        // the owner's plan counts and keeps it; anywhere else, with mine.
        let cloud = self.env.book.cloud_key(&row.community_id, self.now());
        let signed: Vec<(String, Option<String>, Keys)> = hosts
            .iter()
            .map(|(h, k)| (h.clone(), k.clone(), if k.is_none() { cloud.clone().unwrap_or_else(|| keys.clone()) } else { keys.clone() }))
            .collect();
        let backend = HostsBackend::signed(&self.env.blobs, &signed, MIN_HOSTS);
        let last = Mutex::new(parse_state(&row.state_json));
        // The same file stored before and whole on the hosts of the channel:
        // a new message of those chunks, nothing goes up again.
        let resuming = lock(&last).is_some();
        let reused = if resuming { None } else { self.reused(keys, &row, &state.storage, &meter).await };
        let fresh = reused.is_none();
        let ending = match reused {
            Some(d) => Ending::Done(d),
            None => tokio::select! {
                ending = self.upload_runs(&row, keys, &backend, &reg.control, &meter, &last) => ending,
                never = meter.run() => match never {},
            },
        };
        // From here no word reaches the run: a cancel that came is heeded,
        // and so is one by the removal of the row.
        let word = reg.control.close();
        let gone = db::content::upload(&self.env.db, upload_id).await?.is_none();
        let ending = match ending {
            _ if gone || word == CANCEL => Ending::Stopped(CANCEL),
            Ending::Failed(_) if word == PAUSE => Ending::Stopped(PAUSE),
            e => e,
        };
        match ending {
            Ending::Done(mut d) => {
                // The hosts that hold every chunk: those of this upload, or
                // those of the file stored before.
                if fresh {
                    d.servers = backend.holding();
                }
                // What the placeholder knew: its words and album, the name
                // the user knows, how large it is shown, its preview.
                d.caption = row.caption.clone().filter(|c| !c.trim().is_empty());
                d.batch = row.batch.clone();
                d.name = name.clone();
                d.mime = mime_for(&name).to_string();
                d.kind = kind;
                d.dim = carried.dim;
                // A recording's length and outline, as a chat's (none for a file picked).
                d.duration_ms = carried.duration_ms;
                d.waveform = carried.waveform.clone();
                d.thumb = match thumb {
                    Some(t) => t.await.ok().flatten(),
                    None => carried.thumb.clone(),
                }
                .filter(|t| valid_thumb(t));
                if let Err(e) = d.validate() {
                    meter.status("failed", Some(&short_reason(&e)));
                    return self.upload_failed(&row, e).await;
                }
                meter.live().set_stage(TransferStage::Publishing);
                meter.poll();
                let links = Links { thread: Some(row.post_id.clone()).filter(|p| !p.is_empty()), parent: row.parent_id.clone(), target: None };
                // A caption in Markdown says so, as a chat's does (`fmt`).
                let format = if carried.md && d.caption.is_some() { TextFormat::Markdown } else { TextFormat::Plain };
                let envelope = d.to_envelope().with_format(format);
                // Sealed and taken in under the channel's lock, as a post
                // is (`written`): this runs off it, beside the loop.
                let sent = {
                    let lock = self.lock_of(&row.community_id).await;
                    let _guard = lock.lock().await;
                    self.seal_and_send_in(keys, &row.community_id, &row.scope_id, links, envelope, wire::T_MSG).await
                };
                match sent {
                    Ok((id, mut out)) => {
                        db::content::delete_upload(&self.env.db, upload_id).await?;
                        // My post shows the file from where it went, as a
                        // chat's does: it is never fetched back. The same
                        // file sent again goes as these chunks.
                        db::content::put_local_file(&self.env.db, &id, &row.path).await?;
                        if fresh {
                            self.know_file(&d, self.now()).await;
                        }
                        let mut row = row;
                        row.status = "sent".into();
                        row.done = row.total;
                        Self::upload_event(&mut out, &row);
                        meter.done(None);
                        Ok((id, out))
                    }
                    Err(e) => {
                        let failed = self.upload_failed(&row, e).await;
                        drop(reg);
                        if let Err(e) = &failed {
                            meter.status("failed", Some(&short_reason(e)));
                        }
                        failed
                    }
                }
            }
            Ending::Stopped(CANCEL) => {
                if let Some(state) = lock(&last).take() {
                    self.remove_chunks(keys, &state);
                }
                db::content::delete_upload(&self.env.db, upload_id).await?;
                meter.status("cancelled", None);
                Err(MessengerError::Invalid(UPLOAD_CANCELLED.into()))
            }
            // The run is gone before its end is told: a resume asked on
            // seeing it starts a run of its own.
            Ending::Stopped(_) => {
                db::content::set_upload_status(&self.env.db, upload_id, db::content::UPLOAD_PAUSED, None).await?;
                drop(reg);
                meter.status("paused", None);
                Err(MessengerError::Invalid(UPLOAD_PAUSED.into()))
            }
            Ending::Failed(e) => {
                let e = refusal_of(&backend, e);
                // The key of the channel revoked or out of term: dropped here,
                // so that a retry goes with mine (wire §9.2).
                if cloud.is_some() && crate::cloud_keys::is_refused(&e.to_string()) {
                    if let Err(dropped) = self.drop_cloud_key(&row.community_id).await {
                        tracing::warn!(error = %dropped, "the key of the channel for the owner's Cloud was not dropped");
                    }
                }
                let failed = self.upload_failed(&row, e).await;
                drop(reg);
                if let Err(e) = &failed {
                    meter.status("failed", Some(&short_reason(e)));
                }
                failed
            }
        }
    }

    /// Runs the engine for the file of `row` until it is stored, stopped or
    /// failed for good: a failure on the way waits and goes again, as many
    /// times as the messenger's transfers do. The state of the upload is
    /// kept with the row after every chunk, and in `last`.
    async fn upload_runs(
        &self,
        row: &UploadRow,
        keys: &Keys,
        backend: &HostsBackend,
        control: &Control,
        meter: &ProgressMeter<'_>,
        last: &Mutex<Option<UploadState>>,
    ) -> Ending<MediaDescriptor> {
        let path = PathBuf::from(&row.path);
        let mut retries = 0usize;
        loop {
            let resume = lock(last).clone().filter(|s| !s.key.is_empty());
            meter.status("running", None);
            // Kept from a channel so the workers never wait for the
            // database; only the newest state is written, and every state
            // it covers is told kept (a chunk goes only after that). A row
            // gone meanwhile is a cancel: nothing more goes.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, String, tokio::sync::oneshot::Sender<()>)>();
            let (db, id, word) = (self.env.db.clone(), row.id.clone(), control.clone());
            let writer = tokio::spawn(async move {
                while let Some((mut done, mut json, ack)) = rx.recv().await {
                    let mut acks = vec![ack];
                    while let Ok(newer) = rx.try_recv() {
                        (done, json) = (newer.0, newer.1);
                        acks.push(newer.2);
                    }
                    match db::content::set_upload_state(&db, &id, i64::try_from(done).unwrap_or(i64::MAX), &json).await {
                        Ok(true) => {
                            for a in acks {
                                let _ = a.send(());
                            }
                        }
                        Ok(false) => {
                            word.ask(CANCEL);
                        }
                        Err(e) => tracing::warn!(error = %e, "the state of an upload was not kept"),
                    }
                }
            });
            let on_chunk = |done: u64, state: &UploadState| {
                *lock(last) = Some(state.clone());
                let (ack, saved) = tokio::sync::oneshot::channel();
                let _ = tx.send((done, serde_json::to_string(state).unwrap_or_else(|_| "{}".into()), ack));
                Some(saved)
            };
            let params = UploadParams { path: &path, caption: row.caption.clone(), batch: row.batch.clone(), resume, chunk_size: None };
            let ctx = TransferCtx { control: control.clone(), slots: self.chunk_slots.clone(), lane: None, workers: WORKERS, live: meter.live().clone() };
            let outcome = upload::upload(backend, params, &ctx, &on_chunk).await;
            drop(tx);
            let _ = writer.await;
            meter.poll();
            self.settle_upload(&row.id, keys, last).await;
            match outcome {
                Ok(UploadOutcome::Done(d)) => return Ending::Done(*d),
                Ok(UploadOutcome::Paused) => return Ending::Stopped(PAUSE),
                Ok(UploadOutcome::Cancelled) => return Ending::Stopped(CANCEL),
                Err(e) if retryable(&e) && retries < self.retry_delays.len() => {
                    let delay = self.retry_delays[retries];
                    retries += 1;
                    if let Some(stop) = wait_retry(control, meter, retries, delay, &e).await {
                        return Ending::Stopped(stop);
                    }
                }
                Err(e) => return Ending::Failed(e),
            }
        }
    }

    async fn upload_failed(&self, row: &UploadRow, e: MessengerError) -> Result<(String, Outcome)> {
        db::content::set_upload_status(&self.env.db, &row.id, db::content::UPLOAD_FAILED, Some(&e.to_string())).await?;
        Err(e)
    }

    /// What an upload no longer uses (another set of hosts, an earlier
    /// key) is removed from where it was left, in the background, never
    /// what it holds now; its state forgets it.
    async fn settle_upload(&self, id: &str, keys: &Keys, last: &Mutex<Option<UploadState>>) {
        let (stale, state) = {
            let mut l = lock(last);
            let Some(s) = l.as_mut() else { return };
            if s.stale.is_empty() {
                return;
            }
            (std::mem::take(&mut s.stale), s.clone())
        };
        let json = serde_json::to_string(&state).unwrap_or_else(|_| "{}".into());
        let _ = db::content::set_upload_state(&self.env.db, id, i64::try_from(state.stored_bytes()).unwrap_or(0), &json).await;
        for s in stale {
            let held = if state.server_base.as_deref() == Some(s.base.as_str()) { state.on_server() } else { Vec::new() };
            let chunks = s.chunks.into_iter().chain(s.unconfirmed).filter(|c| !held.contains(c)).collect();
            self.remove_blobs(keys, hosts_of(&s.base), chunks);
        }
    }

    /// Everything an upload put on the hosts, removed in the background:
    /// where it holds its chunks now and where it left some before.
    fn remove_chunks(&self, keys: &Keys, state: &UploadState) {
        if let Some(base) = &state.server_base {
            self.remove_blobs(keys, hosts_of(base), state.on_server());
        }
        for s in &state.stale {
            self.remove_blobs(keys, hosts_of(&s.base), s.chunks.iter().chain(&s.unconfirmed).cloned().collect());
        }
    }

    /// Removes `chunks` from `hosts` in the background. A failure leaves
    /// them there: encrypted, named by no message, for the host's own
    /// clean-up.
    fn remove_blobs(&self, keys: &Keys, hosts: Vec<String>, chunks: Vec<String>) {
        if hosts.is_empty() || chunks.is_empty() {
            return;
        }
        let hosts: Vec<(String, Option<String>)> = hosts.into_iter().map(|h| { let k = self.env.host_key(keys, &h, None); (h, k) }).collect();
        // A chunk put with a key of a channel for its owner's Cloud (wire
        // §9.2) is removed by that key: a host where mine fails is asked
        // with each of them in turn.
        let others = self.env.book.cloud_keys(self.now());
        let (blobs, keys) = (self.env.blobs.clone(), keys.clone());
        tokio::spawn(async move {
            for (host, key) in &hosts {
                for sha in &chunks {
                    let mut done = blobs.delete_keyed(&keys, key.as_deref(), host, sha).await;
                    if key.is_none() {
                        for other in &others {
                            if done.is_ok() {
                                break;
                            }
                            done = blobs.delete_keyed(other, None, host, sha).await;
                        }
                    }
                    if let Err(e) = done {
                        tracing::debug!(host, error = %e, "a chunk of a file given up was not removed");
                    }
                }
            }
        });
    }

    /// The file of a message into `dir` through the messenger's engine:
    /// its chunks several at once from the hosts the message names, then
    /// the channel's own, each checked by its hash, those fetched before
    /// kept on disk for the next attempt; opened with the key of the
    /// message and checked whole. A second ask while one runs waits for
    /// it and finds the file. `sink` hears the progress.
    pub async fn media_fetch(&self, community_id: &str, message_id: &str, dir: &Path, sink: &dyn ProgressSink) -> Result<PathBuf> {
        let row = db::content::get(&self.env.db, message_id).await?.filter(|r| r.community_id == community_id).ok_or_else(|| invalid("unknown message"))?;
        let json = row.media_json.ok_or_else(|| invalid("not a file"))?;
        let d = MediaDescriptor::from_envelope(&messenger_core::Envelope::parse(&json)?)?;
        let state = self.state(community_id).await?;
        tokio::fs::create_dir_all(dir).await?;
        // Here already: a file of mine where it was sent from, one fetched
        // before (by the engine or before it). Nothing is fetched again.
        if let Some(here) = self.local_file(message_id, dir, &d).await {
            return Ok(here);
        }
        let reg = self.register(message_id);
        let control = reg.as_ref().map(|r| r.control.clone()).unwrap_or_default();
        let meter = ProgressMeter::new(sink, Arc::new(Live::default()), base_progress(message_id, community_id, "down", &d.name, 0, d.size));
        let fetcher = HostsFetcher { blobs: self.env.blobs.clone() };
        let ending = tokio::select! {
            ending = self.download_runs(&fetcher, &d, &state.storage, dir, &control, &meter) => ending,
            never = meter.run() => match never {},
        };
        control.close();
        match ending {
            Ending::Done(path) => {
                // The same file sent again goes as these chunks (`reused`).
                self.know_file(&d, row.created_at).await;
                meter.done(Some(path.to_string_lossy().into_owned()));
                Ok(path)
            }
            Ending::Stopped(stop) => {
                meter.status(if stop == CANCEL { "cancelled" } else { "paused" }, None);
                Err(MessengerError::Invalid(FETCH_STOPPED.into()))
            }
            Ending::Failed(e) => {
                meter.status("failed", Some(&short_reason(&e)));
                Err(e)
            }
        }
    }

    async fn download_runs(&self, fetcher: &HostsFetcher, d: &MediaDescriptor, hosts: &[String], dir: &Path, control: &Control, meter: &ProgressMeter<'_>) -> Ending<PathBuf> {
        let mut retries = 0usize;
        loop {
            meter.status("running", None);
            let ctx = TransferCtx { control: control.clone(), slots: self.chunk_slots.clone(), lane: None, workers: WORKERS, live: meter.live().clone() };
            let outcome = download::download(fetcher, d, hosts, dir, &ctx, &|_, _| {}).await;
            meter.poll();
            match outcome {
                Ok(DownloadOutcome::Done(p)) => return Ending::Done(p),
                Ok(DownloadOutcome::Paused) => return Ending::Stopped(PAUSE),
                Ok(DownloadOutcome::Cancelled) => return Ending::Stopped(CANCEL),
                Err(e) if retryable(&e) && retries < self.retry_delays.len() => {
                    let delay = self.retry_delays[retries];
                    retries += 1;
                    if let Some(stop) = wait_retry(control, meter, retries, delay, &e).await {
                        return Ending::Stopped(stop);
                    }
                }
                Err(e) => return Ending::Failed(e),
            }
        }
    }

    /// Where the file of a message is here: a file of mine where it was
    /// sent from (while it is that file still: its size), else one fetched
    /// into `dir`, by the engine or before it.
    async fn local_file(&self, message_id: &str, dir: &Path, d: &MediaDescriptor) -> Option<PathBuf> {
        if let Ok(Some(mine)) = db::content::local_file(&self.env.db, message_id).await {
            if tokio::fs::metadata(&mine).await.is_ok_and(|m| m.is_file() && m.len() == d.size) {
                return Some(PathBuf::from(mine));
            }
        }
        if let Some(p) = download::cached(dir, d).await {
            return Some(p);
        }
        let legacy = local_copy(dir, d);
        tokio::fs::metadata(&legacy).await.is_ok_and(|m| m.is_file()).then_some(legacy)
    }

    /// The file of a message fetched into `dir`, with the type it plays
    /// as, for a page to play a video or a sound from it however large it
    /// is (the messenger's server of files, as `messenger_media_playable`):
    /// `None` while it is not here, or when it is not a video or a sound.
    pub async fn media_playable(&self, community_id: &str, message_id: &str, dir: &Path) -> Result<Option<(PathBuf, String)>> {
        let row = db::content::get(&self.env.db, message_id).await?.filter(|r| r.community_id == community_id).ok_or_else(|| invalid("unknown message"))?;
        let json = row.media_json.ok_or_else(|| invalid("not a file"))?;
        let d = MediaDescriptor::from_envelope(&messenger_core::Envelope::parse(&json)?)?;
        let Some(mime) = playable_mime(&d.mime) else { return Ok(None) };
        Ok(self.local_file(message_id, dir, &d).await.map(|p| (p, mime)))
    }

    /// The file of a message fetched into `dir` (`media_fetch`) as a
    /// `data:` URL, for a page to show it inline: `None` while it is not
    /// here, or when it is of a type a webview does not show passively.
    pub async fn media_data_url(&self, community_id: &str, message_id: &str, dir: &Path) -> Result<Option<String>> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let row = db::content::get(&self.env.db, message_id).await?.filter(|r| r.community_id == community_id).ok_or_else(|| invalid("unknown message"))?;
        let json = row.media_json.ok_or_else(|| invalid("not a file"))?;
        let d = MediaDescriptor::from_envelope(&messenger_core::Envelope::parse(&json)?)?;
        let Some(path) = self.local_file(message_id, dir, &d).await else { return Ok(None) };
        let Ok(meta) = tokio::fs::metadata(&path).await else { return Ok(None) };
        let mime = base_mime(&d.mime);
        if !passive(&mime) {
            return Ok(None);
        }
        if meta.len() > MAX_INLINE_BYTES {
            // The messenger's code, so a page says the same words as in a chat.
            return Err(MessengerError::Invalid("err.too_large_for_preview".into()));
        }
        let bytes = tokio::fs::read(&path).await?;
        Ok(Some(format!("data:{mime};base64,{}", B64.encode(bytes))))
    }

    // ─── The cover ──────────────────────────────────────────────────────────

    /// The cover of a channel as a `data:` URL for a page, from this
    /// device's folder `dir`: read from the hosts of the channel's own
    /// `storage` (never an address the picture names, so a manager's
    /// setting makes this device ask no other host), checked by the hash
    /// the address carries, made a square JPEG again as a picture of a
    /// group is, and kept by that hash — the next ask, of the list of
    /// chats or of the card, reads the file and fetches nothing. `None`
    /// when the channel has no cover or no host has it now.
    pub async fn cover_data_url(&self, community_id: &str, dir: &Path) -> Result<Option<String>> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let state = self.state(community_id).await?;
        let Some(sha) = cover_hash(&state.picture) else { return Ok(None) };
        tokio::fs::create_dir_all(dir).await?;
        let target = dir.join(format!("cover-{sha}.jpg"));
        let lock = self.fetches.lock().await.entry(target.clone()).or_default().clone();
        let _one = lock.lock().await;
        if !tokio::fs::try_exists(&target).await.unwrap_or(false) {
            let raw: [u8; 32] = hex::decode(&sha).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("the address of the cover"))?;
            let Ok(blob) = fetch_verified_up_to(self.env.blobs.as_ref(), &state.storage, &raw, MAX_COVER_BYTES as u64).await else { return Ok(None) };
            let jpeg = tokio::task::spawn_blocking(move || {
                let img = messenger_avatar::decode(&blob)?;
                let whole = messenger_avatar::CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
                messenger_avatar::crop_square(&img, whole, messenger_avatar::OWN_SIDE)
            })
            .await
            .map_err(|e| MessengerError::Other(e.to_string()))?
            .map_err(|_| invalid("a cover is not a picture"))?;
            let part = target.with_extension("part");
            tokio::fs::write(&part, &jpeg).await?;
            tokio::fs::rename(&part, &target).await?;
        }
        let bytes = tokio::fs::read(&target).await?;
        Ok(Some(format!("data:image/jpeg;base64,{}", B64.encode(bytes))))
    }

    /// A new cover from any picture the user picked: `upload_cover`, then
    /// `apply_cover` (the module runs the first off the channel's lock).
    pub async fn set_cover(&self, keys: &Keys, community_id: &str, bytes: &[u8]) -> Result<Outcome> {
        let cover = self.upload_cover(keys, community_id, bytes).await?;
        self.apply_cover(keys, community_id, cover).await
    }

    /// A cover made from any picture the user picked (up to
    /// `MAX_COVER_INPUT_BYTES`): as the picture of a group is, a square
    /// JPEG of `messenger_avatar::OWN_SIDE` of its center, written from
    /// pixels alone (no EXIF, no place), and so always within the 1 MB of
    /// wire §9. Put on every host at once as a public blob with the user's
    /// key — the uploader pays, as for the picture of a group; `MIN_HOSTS`
    /// that take it are enough, so a host that is down holds the cover
    /// back no longer (живой запуск 2026-10-23: eu-1 down, the cover
    /// refused twice, each after 12 s of hosts asked one after another).
    /// Nothing of the channel is written here: no lock is held while the
    /// hosts answer.
    pub async fn upload_cover(&self, keys: &Keys, community_id: &str, bytes: &[u8]) -> Result<NewCover> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        if !may_upload(&state, &me) {
            return Err(invalid("not permitted to upload"));
        }
        let jpeg = picked_jpeg(bytes, "cover", None).await?;
        let sha = sha256_hex(&jpeg);
        let jpeg = Arc::new(jpeg);
        let copies = self.put_everywhere(keys, &state.storage, &sha, jpeg.clone(), "image/jpeg", "cover").await?;
        let url = format!("{}/{}", copies[0].trim_end_matches('/'), sha);
        if !url.starts_with("https://") || url.chars().count() > MAX_PICTURE_CHARS {
            return Err(invalid("the address of the cover"));
        }
        Ok(NewCover { sha, url, copies, jpeg: Arc::unwrap_or_clone(jpeg) })
    }

    /// A blob put on every host of `storage` at once (the slowest one sets
    /// the time, not their sum); the hosts that took it, in the order of
    /// `storage`, at least `MIN_HOSTS` of them (or all, when fewer).
    async fn put_everywhere(&self, keys: &Keys, storage: &[String], sha: &str, bytes: Arc<Vec<u8>>, mime: &'static str, what: &str) -> Result<Vec<String>> {
        // A host of the kit that needs the key of its server, which this
        // device has not, is asked nothing (wire §9.1).
        let writable = self.env.writable(keys, storage, None).await;
        if writable.is_empty() && !storage.is_empty() {
            return Err(MessengerError::Transport(format!("{}: no host of the channel takes the {what}", crate::upload_keys::NEEDS_KEY)));
        }
        let storage: Vec<String> = writable.iter().map(|(h, _)| h.clone()).collect();
        let storage = storage.as_slice();
        let mut asked = tokio::task::JoinSet::new();
        for (i, (host, key)) in writable.iter().enumerate() {
            let (blobs, keys, host, key, sha, bytes) = (self.env.blobs.clone(), keys.clone(), host.clone(), key.clone(), sha.to_string(), bytes.clone());
            asked.spawn(async move { (i, blobs.put_typed_keyed(&keys, key.as_deref(), &host, &sha, &bytes, mime).await) });
        }
        let mut taken = vec![false; storage.len()];
        let mut refused = None;
        while let Some(done) = asked.join_next().await {
            let (i, put) = done.map_err(|e| MessengerError::Other(e.to_string()))?;
            match put {
                Ok(()) => taken[i] = true,
                Err(e) => {
                    let host = &storage[i];
                    tracing::warn!(host, error = %e, "a host did not take the {what}");
                    refused = Some((host.clone(), e));
                }
            }
        }
        let copies: Vec<String> = storage.iter().zip(&taken).filter(|(_, t)| **t).map(|(h, _)| h.clone()).collect();
        if copies.is_empty() || copies.len() < MIN_HOSTS.min(storage.len()).max(1) {
            return Err(not_taken(refused, what));
        }
        Ok(copies)
    }

    // ─── The photo of a section ─────────────────────────────────────────────

    /// The photo of a section made from any picture the owner or an admin
    /// picked (wire §14.1, 2026-10-23; owner and admins, 2026-10-25): a JPEG
    /// of `SCOPE_PHOTO_SIZE` from the middle of it (2026-10-25; a square as a
    /// cover before), sealed under a key
    /// of its own (`wire::seal_bytes`) and put on the hosts of the channel
    /// as the cover is; the key travels in the sealed meta alone, so the
    /// hosts hold a picture nobody else opens. Nothing of the channel is
    /// written here: `scope_set` names it (the module runs this off the
    /// channel's lock).
    pub async fn upload_scope_photo(&self, keys: &Keys, community_id: &str, bytes: &[u8]) -> Result<NewScopePhoto> {
        let state = self.state(community_id).await?;
        if !state.role_of(&Self::me(keys)?).is_some_and(|r| crate::roles::permits(r, crate::roles::Action::SetScope)) {
            return Err(invalid("not permitted"));
        }
        let jpeg = picked_jpeg(bytes, "photo", Some(SCOPE_PHOTO_SIZE)).await?;
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let sealed = wire::seal_bytes(&keydist::ChannelKey(raw), &jpeg)?.into_bytes();
        let sha = sha256_hex(&sealed);
        let copies = self.put_everywhere(keys, &state.storage, &sha, Arc::new(sealed), "application/octet-stream", "photo").await?;
        use base64::Engine as _;
        let photo = wire::ScopePhoto { sha256: sha, key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw) };
        Ok(NewScopePhoto { photo, copies, jpeg })
    }

    /// The photo of a section as a `data:` URL for a page, from the folder
    /// `dir` (`scope-<sha>.jpg`), else fetched from the hosts of the
    /// channel and opened by the key of its meta. `None`: the section has
    /// no photo, its meta is not mine to read, or no host has it now.
    pub async fn scope_photo_data_url(&self, community_id: &str, scope_id: &str, dir: &Path) -> Result<Option<String>> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let Some(log) = self.log(community_id).await? else { return Ok(None) };
        let Some(photo) = self.names(&log).await?.look(scope_id).photo else { return Ok(None) };
        tokio::fs::create_dir_all(dir).await?;
        let target = dir.join(format!("scope-{}.jpg", photo.sha256));
        let lock = self.fetches.lock().await.entry(target.clone()).or_default().clone();
        let _one = lock.lock().await;
        if !tokio::fs::try_exists(&target).await.unwrap_or(false) {
            let raw: [u8; 32] = hex::decode(&photo.sha256).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("the address of the photo"))?;
            let storage = &log.state().storage;
            let Ok(blob) = fetch_verified_up_to(self.env.blobs.as_ref(), storage, &raw, MAX_SCOPE_PHOTO_BLOB as u64).await else { return Ok(None) };
            let sealed = String::from_utf8(blob).map_err(|_| invalid("a photo is not sealed"))?;
            let jpeg = wire::open_bytes(&photo.sealing_key()?, &sealed)?;
            if messenger_avatar::picture_ext(&jpeg).is_none() {
                return Err(invalid("a photo is not a picture"));
            }
            let part = target.with_extension("part");
            tokio::fs::write(&part, &jpeg).await?;
            tokio::fs::rename(&part, &target).await?;
        }
        let bytes = tokio::fs::read(&target).await?;
        Ok(Some(format!("data:image/jpeg;base64,{}", B64.encode(bytes))))
    }

    /// Removes the photo of a section no meta names any more from the hosts
    /// it may be on; a host that fails keeps it (sealed, of no use to it).
    pub(crate) async fn drop_scope_photo(&self, keys: &Keys, storage: &[String], photo: &wire::ScopePhoto) {
        for host in storage {
            if let Err(e) = self.env.blobs.delete_keyed(keys, self.env.host_key(keys, host, None).as_deref(), host, &photo.sha256).await {
                tracing::debug!(host, error = %e, "an old photo of a section was not removed");
            }
        }
    }

    /// A cover on the hosts (`upload_cover`) set as `picture` by an
    /// operation of the journal, kept here, and the previous one removed
    /// from the hosts. Under the channel's lock in the module: local work,
    /// the operation goes out by the outbox.
    pub async fn apply_cover(&self, keys: &Keys, community_id: &str, cover: NewCover) -> Result<Outcome> {
        let NewCover { sha, url, copies, .. } = cover;
        let before = db::avatar::get(&self.env.db, community_id).await?;
        let out = self.act(keys, community_id, settings_picture(url.clone())).await?;
        let now = self.now();
        db::avatar::set(
            &self.env.db,
            &ChannelAvatar { community_id: community_id.to_string(), sha256: sha.clone(), url, server_id: None, copies_json: serde_json::to_string(&copies)?, set_at: now, checked_at: now, touched_at: now },
        )
        .await?;
        if let Some(old) = before.filter(|o| o.sha256 != sha) {
            self.drop_cover(keys, &old).await;
        }
        Ok(out)
    }

    /// The cover taken away: `picture` emptied, the blob removed.
    pub async fn remove_cover(&self, keys: &Keys, community_id: &str) -> Result<Outcome> {
        let before = db::avatar::get(&self.env.db, community_id).await?;
        let out = self.act(keys, community_id, settings_picture(String::new())).await?;
        db::avatar::clear(&self.env.db, community_id).await?;
        if let Some(old) = before {
            self.drop_cover(keys, &old).await;
        }
        Ok(out)
    }

    /// Removes a cover from the hosts it was put on; a host that fails
    /// keeps it (a stale picture, nothing secret).
    async fn drop_cover(&self, keys: &Keys, old: &ChannelAvatar) {
        for host in serde_json::from_str::<Vec<String>>(&old.copies_json).unwrap_or_default() {
            if let Err(e) = self.env.blobs.delete_keyed(keys, self.env.host_key(keys, &host, None).as_deref(), &host, &old.sha256).await {
                tracing::debug!(host, error = %e, "an old cover was not removed");
            }
        }
    }
}

/// A picture the user picked (up to `MAX_COVER_INPUT_BYTES`) as a JPEG of
/// its center from pixels alone, within `MAX_COVER_BYTES`: `None`, a square
/// of `messenger_avatar::OWN_SIDE` (a cover); `Some((w, h))`, that shape
/// (the photo of a section).
async fn picked_jpeg(bytes: &[u8], what: &str, shape: Option<(u32, u32)>) -> Result<Vec<u8>> {
    if bytes.len() > MAX_COVER_INPUT_BYTES {
        return Err(invalid(&format!("a {what} is too large")));
    }
    let picked = bytes.to_vec();
    let jpeg = tokio::task::spawn_blocking(move || {
        let img = messenger_avatar::decode(&picked)?;
        match shape {
            Some((w, h)) => messenger_avatar::crop_center(&img, w, h, MAX_COVER_BYTES),
            None => {
                let whole = messenger_avatar::CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
                messenger_avatar::crop_square(&img, whole, messenger_avatar::OWN_SIDE)
            }
        }
    })
    .await
    .map_err(|e| MessengerError::Other(e.to_string()))?
    .map_err(|_| invalid(&format!("a {what} is not a picture")))?;
    if jpeg.len() > MAX_COVER_BYTES {
        return Err(invalid(&format!("a {what} is too large")));
    }
    Ok(jpeg)
}

/// Where `media_fetch` keeps the file of a descriptor in `dir`: the hash
/// in front, so that two files of one name do not meet.
/// The hash a cover's address ends with (`https://host/<sha256>`); none for
/// an address of another shape.
fn cover_hash(picture: &str) -> Option<String> {
    let rest = picture.strip_prefix("https://")?;
    let (_, tail) = rest.rsplit_once('/')?;
    (tail.len() == 64 && tail.bytes().all(|b| b.is_ascii_hexdigit())).then(|| tail.to_ascii_lowercase())
}

fn local_copy(dir: &Path, d: &MediaDescriptor) -> PathBuf {
    dir.join(format!("{}-{}", d.sha256.get(..12).unwrap_or(&d.sha256), safe_name(&d.name)))
}

/// `audio/webm;codecs=opus` → `audio/webm`.
fn base_mime(mime: &str) -> String {
    mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// Types a webview renders passively, the messenger's list
/// (messenger-runtime `media::passive`): pictures, videos, sounds; never
/// HTML or SVG, which could run in the page.
fn passive(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/avif" | "image/bmp"
            | "video/mp4" | "video/webm" | "video/quicktime"
            | "audio/mpeg" | "audio/ogg" | "audio/wav" | "audio/mp4" | "audio/flac" | "audio/webm"
    )
}

/// The type a video or a sound plays as from the messenger's server of
/// files (its base, among the passive ones); `None` for anything else.
pub fn playable_mime(mime: &str) -> Option<String> {
    let base = base_mime(mime);
    (passive(&base) && (base.starts_with("video/") || base.starts_with("audio/"))).then_some(base)
}

/// `playable_mime` of a file by its name (a file picked to be sent).
pub fn playable_by_name(name: &str) -> Option<String> {
    playable_mime(mime_for(name))
}

fn settings_picture(picture: String) -> OpBody {
    OpBody::Settings {
        name: None,
        about: None,
        picture: Some(picture),
        history: None,
        relays: None,
        storage: None,
        post_mode: None,
        premod: None,
        schedule: None,
        comments: None,
        join: None,
    }
}

#[cfg(test)]
mod picked_tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 13) as u8 * 19, 200, 255]).collect();
        messenger_avatar::png_of_rgba(w, h, &rgba).unwrap_or_else(|e| panic!("png: {e:?}"))
    }

    fn jpeg_of(url: &str) -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let b64 = url.strip_prefix("data:image/jpeg;base64,").unwrap_or_else(|| panic!("not a JPEG data URL: {url:.40}"));
        B64.decode(b64).unwrap_or_else(|e| panic!("base64: {e}"))
    }

    /// A photo becomes a small square, whatever its shape and size.
    #[test]
    fn a_picture_is_shown_as_a_small_square() {
        for (w, h) in [(1200, 800), (300, 900), (40, 30)] {
            let url = picked_preview(png(w, h).as_slice()).unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("{w}x{h}: no preview"));
            let img = messenger_avatar::decode(&jpeg_of(&url)).unwrap_or_else(|e| panic!("{e:?}"));
            assert_eq!((img.width(), img.height()), (PICKED_PREVIEW_SIDE, PICKED_PREVIEW_SIDE), "{w}x{h}");
        }
    }

    /// Anything else has no preview: the tray shows its icon and name.
    #[test]
    fn what_is_not_a_picture_has_no_preview() {
        let none = |bytes: &[u8]| picked_preview(bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(none(b""), None);
        assert_eq!(none(b"%PDF-1.7 a document"), None);
        // A video (the first box of an mp4) is not decoded here.
        assert_eq!(none(b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom"), None);
        // The head of a picture with nothing after it does not decode.
        assert_eq!(none(&png(64, 64)[..40]), None);
    }

    /// No more is read of a file than a cover is made from.
    #[test]
    fn a_picture_too_large_is_not_read_whole() {
        struct Counted<'a>(&'a [u8], usize);
        impl std::io::Read for Counted<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = std::io::Read::read(&mut self.0, buf)?;
                self.1 += n;
                Ok(n)
            }
        }
        let mut big = png(8, 8);
        big.resize(MAX_COVER_INPUT_BYTES + 10, 0);
        let mut src = Counted(&big, 0);
        assert_eq!(picked_preview(&mut src).unwrap_or_else(|e| panic!("{e}")), None);
        assert_eq!(src.1, MAX_COVER_INPUT_BYTES + 1);
    }
}

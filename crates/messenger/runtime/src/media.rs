// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Attachments: glue between the DM module (message rows), the media
//! module (blobs and transfers) and the outbox. Uploads and downloads run
//! in background tasks; progress reaches the host as `transfer.progress`
//! events.
//!
//! A photo is made smaller before its upload unless the user asks for the
//! original (`PHOTO_SIDE`, `PHOTO_QUALITY`): the transfer is in the
//! preparing stage meanwhile. Transfers the closing of the app interrupted
//! start again by themselves once a session runs, and a transfer waiting
//! for its next attempt makes it at once when the relays come back; one
//! whose attempts ran out while the connection was lost starts again then.

use crate::{MessengerRuntime, REGION_FALLBACK};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::traits::UiEvent;
use messenger_core::{MessengerError, PubKey, Result};
use messenger_dm::{DmService, MessageView};
use messenger_ingress::Outbox;
pub use messenger_media::descriptor::MAX_SEND_BYTES;
use messenger_media::descriptor::{valid_thumb, MAX_THUMB_BYTES};
use messenger_media::service::{progress_of, short_reason, SMALL_BYTES};
use messenger_media::{
    Cancelled, MediaDescriptor, MediaKind, MediaServerInput, MediaServerView, MediaService, Paused, Progress, ProgressSink, Publishing,
    TransferStage, TransferView,
};
use messenger_store::media::{DIR_DOWN, DIR_UP, REASON_INTERRUPTED, ST_FAILED, ST_PAUSED, ST_QUEUED, ST_RUNNING, ST_WAITING_RETRY};
use crate::relays::ServersMode;
use nostr::key::Keys;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};

pub const UI_EVENT_TRANSFER: &str = "transfer.progress";
/// Received files up to this size are fetched without asking.
pub const AUTO_DOWNLOAD_BYTES: u64 = SMALL_BYTES;
/// Largest file handed to the UI inline (previews).
pub const MAX_INLINE_BYTES: u64 = 24 * 1024 * 1024;

/// Largest recording accepted from the UI in one call.
pub const MAX_RECORDING_BYTES: u64 = 64 * 1024 * 1024;

/// A photo is sent with its longer side at most this, unless the user
/// asks for the original; as the earlier app did.
pub const PHOTO_SIDE: u32 = 1280;
/// The JPEG quality of a photo made smaller.
pub const PHOTO_QUALITY: u8 = 78;
/// A photo larger than this is made smaller even when its sides are
/// within `PHOTO_SIDE`.
pub const PHOTO_MAX_BYTES: u64 = 1024 * 1024;
/// The first bytes of a picture read for its size.
const HEADER_BYTES: u64 = 512 * 1024;
/// The longer side of a preview carried in the message, tried smaller
/// until it fits `MAX_THUMB_BYTES`.
const THUMB_SIDES: [u32; 3] = [160, 96, 64];
/// A frame the UI hands over is never larger than this (base64).
const MAX_POSTER_B64: usize = 1024 * 1024;

/// A frame of a video the UI took from the picked file, shown before the
/// file is fetched: a JPEG (base64) and the size of the video.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Poster {
    pub jpeg: String,
    pub width: u32,
    pub height: u32,
}

/// A preview of a picture (any the avatar crate reads), written anew as a
/// small JPEG: base64 within `MAX_THUMB_BYTES`; `None` when it cannot be
/// read. Only pixels go: nothing of the source survives.
fn thumb_of(bytes: &[u8]) -> Option<String> {
    let img = messenger_avatar::decode(bytes).ok()?;
    THUMB_SIDES
        .into_iter()
        .map(|side| messenger_avatar::preview_jpeg(&img, side))
        .find(|j| j.len() <= MAX_THUMB_BYTES)
        .map(|j| B64.encode(j))
}

/// `thumb_of` the picture at `path`, within the avatar crate's limits.
fn thumb_of_file(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > messenger_avatar::MAX_INPUT_BYTES as u64 {
        return None;
    }
    thumb_of(&std::fs::read(path).ok()?)
}

/// What a transfer is in while it is not over.
const NOT_OVER: [&str; 5] = [ST_QUEUED, ST_RUNNING, ST_WAITING_RETRY, ST_PAUSED, ST_FAILED];
/// Failures that mean the connection was lost: such a transfer starts
/// again by itself when the network is back.
const LOST_CONNECTION: [&str; 2] = ["err.network", "err.timeout"];

/// Photos made smaller at once. Each holds its file and the decoded
/// picture in memory (hundreds of megabytes for a large one), and an album
/// reaches the preparing stage all together: the others wait in it. A
/// phone has less to spare.
pub const PHOTO_SLOTS: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 1 } else { 2 };

/// A voice message or a video circle as the UI hands it over.
pub struct Recording {
    pub kind: MediaKind,
    /// As the recorder reported it; codec parameters are dropped.
    pub mime: String,
    pub duration_ms: Option<u64>,
    pub waveform: Option<Vec<u8>>,
    pub bytes: Vec<u8>,
}

struct AttachmentMeta {
    kind: MediaKind,
    mime: String,
    duration_ms: Option<u64>,
    waveform: Option<Vec<u8>>,
}

/// A picked file waiting in the composer (`MessengerRuntime::picked`).
#[derive(Clone, Debug, serde::Serialize)]
pub struct PickedView {
    /// Where it is on this device; what is sent.
    pub path: String,
    pub name: String,
    /// `image`, `video`, `audio` or `file`.
    pub kind: String,
    /// The type it is sent as, by its name.
    pub mime: String,
    pub size: u64,
    /// A picture as a `data:` url; `None` for anything else.
    pub preview: Option<String>,
}

/// `audio/webm;codecs=opus` → `audio/webm`.
fn base_mime(mime: &str) -> String {
    mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

struct UiSink(broadcast::Sender<UiEvent>);

impl ProgressSink for UiSink {
    fn progress(&self, p: Progress) {
        let _ = self.0.send(UiEvent {
            name: UI_EVENT_TRANSFER.into(),
            payload: serde_json::to_value(&p).unwrap_or(serde_json::Value::Null),
        });
    }
}

fn updated_event(chat_id: &str, message_id: &str) -> UiEvent {
    UiEvent {
        name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
        payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
    }
}

/// Can the file be sent: a file, not empty, within `MAX_SEND_BYTES` (what
/// others send may be larger; see `messenger_media::descriptor`).
fn sendable(meta: &std::fs::Metadata) -> Result<()> {
    let refused = if !meta.is_file() {
        "err.not_a_file"
    } else if meta.len() == 0 {
        "err.file_empty"
    } else if meta.len() > MAX_SEND_BYTES {
        "err.file_too_large"
    } else {
        return Ok(());
    };
    Err(MessengerError::Invalid(refused.into()))
}

/// The upright size of the picture at `path`, from its first bytes;
/// `None` when they do not tell it.
async fn picture_size(path: &Path) -> Option<(u32, u32)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut head = Vec::new();
        std::fs::File::open(path).ok()?.take(HEADER_BYTES).read_to_end(&mut head).ok()?;
        messenger_avatar::upright_size(&head)
    })
    .await
    .ok()
    .flatten()
}

/// The photo at `path` made smaller, when that is worth it: a still JPEG,
/// PNG or WebP whose longer side is over `PHOTO_SIDE` or whose file is
/// over `PHOTO_MAX_BYTES`, written again as a JPEG that is smaller than
/// the file. The JPEG and its size; `None` to send the file as it is
/// (also when it cannot be decoded).
fn smaller_photo(path: &Path) -> Option<(Vec<u8>, (u32, u32))> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > messenger_avatar::MAX_INPUT_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if !messenger_avatar::still_photo(&bytes) {
        return None;
    }
    let (w, h) = messenger_avatar::upright_size(&bytes)?;
    if w.max(h) <= PHOTO_SIDE && len <= PHOTO_MAX_BYTES {
        return None;
    }
    let img = messenger_avatar::decode(&bytes).ok()?;
    let jpeg = messenger_avatar::photo_jpeg(&img, PHOTO_SIDE, PHOTO_QUALITY);
    let size = messenger_avatar::upright_size(&jpeg)?;
    ((jpeg.len() as u64) < len).then_some((jpeg, size))
}

/// The folder of `path` when the app made it in `outgoing/` (one folder
/// for each file there): a copy of what was picked on Android, a
/// recording, a photo made smaller. `None` for a file the user picked
/// where it is, which is never removed.
fn own_copy(outgoing: &Path, path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    (dir.parent()? == outgoing).then(|| dir.to_path_buf())
}

/// Remove the folder of `path` when the app made it (`own_copy`): nothing
/// refers to that file any more.
async fn drop_own_copy(outgoing: &Path, path: &Path) {
    if let Some(dir) = own_copy(outgoing, path) {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}

/// Everything a download task needs, detached from the runtime's lifetime.
#[derive(Clone)]
struct FetchJob {
    dm: DmService,
    media: MediaService,
    ui: broadcast::Sender<UiEvent>,
}

impl FetchJob {
    /// Fetch the attachment of a message; its row learns where the file is.
    async fn run(self, message_id: String, chat_id: String, d: MediaDescriptor, manual: bool) -> Result<Option<PathBuf>> {
        let sink = UiSink(self.ui.clone());
        let path = self.media.run_download(&message_id, &chat_id, &d, manual, &sink).await?;
        if let Some(p) = &path {
            self.dm.media_set_local_path(&message_id, &p.to_string_lossy()).await?;
            let _ = self.ui.send(updated_event(&chat_id, &message_id));
        }
        Ok(path)
    }
}

/// Everything an upload task needs, detached from the runtime's lifetime.
#[derive(Clone)]
struct UploadJob {
    groups: messenger_groups::GroupService,
    dm: DmService,
    media: MediaService,
    outbox: Outbox,
    ui: broadcast::Sender<UiEvent>,
    keys: Keys,
    /// Where a photo made smaller is written (`outgoing/` of the data folder).
    outgoing: PathBuf,
    /// See `PHOTO_SLOTS`; shared by every upload of the runtime.
    photo_slots: Arc<Semaphore>,
    /// See `Preparing`; shared by every upload of the runtime.
    preparing: Preparing,
}

/// The uploads in the preparing stage now, by transfer. Only one job
/// prepares a transfer: a pause and a resume while it waits for its slot
/// leave it to the job that waits already.
pub(crate) type Preparing = Arc<tokio::sync::Mutex<HashSet<String>>>;

/// What preparing an upload left to do.
#[derive(Debug, PartialEq, Eq)]
enum Prepared {
    /// The upload goes on with the file its row names now.
    GoOn,
    /// Paused or cancelled meanwhile: whoever did it told the placeholder.
    /// Or another job prepares it, and goes on with it.
    Stopped,
}

impl UploadJob {
    fn updated(&self, chat_id: &str, message_id: &str) {
        let _ = self.ui.send(updated_event(chat_id, message_id));
    }

    /// Before its first run a photo is made smaller (`smaller_photo`),
    /// unless the user asked for the original: the transfer is in the
    /// preparing stage meanwhile, and the transfer and the placeholder
    /// follow the new file. One resumed before it ran goes back to the
    /// queue first, so that a pause or a cancel meanwhile is heard. Only
    /// one job prepares a transfer (`Preparing`): another one steps aside,
    /// and the one that prepares it goes on with it.
    /// Whatever fails, the file goes as it is.
    async fn prepare(&self, transfer_id: &str, placeholder: &str, chat_id: &str, sink: &UiSink) -> Prepared {
        let Ok(Some(t)) = self.media.transfer(transfer_id).await else { return Prepared::GoOn };
        let Ok(Some(ph)) = self.dm.message(placeholder).await else { return Prepared::GoOn };
        let fields = ph.media.unwrap_or_else(|| serde_json::json!({}));
        let fields = self.follow_the_row(&t, placeholder, chat_id, fields).await;
        let photo = fields.get("kind").and_then(|v| v.as_str()) == Some(MediaKind::Image.as_str());
        let original = fields.get("original").and_then(|v| v.as_bool()) == Some(true);
        if !photo || original || t.attempts > 0 || t.done_bytes > 0 {
            return Prepared::GoOn;
        }
        {
            // Held while the row goes back to the queue: a job that stops
            // decides under it whether it prepares again.
            let mut preparing = self.preparing.lock().await;
            if t.status != ST_QUEUED && !self.media.queue_again(transfer_id).await.unwrap_or(false) {
                return Prepared::GoOn;
            }
            if !preparing.insert(transfer_id.to_string()) {
                // Another job prepares it: the bubble hears it is queued again.
                let _ = self.media.emit_stage(transfer_id, TransferStage::Preparing, sink).await;
                return Prepared::Stopped;
            }
        }
        let prepared = loop {
            let _ = self.media.emit_stage(transfer_id, TransferStage::Preparing, sink).await;
            let prepared = self.make_smaller(&t, placeholder, chat_id, fields.clone()).await;
            let mut preparing = self.preparing.lock().await;
            // Paused and resumed meanwhile: the job of the resume stepped
            // aside for this one, which prepares it again.
            if prepared == Prepared::Stopped && self.queued(transfer_id).await {
                continue;
            }
            preparing.remove(transfer_id);
            break prepared;
        };
        self.media.stage_over(transfer_id, TransferStage::Preparing);
        prepared
    }

    /// A picture's placeholder gets a preview made from the file that goes
    /// (`thumb_of`), unless it has one; the bubble is told.
    async fn add_thumb(&self, placeholder: &str, chat_id: &str, local_path: &str) {
        let Ok(Some(ph)) = self.dm.message(placeholder).await else { return };
        let Some(fields) = ph.media else { return };
        if fields.get("kind").and_then(|v| v.as_str()) != Some(MediaKind::Image.as_str()) || fields.get("thumb").is_some() {
            return;
        }
        let path = PathBuf::from(local_path);
        let Some(thumb) = tokio::task::spawn_blocking(move || thumb_of_file(&path)).await.ok().flatten() else { return };
        // The placeholder as it is now: the upload may have told it more.
        let Ok(Some(ph)) = self.dm.message(placeholder).await else { return };
        let mut fields = ph.media.unwrap_or_else(|| serde_json::json!({}));
        fields["thumb"] = thumb.into();
        if messenger_store::messages::set_media_json(self.dm.store(), placeholder, &fields.to_string()).await.is_ok() {
            self.updated(chat_id, placeholder);
        }
    }

    async fn queued(&self, transfer_id: &str) -> bool {
        matches!(self.media.transfer(transfer_id).await, Ok(Some(t)) if t.status == ST_QUEUED)
    }

    /// The row of the upload names another file than its placeholder: a
    /// photo was made smaller and the app ended before the placeholder was
    /// told. The placeholder follows the row, so the message says what is
    /// sent, and a copy the app made of the first file goes. Its fields
    /// as they are then.
    async fn follow_the_row(&self, t: &TransferView, placeholder: &str, chat_id: &str, fields: serde_json::Value) -> serde_json::Value {
        let Some(sent) = t.local_path.as_deref() else { return fields };
        let Some(named) = fields.get("local_path").and_then(|v| v.as_str()).map(String::from) else { return fields };
        if named == sent {
            return fields;
        }
        let dim = picture_size(Path::new(sent)).await;
        let fields = self.point_at(placeholder, chat_id, fields, t, dim).await;
        drop_own_copy(&self.outgoing, Path::new(&named)).await;
        fields
    }

    /// The placeholder names the file `t` sends now: its name, type, size,
    /// place and, for a picture, how large it is shown.
    async fn point_at(
        &self,
        placeholder: &str,
        chat_id: &str,
        mut fields: serde_json::Value,
        t: &TransferView,
        dim: Option<(u32, u32)>,
    ) -> serde_json::Value {
        fields["name"] = t.file_name.clone().into();
        fields["mime"] = t.mime.clone().into();
        fields["size"] = t.size.into();
        fields["local_path"] = t.local_path.clone().unwrap_or_default().into();
        if let Some((w, h)) = dim {
            fields["dim"] = serde_json::json!([w, h]);
        }
        let _ = messenger_store::messages::set_media_json(self.dm.store(), placeholder, &fields.to_string()).await;
        self.updated(chat_id, placeholder);
        fields
    }

    /// See `prepare`: the photo of `t` made smaller, when that is worth it.
    /// Only `PHOTO_SLOTS` photos at once: the others stay in the preparing
    /// stage meanwhile. A copy the app made of the photo goes once the new
    /// file takes its place.
    async fn make_smaller(&self, t: &TransferView, placeholder: &str, chat_id: &str, fields: serde_json::Value) -> Prepared {
        let transfer_id = t.id.as_str();
        let source = PathBuf::from(t.local_path.clone().unwrap_or_default());
        // Held until the new file is written.
        let Ok(slot) = self.photo_slots.clone().acquire_owned().await else { return Prepared::GoOn };
        // A pause or a cancel came while it waited; or the row names
        // another file already, and that one goes.
        match self.media.transfer(transfer_id).await {
            Ok(Some(now)) if now.status == ST_QUEUED && now.local_path == t.local_path => {}
            Ok(Some(now)) if now.status == ST_QUEUED => return Prepared::GoOn,
            _ => return Prepared::Stopped,
        }
        let made = {
            let source = source.clone();
            tokio::task::spawn_blocking(move || smaller_photo(&source)).await.ok().flatten()
        };
        // A pause or a cancel came meanwhile.
        match self.media.transfer(transfer_id).await {
            Ok(Some(now)) if now.status == ST_QUEUED => {}
            _ => return Prepared::Stopped,
        }
        let Some((jpeg, dim)) = made else { return Prepared::GoOn };
        let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.trim().is_empty());
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = self.outgoing.join(format!("{stamp:x}"));
        let path = dir.join(format!("{}.jpg", stem.as_deref().unwrap_or("photo")));
        let written = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(&path, &jpeg).await
        };
        if written.await.is_err() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Prepared::GoOn;
        }
        drop(slot);
        // The row first: should the app end before the placeholder is
        // told, the next run tells it (`follow_the_row`).
        let view = match self.media.replace_file(transfer_id, &path).await {
            Ok(Some(v)) => v,
            other => {
                let _ = tokio::fs::remove_dir_all(&dir).await;
                // Not queued any more: paused or cancelled meanwhile.
                return if matches!(other, Ok(None)) { Prepared::Stopped } else { Prepared::GoOn };
            }
        };
        self.point_at(placeholder, chat_id, fields, &view, Some(dim)).await;
        // Nothing refers to the full-size copy any more.
        drop_own_copy(&self.outgoing, &source).await;
        Prepared::GoOn
    }

    /// The message could not go out: the transfer has failed (a retry
    /// finds the chunks stored and tries again), and the placeholder says
    /// so.
    async fn not_published(&self, publishing: Publishing<'_>, placeholder: &str, chat_id: &str, e: MessengerError) {
        let _ = publishing.failed(&e).await;
        let _ = self.dm.media_set_status(placeholder, "failed", Some(&short_reason(&e))).await;
        let _ = self.ui.send(UiEvent {
            name: "error".into(),
            payload: serde_json::json!({ "family": "media", "error": e.to_string() }),
        });
        self.updated(chat_id, placeholder);
    }

    /// The message took the place of the placeholder: the transfer is
    /// done, and the message may go out. Not when a cancel took the
    /// transfer meanwhile (its chunks are being removed): then the message
    /// goes, as the placeholder would have. A row that could not be told
    /// is told at the next cancel or resume (`settle_sent`).
    async fn may_go_out(&self, publishing: Publishing<'_>, message_id: &str) -> bool {
        match publishing.published(Some(message_id)).await {
            Ok(true) => true,
            Ok(false) => {
                let _ = self.dm.delete_local(message_id).await;
                false
            }
            Err(e) => {
                eprintln!("messenger media: the upload of {message_id} was not marked done: {e}");
                true
            }
        }
    }

    /// The blob is stored and the chat is a group: the message goes out
    /// under the group key.
    async fn finish_in_group(
        &self,
        publishing: Publishing<'_>,
        placeholder: &str,
        chat_id: &str,
        envelope: messenger_core::Envelope,
        local: serde_json::Value,
    ) {
        let group_id = chat_id.trim_start_matches("group:").to_string();
        let lock = self.groups.lock_of(&group_id).await;
        let finished = {
            let _guard = lock.lock().await;
            self.groups.media_finish(&self.keys, placeholder, envelope, local).await
        };
        match finished {
            Ok((message, out)) => {
                if self.may_go_out(publishing, &message.id).await {
                    if let Ok(local_id) = self.outbox.enqueue_message(out).await {
                        let _ = self.dm.attach_outbox(&message.id, &local_id).await;
                    }
                    self.outbox.kick();
                }
                self.updated(chat_id, &message.id);
            }
            Err(e) => self.not_published(publishing, placeholder, chat_id, e).await,
        }
    }

    async fn run(self, transfer_id: String, placeholder: String, chat_id: String) {
        let sink = UiSink(self.ui.clone());
        if self.prepare(&transfer_id, &placeholder, &chat_id, &sink).await == Prepared::Stopped {
            return;
        }
        // The file that goes: a photo made smaller is another one.
        let local_path = match self.media.transfer(&transfer_id).await {
            Ok(Some(t)) => t.local_path.unwrap_or_default(),
            _ => String::new(),
        };
        // A picture's preview is made while the file goes up.
        let thumb = tokio::spawn({
            let job = self.clone();
            let (placeholder, chat_id, local_path) = (placeholder.clone(), chat_id.clone(), local_path.clone());
            async move { job.add_thumb(&placeholder, &chat_id, &local_path).await }
        });
        // The transfer ends once the message is out (the publishing stage).
        let outcome = self.media.upload_to_publish(&transfer_id, &self.keys, None, &sink).await;
        let _ = thumb.await;
        match outcome {
            Ok(Some(publishing)) => {
                let mut descriptor = publishing.descriptor().clone();
                // The caption lives on the placeholder row.
                let caption = self.dm.message(&placeholder).await.ok().flatten().and_then(|m| m.text);
                descriptor.caption = caption;
                // What the app knew before the upload (a recording's kind,
                // length and outline) is on the placeholder too.
                if let Some(ph) = self.dm.message(&placeholder).await.ok().flatten().and_then(|m| m.media) {
                    if let Some(k) = ph.get("kind").and_then(|v| v.as_str()).and_then(MediaKind::parse) {
                        descriptor.kind = k;
                    }
                    if let Some(m) = ph.get("mime").and_then(|v| v.as_str()) {
                        descriptor.mime = m.to_string();
                    }
                    descriptor.duration_ms = ph.get("duration_ms").and_then(|v| v.as_u64());
                    descriptor.batch = ph.get("batch").and_then(|v| v.as_str()).map(String::from);
                    descriptor.dim = ph.get("dim").and_then(|v| serde_json::from_value::<(u32, u32)>(v.clone()).ok());
                    descriptor.thumb = ph.get("thumb").and_then(|v| v.as_str()).filter(|t| valid_thumb(t)).map(String::from);
                    descriptor.waveform = ph
                        .get("waveform")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x.min(255) as u8).collect());
                }
                let envelope = descriptor.to_envelope();
                let mut local = serde_json::to_value(&descriptor).unwrap_or_else(|_| serde_json::json!({}));
                local["local_path"] = serde_json::Value::String(local_path);
                local["transfer_id"] = serde_json::Value::String(transfer_id.clone());
                if chat_id.starts_with("group:") {
                    self.finish_in_group(publishing, &placeholder, &chat_id, envelope, local).await;
                    return;
                }
                match self.dm.media_finish(&self.keys, &placeholder, envelope, local).await {
                    Ok(p) => {
                        let message_id = p.message.id.clone();
                        if self.may_go_out(publishing, &message_id).await {
                            if let Ok(local_id) = self.outbox.enqueue_message(p.to_peer).await {
                                let _ = self.dm.attach_outbox(&p.tracking_id, &local_id).await;
                            }
                            if let Some(own) = p.to_self {
                                let _ = self.outbox.enqueue_message(own).await;
                            }
                            self.outbox.kick();
                            if let Ok(events) = self.dm.sync_statuses().await {
                                for ev in events {
                                    let _ = self.ui.send(ev);
                                }
                            }
                        }
                        self.updated(&chat_id, &message_id);
                    }
                    Err(e) => self.not_published(publishing, &placeholder, &chat_id, e).await,
                }
            }
            Ok(None) => {
                // Paused or cancelled; the transfer row says which.
                let cancelled = matches!(
                    self.media.transfer(&transfer_id).await,
                    Ok(Some(t)) if t.status == "cancelled"
                );
                if cancelled {
                    let _ = self.dm.media_discard(&placeholder).await;
                    drop_own_copy(&self.outgoing, Path::new(&local_path)).await;
                } else {
                    let _ = self.dm.media_set_status(&placeholder, "paused", None).await;
                }
                self.updated(&chat_id, &placeholder);
            }
            // Another run of it is under way: the placeholder is that one's.
            Err(e) if short_reason(&e) == "err.transfer_in_progress" => {}
            Err(e) => {
                let reason = self
                    .media
                    .transfer(&transfer_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|t| t.failure_reason)
                    .unwrap_or_else(|| short_reason(&e));
                let _ = self.dm.media_set_status(&placeholder, "failed", Some(&reason)).await;
                self.updated(&chat_id, &placeholder);
            }
        }
    }
}

/// `dest` with the extension of `src` when it has none of its own.
fn with_source_extension(dest: &Path, src: &Path) -> PathBuf {
    match (dest.extension(), src.extension()) {
        (None, Some(ext)) if dest.file_name().is_some() => {
            let mut name = dest.as_os_str().to_owned();
            name.push(".");
            name.push(ext);
            PathBuf::from(name)
        }
        _ => dest.to_path_buf(),
    }
}

/// Copy `src` to `dest`, which must not exist yet (`AlreadyExists` then);
/// a copy that fails half way is removed.
fn copy_to_new(src: &Path, dest: &Path) -> std::io::Result<()> {
    let mut from = std::fs::File::open(src)?;
    let mut to = std::fs::OpenOptions::new().write(true).create_new(true).open(dest)?;
    let copied = std::io::copy(&mut from, &mut to).and_then(|_| to.sync_all());
    if copied.is_err() {
        drop(to);
        let _ = std::fs::remove_file(dest);
    }
    copied
}

impl MessengerRuntime {
    pub fn media(&self) -> &MediaService {
        &self.media
    }

    /// Media servers of the manifest in use for the current region, when the
    /// Veydan servers are chosen; none otherwise. Credentials are never in a
    /// manifest; the user adds them. A server an earlier manifest brought and
    /// this one no longer lists is removed; the user's own servers stay.
    pub(crate) async fn seed_media_servers(&self) -> Result<()> {
        let veydan = self.relays.servers_mode().await? == Some(ServersMode::Veydan);
        let (manifest, _) = self.relays.current_manifest().await?;
        let region = self.relays.region().await.unwrap_or_else(|_| REGION_FALLBACK.into());
        let listed = if veydan { manifest.media_for_region(&region) } else { Vec::new() };
        let before = self.media.servers().await?;
        for old in &before {
            if old.source == "manifest" && !listed.iter().any(|m| m.id == old.id) {
                self.media.remove_server(&old.id).await?;
            }
        }
        for (i, m) in listed.into_iter().enumerate() {
            self.media
                .put_server(MediaServerInput {
                    id: Some(m.id.clone()),
                    kind: m.kind.clone(),
                    url: m.url.clone(),
                    bucket: m.bucket.clone(),
                    region: m.s3_region.clone(),
                    access_key: None,
                    secret_key: None,
                    priority: Some(10 + i as i64),
                    source: Some("manifest".into()),
                })
                .await?;
        }
        // The keeper of my avatar looks again when the servers changed.
        if self.media.servers().await? != before {
            self.avatars.kick();
        }
        Ok(())
    }

    pub async fn media_servers(&self) -> Result<Vec<MediaServerView>> {
        self.media.servers().await
    }

    /// The keeper of my avatar looks again after each change of the servers.
    pub async fn media_server_put(&self, input: MediaServerInput) -> Result<MediaServerView> {
        let view = self.media.put_server(input).await?;
        self.avatars.kick();
        Ok(view)
    }

    pub async fn media_server_remove(&self, id: &str) -> Result<()> {
        self.media.remove_server(id).await?;
        self.avatars.kick();
        Ok(())
    }

    pub async fn media_server_set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.media.set_server_enabled(id, enabled).await?;
        self.avatars.kick();
        Ok(())
    }

    /// Verify credentials, create the bucket if needed, write and read a
    /// probe blob.
    pub async fn media_server_check(&self, id: &str) -> Result<()> {
        let keys = self.session_keys().await?;
        self.media.check_server(id, &keys).await
    }

    fn upload_job(&self, keys: Keys) -> UploadJob {
        UploadJob {
            groups: self.group_driver.groups.clone(),
            dm: self.dm.clone(),
            media: self.media.clone(),
            outbox: self.outbox.clone(),
            ui: self.ui.clone(),
            keys,
            outgoing: self.outgoing(),
            photo_slots: self.photo_slots.clone(),
            preparing: self.preparing.clone(),
        }
    }

    /// Where the app keeps the files it sends from: copies of what was
    /// picked on Android, recordings, photos made smaller.
    fn outgoing(&self) -> PathBuf {
        self.config.data_dir().join("outgoing")
    }

    fn fetch_job(&self) -> FetchJob {
        FetchJob { dm: self.dm.clone(), media: self.media.clone(), ui: self.ui.clone() }
    }

    /// Attach a file to the chat with `to` (a person, or `group:<id>`).
    /// Returns the placeholder message at once; the upload continues in
    /// the background. A photo is made smaller first unless `original`.
    pub async fn dm_send_file(
        &self,
        to: &str,
        path: &Path,
        caption: Option<&str>,
        batch: Option<&str>,
        original: bool,
    ) -> Result<MessageView> {
        self.dm_send_file_with(to, path, caption, batch, original, None).await
    }

    /// `dm_send_file` with a frame the UI took from a video (`Poster`):
    /// written anew as the preview the other side sees before it fetches
    /// the file. A frame that cannot be read is left out.
    pub async fn dm_send_file_with(
        &self,
        to: &str,
        path: &Path,
        caption: Option<&str>,
        batch: Option<&str>,
        original: bool,
        poster: Option<Poster>,
    ) -> Result<MessageView> {
        if batch.is_some_and(|b| !messenger_media::descriptor::valid_batch(b)) {
            return Err(MessengerError::Invalid("err.bad_batch".into()));
        }
        self.send_attachment(to, path, caption, None, batch, original, poster).await
    }

    /// Send something recorded in the app (voice message, video circle).
    /// The bytes are written to the messenger folder and sent from there.
    pub async fn dm_send_recording(&self, to: &str, rec: Recording, caption: Option<&str>) -> Result<MessageView> {
        if rec.bytes.is_empty() {
            return Err(MessengerError::Invalid("the recording is empty".into()));
        }
        if rec.bytes.len() as u64 > MAX_RECORDING_BYTES {
            return Err(MessengerError::Invalid("err.file_too_large".into()));
        }
        if !matches!(rec.kind, MediaKind::Voice | MediaKind::Circle) {
            return Err(MessengerError::Invalid("a recording is a voice message or a circle".into()));
        }
        let mime = base_mime(&rec.mime);
        let ok = match rec.kind {
            MediaKind::Voice => matches!(mime.as_str(), "audio/webm" | "audio/ogg" | "audio/mp4"),
            _ => matches!(mime.as_str(), "video/webm" | "video/mp4"),
        };
        if !ok {
            return Err(MessengerError::Invalid(format!("unsupported recording type {mime}")));
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let ext = match mime.as_str() {
            "audio/webm" => "weba",
            "audio/ogg" => "ogg",
            "audio/mp4" => "m4a",
            "video/mp4" => "mp4",
            _ => "webm",
        };
        let dir = self.config.data_dir().join("outgoing").join(format!("{stamp:x}"));
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{}.{ext}", rec.kind.as_str()));
        tokio::fs::write(&path, &rec.bytes).await?;
        let waveform = rec.waveform.map(|w| w.into_iter().take(messenger_media::descriptor::MAX_WAVEFORM).collect::<Vec<u8>>());
        let meta = AttachmentMeta { kind: rec.kind, mime, duration_ms: rec.duration_ms, waveform };
        let result = self.send_attachment(to, &path, caption, Some(meta), None, false, None).await;
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_attachment(
        &self,
        to: &str,
        path: &Path,
        caption: Option<&str>,
        meta_override: Option<AttachmentMeta>,
        batch: Option<&str>,
        original: bool,
        poster: Option<Poster>,
    ) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let group = to.strip_prefix("group:").map(String::from);
        let peer = match &group {
            Some(_) => None,
            None => Some(messenger_contacts::book::parse_key(to)?),
        };
        // Refuse early: no server, no upload; a file that cannot be sent
        // never shows as a message.
        self.media.upload_backend(&keys).await?;
        let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
        sendable(&meta)?;
        let name = messenger_media::descriptor::safe_name(
            &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        );
        let mut fields = serde_json::json!({
            "name": name,
            "mime": messenger_media::descriptor::mime_for(&name),
            "size": meta.len(),
            "kind": MediaKind::from_mime(messenger_media::descriptor::mime_for(&name)).as_str(),
            "local_path": path.to_string_lossy(),
        });
        if let Some(m) = meta_override {
            fields["kind"] = m.kind.as_str().into();
            fields["mime"] = m.mime.into();
            if let Some(d) = m.duration_ms {
                fields["duration_ms"] = d.into();
            }
            if let Some(w) = m.waveform {
                fields["waveform"] = serde_json::json!(w);
            }
        }
        // A picture says how large it is shown, so the other side keeps
        // its place before the file is there; one sent as it is says so,
        // for the upload not to make it smaller.
        if fields["kind"] == MediaKind::Image.as_str() {
            if let Some((w, h)) = picture_size(path).await {
                fields["dim"] = serde_json::json!([w, h]);
            }
            if original {
                fields["original"] = true.into();
            }
        }
        // A video shows the frame the UI took, and keeps its place.
        if fields["kind"] == MediaKind::Video.as_str() {
            if let Some(p) = poster.filter(|p| p.jpeg.len() <= MAX_POSTER_B64) {
                let jpeg = B64.decode(p.jpeg.trim()).unwrap_or_default();
                if let Some(thumb) = tokio::task::spawn_blocking(move || thumb_of(&jpeg)).await.ok().flatten() {
                    fields["thumb"] = thumb.into();
                }
                if (1..=16384).contains(&p.width) && (1..=16384).contains(&p.height) {
                    fields["dim"] = serde_json::json!([p.width, p.height]);
                }
            }
        }
        // Files picked together stay together on the other side.
        if let Some(b) = batch {
            fields["batch"] = b.into();
        }
        let placeholder = match (&group, &peer) {
            (Some(g), _) => self.groups().media_placeholder(&keys, g, fields, caption).await?,
            (None, Some(peer)) => self.dm.media_placeholder(&keys, peer, fields, caption).await?,
            (None, None) => return Err(MessengerError::Invalid("no recipient".into())),
        };
        let transfer = match self.media.queue_upload(path, &placeholder.chat_id, &placeholder.id).await {
            Ok(t) => t,
            Err(e) => {
                let _ = self.dm.media_discard(&placeholder.id).await;
                return Err(e);
            }
        };
        let mut fields = placeholder.media.clone().unwrap_or_else(|| serde_json::json!({}));
        fields["transfer_id"] = serde_json::Value::String(transfer.id.clone());
        messenger_store::messages::set_media_json(&self.store, &placeholder.id, &fields.to_string()).await?;

        let job = self.upload_job(keys);
        tokio::spawn(job.run(transfer.id, placeholder.id.clone(), placeholder.chat_id.clone()));
        Ok(self.dm.message(&placeholder.id).await?.unwrap_or(placeholder))
    }

    async fn descriptor_of(&self, message_id: &str) -> Result<(MessageView, MediaDescriptor)> {
        let m = self.dm.message(message_id).await?.ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        let fields = m.media.clone().ok_or_else(|| MessengerError::Invalid("the message has no attachment".into()))?;
        let d = MediaDescriptor::from_fields(&fields)?;
        Ok((m, d))
    }

    /// Path of the attachment on this device, if it is here: the original
    /// file for what we sent, the cache for what we received.
    pub async fn media_local_path(&self, message_id: &str) -> Result<Option<PathBuf>> {
        let Some(m) = self.dm.message(message_id).await? else { return Ok(None) };
        let Some(fields) = m.media else { return Ok(None) };
        if let Some(p) = fields.get("local_path").and_then(|v| v.as_str()) {
            if tokio::fs::metadata(p).await.map(|x| x.is_file()).unwrap_or(false) {
                return Ok(Some(PathBuf::from(p)));
            }
        }
        match MediaDescriptor::from_fields(&fields) {
            Ok(d) => Ok(self.media.cached(&d).await),
            Err(_) => Ok(None),
        }
    }

    /// Fetch the attachment of a message. `manual = false` is the
    /// automatic path: it respects the size limit, earlier failures and a
    /// previous cancel, and answers `None` when it decides not to start.
    pub async fn media_download(&self, message_id: &str, manual: bool) -> Result<Option<PathBuf>> {
        if let Some(p) = self.media_local_path(message_id).await? {
            return Ok(Some(p));
        }
        let (m, d) = self.descriptor_of(message_id).await?;
        if !manual {
            if d.size > AUTO_DOWNLOAD_BYTES || !self.media.may_auto_download(message_id).await? {
                return Ok(None);
            }
            // Nothing is fetched on behalf of someone I block.
            if let Some(peer) = m.chat_id.strip_prefix("dm:").and_then(PubKey::parse) {
                if self.dm.is_blocked(&peer).await? {
                    return Ok(None);
                }
            }
        }
        self.fetch_job().run(message_id.to_string(), m.chat_id, d, manual).await
    }

    /// The transfer of a message (upload for ours, download for theirs).
    pub async fn media_transfer(&self, message_id: &str) -> Result<Option<TransferView>> {
        match self.media.transfer_for_message(message_id, DIR_UP).await? {
            Some(t) => Ok(Some(t)),
            None => self.media.transfer_for_message(message_id, DIR_DOWN).await,
        }
    }

    /// The bubble and the list of transfers hear the row of a transfer as
    /// it is now: no run tells them what was done to it here.
    async fn tell(&self, transfer_id: &str) -> Result<()> {
        if let Some(t) = self.media.transfer(transfer_id).await? {
            UiSink(self.ui.clone()).progress(progress_of(&t));
        }
        Ok(())
    }

    /// A run pauses itself and tells it. A transfer whose run is gone is
    /// paused here: its bubble is told, and the placeholder of an upload
    /// says paused.
    pub async fn media_pause(&self, transfer_id: &str) -> Result<()> {
        if self.media.pause(transfer_id).await? != Paused::Here {
            return Ok(());
        }
        let Some(t) = self.media.transfer(transfer_id).await? else { return Ok(()) };
        UiSink(self.ui.clone()).progress(progress_of(&t));
        if let Some(mid) = t.message_id.as_deref().filter(|m| t.direction == DIR_UP && m.starts_with("local:")) {
            self.dm.media_set_status(mid, "paused", None).await?;
            let _ = self.ui.send(UiEvent {
                name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
                payload: serde_json::json!({ "chat_id": t.chat_id, "message_id": mid }),
            });
        }
        Ok(())
    }

    /// An upload whose message went out before its row was told it is
    /// done: the row names that message, or a placeholder that is gone (its
    /// message took its place; the app ended before the row was told).
    /// Never cancelled nor sent again: its chunks are the message's.
    /// Marked done, where nothing runs it, and the bubble is told. Whether
    /// it is one.
    async fn settle_sent(&self, t: &TransferView) -> Result<bool> {
        let Some(named) = t.message_id.as_deref().filter(|_| t.direction == DIR_UP) else { return Ok(false) };
        if named.starts_with("local:") && self.dm.message(named).await?.is_some() {
            return Ok(false);
        }
        self.media.mark_published(&t.id, named).await?;
        self.tell(&t.id).await?;
        Ok(true)
    }

    /// A run cancels itself and tells it. A transfer nothing runs (paused,
    /// failed, or a photo still being made smaller) is cancelled here: its
    /// bubble and the list are told, and the placeholder of an upload goes.
    pub async fn media_cancel(&self, transfer_id: &str) -> Result<()> {
        let Some(t) = self.media.transfer(transfer_id).await? else { return Ok(()) };
        if self.settle_sent(&t).await? {
            return Ok(());
        }
        // The keys let the chunks of an upload go from a Blossom server too.
        let keys = self.session_keys().await.ok();
        let unsent = t.message_id.as_deref().filter(|m| t.direction == DIR_UP && t.status == "done" && m.starts_with("local:"));
        // Stored, but its message never went out: its chunks go too. The
        // media service decides on the row as it is then, not on this read:
        // only while it still names this placeholder.
        let outcome = if let Some(placeholder) = unsent {
            self.media.discard_unpublished(transfer_id, placeholder, keys.as_ref()).await?
        } else {
            self.media.cancel_with(transfer_id, keys.as_ref()).await?
        };
        // A run discards its placeholder itself; a transfer cancelled here
        // has nobody else to. Its message is read again: only a placeholder
        // that is one now goes.
        if outcome != Cancelled::Here {
            return Ok(());
        }
        self.tell(transfer_id).await?;
        if let Some(t) = self.media.transfer(transfer_id).await?.filter(|t| t.direction == DIR_UP) {
            if let Some(mid) = t.message_id.filter(|m| m.starts_with("local:")) {
                self.dm.media_discard(&mid).await?;
                // A copy the app made to send it has no use any more.
                if let Some(p) = t.local_path.as_deref() {
                    drop_own_copy(&self.outgoing(), Path::new(p)).await;
                }
                let _ = self.ui.send(UiEvent {
                    name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
                    payload: serde_json::json!({ "chat_id": t.chat_id, "message_id": mid }),
                });
            }
        }
        Ok(())
    }

    /// The message or the chat of these transfers is being deleted: those
    /// that are not over are cancelled, so that nothing is left of them
    /// (chunks, folders, a row in the list). One that cannot be cancelled
    /// now is left; the list does not show it once its message is gone.
    async fn cancel_all(&self, list: Vec<TransferView>) {
        for t in list.into_iter().filter(|t| NOT_OVER.contains(&t.status.as_str())) {
            if let Err(e) = self.media_cancel(&t.id).await {
                eprintln!("messenger media: the transfer {} of a deleted message was not cancelled: {e}", t.id);
            }
        }
    }

    /// Before the message goes from this device: its transfers end.
    pub(crate) async fn cancel_transfers_of_message(&self, message_id: &str) {
        let mut list = Vec::new();
        for dir in [DIR_UP, DIR_DOWN] {
            if let Ok(Some(t)) = self.media.transfer_for_message(message_id, dir).await {
                list.push(t);
            }
        }
        self.cancel_all(list).await;
    }

    /// Before the chat goes from this device: the transfers of its messages end.
    pub(crate) async fn cancel_transfers_of_chat(&self, chat_id: &str) {
        let Ok(list) = self.media.active_transfers().await else { return };
        self.cancel_all(list.into_iter().filter(|t| t.chat_id.as_deref() == Some(chat_id)).collect()).await;
    }

    /// Continue a paused or failed transfer; one waiting for its next
    /// automatic attempt makes it now.
    pub async fn media_resume(&self, transfer_id: &str) -> Result<()> {
        let t = self.media.transfer(transfer_id).await?.ok_or_else(|| MessengerError::Invalid("unknown transfer".into()))?;
        // A row under way whose run is gone (the app was killed while the
        // CLI had the data folder open) is taken over by a new run.
        if (t.status == "running" || t.status == "queued") && self.media.run_alive(transfer_id) {
            return Ok(());
        }
        if t.status == "waiting_retry" && self.media.retry_now(transfer_id) {
            return Ok(());
        }
        let message_id = t.message_id.clone().ok_or_else(|| MessengerError::Invalid("transfer has no message".into()))?;
        if t.direction == DIR_UP {
            if self.settle_sent(&t).await? {
                return Ok(()); // already sent
            }
            let keys = self.session_keys().await?;
            self.upload_again(&t, &message_id, keys).await
        } else {
            self.download_again(&message_id).await
        }
    }

    /// The download of `message_id` runs again in the background, as the
    /// user asked for it: only the chunks not on disk yet are fetched. Its
    /// progress and its end are told by its events.
    async fn download_again(&self, message_id: &str) -> Result<()> {
        if self.media_local_path(message_id).await?.is_some() {
            return Ok(());
        }
        let (m, d) = self.descriptor_of(message_id).await?;
        let job = self.fetch_job();
        let message_id = message_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = job.run(message_id.clone(), m.chat_id, d, true).await {
                eprintln!("messenger media: the download of {message_id} did not go on: {e}");
            }
        });
        Ok(())
    }

    /// The upload of `placeholder` runs again in the background: only the
    /// chunks not stored yet are sent.
    async fn upload_again(&self, t: &TransferView, placeholder: &str, keys: Keys) -> Result<()> {
        let chat_id = t.chat_id.clone().unwrap_or_default();
        self.dm.media_set_status(placeholder, "uploading", None).await?;
        let _ = self.ui.send(updated_event(&chat_id, placeholder));
        tokio::spawn(self.upload_job(keys).run(t.id.clone(), placeholder.to_string(), chat_id));
        Ok(())
    }

    /// Start `t` again in the background, as the user would: an upload
    /// whose message has not gone out (its placeholder is here), or a
    /// download as if the user asked for it. Not one that runs anywhere,
    /// nor an upload while no session gives the keys, nor one another
    /// identity left (its placeholder is not signed with the keys of this
    /// session). Whether it started.
    async fn take_up(&self, t: &TransferView) -> Result<bool> {
        if self.media.run_alive(&t.id) {
            return Ok(false);
        }
        let Some(message_id) = t.message_id.clone() else { return Ok(false) };
        if t.direction == DIR_UP {
            if !message_id.starts_with("local:") || self.settle_sent(t).await? {
                return Ok(false);
            }
            let Ok(keys) = self.session_keys().await else { return Ok(false) };
            let mine = self.dm.message(&message_id).await?.is_some_and(|m| m.sender_pubkey == keys.public_key().to_hex());
            if !mine {
                return Ok(false);
            }
            self.upload_again(t, &message_id, keys).await?;
            return Ok(true);
        }
        // A file in the cache already ends its row at once.
        let Ok((m, d)) = self.descriptor_of(&message_id).await else { return Ok(false) };
        let job = self.fetch_job();
        tokio::spawn(async move {
            if let Err(e) = job.run(message_id.clone(), m.chat_id, d, true).await {
                eprintln!("messenger media: the download of {message_id} did not go on: {e}");
            }
        });
        Ok(true)
    }

    /// Every transfer the user sees in the list of transfers: queued,
    /// running, waiting for its next attempt, paused or failed, whose
    /// message is still here (not one of a chat or a message deleted
    /// since). The newest first.
    pub async fn media_transfers(&self) -> Result<Vec<TransferView>> {
        let mut out = Vec::new();
        for t in self.media.active_transfers().await? {
            let Some(mid) = t.message_id.as_deref() else { continue };
            if messenger_store::messages::get(&self.store, mid).await?.is_some_and(|m| m.deleted_at.is_none()) {
                out.push(t);
            }
        }
        Ok(out)
    }

    /// "Retry all": every transfer of the list that failed, or that the
    /// closing of the app paused, starts again (uploads whose message has
    /// not gone out, and downloads as if the user asked); one waiting for
    /// its next attempt makes it now. A pause the user made stays. How
    /// many.
    pub async fn media_retry_failed(&self) -> Result<u32> {
        let mut n = 0;
        for t in self.media_transfers().await? {
            let interrupted = t.status == ST_PAUSED && t.failure_reason.as_deref() == Some(REASON_INTERRUPTED);
            let started = if t.status == ST_WAITING_RETRY {
                self.media.retry_now(&t.id)
            } else if t.status == ST_FAILED || interrupted {
                self.take_up(&t).await?
            } else {
                false
            };
            n += u32::from(started);
        }
        Ok(n)
    }

    /// Transfers the closing of the app interrupted start again by
    /// themselves (`MediaService::interrupted`): an upload whose
    /// placeholder is still here, a download the user started. Never one
    /// the user paused, nor one another process runs. Called once a
    /// session runs; how many started.
    pub(crate) async fn resume_interrupted(&self) -> Result<u32> {
        let mut n = 0;
        for t in self.media.interrupted().await? {
            n += u32::from(self.take_up(&t).await?);
        }
        Ok(n)
    }

    /// The relays came back after none was connected: the network is back.
    /// A transfer waiting for its next attempt makes it now, and one whose
    /// automatic attempts ran out while the connection was lost starts
    /// again (`LOST_CONNECTION`). Other failures stay for the user.
    pub(crate) async fn media_network_back(&self) {
        let Ok(list) = self.media_transfers().await else { return };
        for t in list {
            if t.status == ST_WAITING_RETRY {
                self.media.retry_now(&t.id);
            } else if t.status == ST_FAILED && t.failure_reason.as_deref().is_some_and(|r| LOST_CONNECTION.contains(&r)) {
                let _ = self.take_up(&t).await;
            }
        }
    }

    /// Copy the attachment somewhere the user chose. A name chosen without
    /// an extension gets the file's own: a dialog may drop it, and a photo
    /// without it opens nowhere. The dialog asked about overwriting only
    /// the name it returned, so the name with the extension is written
    /// only when no file has it yet; else the copy goes to `dest` as chosen.
    pub async fn media_save_as(&self, message_id: &str, dest: &Path) -> Result<()> {
        let src = self
            .media_local_path(message_id)
            .await?
            .ok_or_else(|| MessengerError::Invalid("err.not_downloaded".into()))?;
        let target = with_source_extension(dest, &src);
        if target != dest {
            let (from, to) = (src.clone(), target);
            let made = tokio::task::spawn_blocking(move || copy_to_new(&from, &to))
                .await
                .map_err(std::io::Error::other)?;
            match made {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        tokio::fs::copy(&src, dest).await?;
        Ok(())
    }

    /// The attachment as a `data:` url for inline previews.
    /// A file the user picked, as the composer shows it before it is sent:
    /// its name, what it will be sent as, and a picture as a `data:` url.
    /// The host checks first that the user did pick this path. A file that
    /// cannot be sent (over `MAX_SEND_BYTES`, empty, not a file) is refused
    /// here already.
    pub async fn picked(&self, path: &Path) -> Result<PickedView> {
        let name = messenger_media::descriptor::safe_name(
            &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        );
        let mime = messenger_media::descriptor::mime_for(&name);
        let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
        sendable(&meta)?;
        let picture = matches!(mime, "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/avif" | "image/bmp");
        let preview = if picture && meta.is_file() && meta.len() <= MAX_INLINE_BYTES {
            Some(format!("data:{mime};base64,{}", B64.encode(tokio::fs::read(path).await?)))
        } else {
            None
        };
        Ok(PickedView {
            path: path.to_string_lossy().into_owned(),
            name,
            kind: MediaKind::from_mime(mime).as_str().into(),
            mime: mime.into(),
            size: meta.len(),
            preview,
        })
    }

    pub async fn media_data_url(&self, message_id: &str) -> Result<Option<String>> {
        let Some(path) = self.media_local_path(message_id).await? else { return Ok(None) };
        let meta = tokio::fs::metadata(&path).await?;
        if meta.len() > MAX_INLINE_BYTES {
            return Err(MessengerError::Invalid("err.too_large_for_preview".into()));
        }
        let mime = self.media_mime(message_id).await?;
        if !passive(&mime) {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await?;
        Ok(Some(format!("data:{mime};base64,{}", B64.encode(bytes))))
    }

    /// The attachment of a message on this device and the type it is
    /// shown as, when a webview shows that type passively (`passive`):
    /// for the host to let the webview read the file by itself.
    pub async fn media_playable(&self, message_id: &str) -> Result<Option<(PathBuf, String)>> {
        let Some(path) = self.media_local_path(message_id).await? else { return Ok(None) };
        let mime = self.media_mime(message_id).await?;
        Ok(passive(&mime).then_some((path, mime)))
    }

    async fn media_mime(&self, message_id: &str) -> Result<String> {
        let m = self.dm.message(message_id).await?.and_then(|m| m.media);
        let mime = m.as_ref().and_then(|f| f.get("mime")).and_then(|v| v.as_str()).unwrap_or("application/octet-stream");
        Ok(base_mime(mime))
    }
}

/// Types a webview renders passively: pictures, videos, sounds; never
/// HTML or SVG.
pub fn passive(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/avif" | "image/bmp"
            | "video/mp4" | "video/webm" | "video/quicktime"
            | "audio/mpeg" | "audio/ogg" | "audio/wav" | "audio/mp4" | "audio/flac" | "audio/webm"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::MessengerConfig;
    use messenger_store::media as transfers;
    use messenger_store::messages::{self, NewMessage};
    use messenger_testkit::MemorySecretStore;
    use std::sync::Arc;

    async fn runtime(cfg: &MessengerConfig, secrets: &Arc<MemorySecretStore>) -> MessengerRuntime {
        let rt = MessengerRuntime::start(cfg.clone(), secrets.clone()).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        rt
    }

    /// The placeholder of an upload, as the chat shows it while it runs.
    async fn placeholder(rt: &MessengerRuntime, n: i64) -> String {
        let chat = messenger_store::chats::ensure_dm(&rt.store, &"b".repeat(64)).await.unwrap();
        let id = format!("local:test-{n}");
        let m = NewMessage {
            id: id.clone(),
            chat_id: chat.id,
            wire_id: None,
            direction: messages::DIR_OUT.into(),
            status: messages::STATUS_UPLOADING.into(),
            content_type: messages::CT_MEDIA.into(),
            text: None,
            envelope_json: "{}".into(),
            sender_pubkey: "a".repeat(64),
            reply_to_id: None,
            target_id: None,
            created_at: n,
            is_hidden: false,
            outbox_local_id: None,
            media_json: Some("{}".into()),
        };
        messages::insert(&rt.store, &m).await.unwrap();
        id
    }

    /// An upload of `path` for `placeholder`, its row in `status`.
    async fn upload(rt: &MessengerRuntime, path: &Path, placeholder: &str, status: &str) -> String {
        let t = rt.media.queue_upload(path, "dm:x", placeholder).await.unwrap();
        transfers::set_status(&rt.store, &t.id, status, None).await.unwrap();
        t.id
    }

    /// The message of an upload went out and the app ended before its row
    /// was told: the row names a placeholder that is gone. A cancel never
    /// removes the chunks of the message, and a resume never sends it
    /// again: the transfer is done.
    #[tokio::test]
    async fn an_upload_whose_message_went_out_is_never_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"0123456789").await.unwrap();

        let mut sent = Vec::new();
        for n in 0..2 {
            let ph = placeholder(&rt, n).await;
            sent.push(upload(&rt, &path, &ph, transfers::ST_PAUSED).await);
            messages::delete(&rt.store, &ph).await.unwrap();
        }
        // The row names the message itself, but could not be marked done
        // as it went out.
        for n in 0..2 {
            sent.push(upload(&rt, &path, &format!("msg-{n}"), transfers::ST_RUNNING).await);
        }
        rt.media_cancel(&sent[0]).await.unwrap();
        rt.media_resume(&sent[1]).await.unwrap();
        rt.media_cancel(&sent[2]).await.unwrap();
        rt.media_resume(&sent[3]).await.unwrap();
        for id in &sent {
            assert_eq!(rt.media.transfer(id).await.unwrap().unwrap().status, "done");
        }

        // One whose placeholder is there is cancelled, placeholder and all.
        let ph = placeholder(&rt, 2).await;
        let id = upload(&rt, &path, &ph, transfers::ST_PAUSED).await;
        rt.media_cancel(&id).await.unwrap();
        assert_eq!(rt.media.transfer(&id).await.unwrap().unwrap().status, "cancelled");
        assert!(rt.dm.message(&ph).await.unwrap().is_none());
        rt.shutdown().await;
    }

    /// An upload the CLI ran beside the app, and the CLI was killed: no run
    /// is left anywhere. A pause marks it paused here, and its bubble and
    /// its placeholder are told.
    #[tokio::test]
    async fn an_upload_whose_run_is_gone_is_paused_here() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"0123456789").await.unwrap();
        let ph = placeholder(&rt, 0).await;
        let id = upload(&rt, &path, &ph, transfers::ST_RUNNING).await;
        let mut events = rt.ui.subscribe();

        rt.media_pause(&id).await.unwrap();
        assert_eq!(rt.media.transfer(&id).await.unwrap().unwrap().status, "paused");
        assert_eq!(rt.dm.message(&ph).await.unwrap().unwrap().status, "paused");
        let mut told = Vec::new();
        while let Ok(ev) = events.try_recv() {
            told.push((ev.name, ev.payload["status"].as_str().map(String::from)));
        }
        assert!(told.contains(&(UI_EVENT_TRANSFER.to_string(), Some("paused".into()))), "{told:?}");
        assert!(told.iter().any(|(name, _)| name == messenger_dm::UI_EVENT_DM_UPDATED), "{told:?}");
        rt.shutdown().await;
    }

    /// Started beside another process with the data folder (the CLI): the
    /// placeholder of an upload it runs goes on, every other one pauses,
    /// also one whose transfer was never queued.
    #[tokio::test]
    async fn beside_another_process_only_its_uploads_go_on() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = runtime(&cfg, &secrets).await;
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"0123456789").await.unwrap();
        // Killed before its upload was queued.
        let lost = placeholder(&rt, 0).await;
        // One the other process runs now, and one whose run is gone.
        let (theirs, gone) = (placeholder(&rt, 1).await, placeholder(&rt, 2).await);
        let running = upload(&rt, &path, &theirs, transfers::ST_RUNNING).await;
        upload(&rt, &path, &gone, transfers::ST_RUNNING).await;
        rt.shutdown().await;

        // The other process holds the data folder and the run of `running`.
        let media_dir = cfg.data_dir().join("media");
        let folder = std::fs::File::open(media_dir.join("transfers.lock")).unwrap();
        folder.lock_shared().unwrap();
        let run = messenger_media::runs::take(&media_dir, &running).await.unwrap().unwrap();

        let rt = runtime(&cfg, &secrets).await;
        let status = async |id: &str| rt.dm.message(id).await.unwrap().unwrap().status;
        assert_eq!(status(&theirs).await, "uploading", "its run lives there");
        assert_eq!(status(&gone).await, "paused");
        assert_eq!(status(&lost).await, "paused", "nothing runs it");
        drop(run);
        rt.shutdown().await;
    }

    /// Reads the blobs of a memory backend as a server would hand them out.
    struct MemFetch(MemoryBackend);

    #[async_trait::async_trait]
    impl messenger_media::download::BlobFetcher for MemFetch {
        async fn fetch(&self, url: &str, _max: u64) -> Result<Option<Vec<u8>>> {
            Ok(url.rsplit('/').next().and_then(|sha| self.0.get(sha)))
        }
    }

    use messenger_media::MemoryBackend;

    /// Its files go to `backend` and come from there; any peer may get them.
    fn use_memory(rt: &mut MessengerRuntime, backend: &MemoryBackend) {
        rt.dm().set_gate(false);
        rt.media = rt.media.clone().with_backend(Arc::new(backend.clone()), Arc::new(MemFetch(backend.clone())));
    }

    /// A session with a new identity.
    async fn with_session(rt: &MessengerRuntime) {
        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
    }

    /// Wait (up to twenty seconds) until `check` holds.
    async fn until(mut check: impl AsyncFnMut() -> bool) {
        for _ in 0..2000 {
            if check().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the condition never held");
    }

    async fn status_of(rt: &MessengerRuntime, transfer: &str) -> String {
        rt.media.transfer(transfer).await.unwrap().unwrap().status
    }

    /// A picture of `w` by `h` with fine detail, so that it takes room.
    fn picture(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, (x.wrapping_mul(y) >> 3) as u8]))
    }

    fn transfer_of(m: &MessageView) -> String {
        m.media.as_ref().and_then(|f| f["transfer_id"].as_str()).unwrap().to_string()
    }

    /// A video goes with the frame the UI took, written anew as a small
    /// JPEG, and with its size; a frame that is no picture is left out.
    #[tokio::test]
    async fn a_video_goes_with_its_frame() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let peer = Keys::generate().public_key().to_hex();
        let video = dir.path().join("trip.mp4");
        std::fs::write(&video, vec![7u8; 4096]).unwrap();
        let mut frame = Vec::new();
        image::DynamicImage::ImageRgb8(picture(640, 360))
            .write_to(&mut std::io::Cursor::new(&mut frame), image::ImageFormat::Jpeg)
            .unwrap();
        let poster = Poster { jpeg: B64.encode(&frame), width: 1920, height: 1080 };

        let ph = rt.dm_send_file_with(&peer, &video, None, None, false, Some(poster)).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let m = sent(&rt, &t).await;
        assert_eq!(m["kind"].as_str(), Some("video"));
        assert_eq!(m["dim"], serde_json::json!([1920, 1080]));
        let thumb = B64.decode(m["thumb"].as_str().expect("a preview")).unwrap();
        assert_ne!(thumb, frame, "written anew");
        assert_eq!(messenger_avatar::upright_size(&thumb), Some((160, 90)));

        let bad = Poster { jpeg: B64.encode(b"<svg onload=alert(1)>"), width: 0, height: 5 };
        let ph = rt.dm_send_file_with(&peer, &video, None, None, false, Some(bad)).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let m = sent(&rt, &t).await;
        assert!(m.get("thumb").is_none() && m.get("dim").is_none(), "{m}");
        rt.shutdown().await;
    }

    /// The message that took the place of the placeholder of `transfer`.
    async fn sent(rt: &MessengerRuntime, transfer: &str) -> serde_json::Value {
        let id = rt.media.transfer(transfer).await.unwrap().unwrap().message_id.unwrap();
        assert!(!id.starts_with("local:"), "the message went out");
        rt.dm.message(&id).await.unwrap().unwrap().media.unwrap()
    }

    /// A file saved under a name without an extension keeps its own; one
    /// saved under another extension is the user's choice.
    #[tokio::test]
    async fn save_as_keeps_the_extension_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let peer = Keys::generate().public_key().to_hex();

        let file = dir.path().join("notes.txt");
        tokio::fs::write(&file, b"hello").await.unwrap();
        let ph = rt.dm_send_file(&peer, &file, None, None, true).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let id = rt.media.transfer(&t).await.unwrap().unwrap().message_id.unwrap();

        rt.media_save_as(&id, &dir.path().join("bare")).await.unwrap();
        assert_eq!(tokio::fs::read(dir.path().join("bare.txt")).await.unwrap(), b"hello");
        assert!(!dir.path().join("bare").exists());
        rt.media_save_as(&id, &dir.path().join("chosen.md")).await.unwrap();
        assert!(dir.path().join("chosen.md").exists() && !dir.path().join("chosen.md.txt").exists());

        // The dialog confirmed only "taken": a "taken.txt" already there
        // stays as it was, and the copy goes to the name as chosen.
        tokio::fs::write(dir.path().join("taken.txt"), b"old").await.unwrap();
        rt.media_save_as(&id, &dir.path().join("taken")).await.unwrap();
        assert_eq!(tokio::fs::read(dir.path().join("taken.txt")).await.unwrap(), b"old");
        assert_eq!(tokio::fs::read(dir.path().join("taken")).await.unwrap(), b"hello");
        // The name the dialog returned is overwritten: it asked about that one.
        tokio::fs::write(dir.path().join("taken"), b"older").await.unwrap();
        rt.media_save_as(&id, &dir.path().join("taken")).await.unwrap();
        assert_eq!(tokio::fs::read(dir.path().join("taken")).await.unwrap(), b"hello");
        rt.shutdown().await;
    }

    #[test]
    fn a_name_without_extension_takes_the_one_of_the_source() {
        let src = Path::new("/c/abc/photo.jpg");
        assert_eq!(with_source_extension(Path::new("/d/pic"), src), Path::new("/d/pic.jpg"));
        assert_eq!(with_source_extension(Path::new("/d/pic.png"), src), Path::new("/d/pic.png"));
        assert_eq!(with_source_extension(Path::new("/d/pic"), Path::new("/c/abc/noext")), Path::new("/d/pic"));
    }

    /// A photo is made smaller before its upload (the preparing stage) and
    /// goes as a JPEG of at most 1280 on its longer side, its size on the
    /// wire; with the original asked for it goes as it is. A file over
    /// 1 GiB is refused at the pick and at the send, before anything
    /// shows.
    #[tokio::test]
    async fn a_photo_goes_smaller_unless_the_original_is_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let peer = Keys::generate().public_key().to_hex();

        let big = dir.path().join("holiday.png");
        picture(1600, 1200).save(&big).unwrap();
        let big_len = std::fs::metadata(&big).unwrap().len();
        let mut events = rt.ui_events();
        let ph = rt.dm_send_file(&peer, &big, Some("sea"), None, false).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let m = sent(&rt, &t).await;
        assert_eq!((m["name"].as_str(), m["mime"].as_str()), (Some("holiday.jpg"), Some("image/jpeg")));
        assert_eq!(m["dim"], serde_json::json!([1280, 960]));
        assert!(m["size"].as_u64().unwrap() < big_len);
        // Its preview goes with it: a small JPEG within the limit.
        let thumb = m["thumb"].as_str().expect("a preview");
        assert!(valid_thumb(thumb));
        assert_eq!(messenger_avatar::upright_size(&B64.decode(thumb).unwrap()), Some((160, 120)));
        let local = PathBuf::from(m["local_path"].as_str().unwrap());
        assert!(local.starts_with(cfg.data_dir().join("outgoing")), "{local:?}");
        assert_eq!(std::fs::metadata(&local).unwrap().len(), m["size"].as_u64().unwrap());
        assert!(big.exists(), "the picked file stays");

        let mut told = Vec::new();
        while let Ok(ev) = events.try_recv() {
            if ev.name == UI_EVENT_TRANSFER && ev.payload["transfer_id"] == t.as_str() {
                let p: Progress = serde_json::from_value(ev.payload).unwrap();
                told.push(p);
            }
        }
        let stages: Vec<TransferStage> = told.iter().map(|p| p.stage).collect();
        assert_eq!(stages.first(), Some(&TransferStage::Preparing), "{stages:?}");
        let publishing = told.iter().find(|p| p.stage == TransferStage::Publishing).expect("a publishing event");
        assert_eq!(publishing.message_id.as_deref(), Some(ph.id.as_str()), "about the placeholder");
        assert_eq!(told.last().unwrap().status, "done");
        assert_eq!(told.last().unwrap().file_name, "holiday.jpg");

        // The original, as it is.
        let ph = rt.dm_send_file(&peer, &big, None, None, true).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let m = sent(&rt, &t).await;
        assert_eq!((m["name"].as_str(), m["size"].as_u64()), (Some("holiday.png"), Some(big_len)));
        assert_eq!(m["dim"], serde_json::json!([1600, 1200]));

        // A small photo needs nothing: it goes as it is, its size told.
        let small = dir.path().join("icon.jpg");
        picture(64, 48).save(&small).unwrap();
        let ph = rt.dm_send_file(&peer, &small, None, None, false).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let m = sent(&rt, &t).await;
        assert_eq!((m["name"].as_str(), m["dim"].clone()), (Some("icon.jpg"), serde_json::json!([64, 48])));

        // Over the limit: refused before anything shows.
        let huge = dir.path().join("huge.bin");
        std::fs::File::create(&huge).unwrap().set_len(MAX_SEND_BYTES + 1).unwrap();
        let too_large = |e: MessengerError| assert_eq!(short_reason(&e), "err.file_too_large");
        too_large(rt.picked(&huge).await.unwrap_err());
        let chat = format!("dm:{peer}");
        let before = rt.dm.messages(&chat, None, 100).await.unwrap().len();
        too_large(rt.dm_send_file(&peer, &huge, None, None, false).await.unwrap_err());
        assert_eq!(rt.dm.messages(&chat, None, 100).await.unwrap().len(), before, "no placeholder came and went");
        rt.shutdown().await;
    }

    /// Photos are made smaller a few at a time (`PHOTO_SLOTS`): one that
    /// finds no slot stays in the preparing stage, its file untouched,
    /// until one is free.
    #[tokio::test]
    async fn photos_are_made_smaller_a_few_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let peer = Keys::generate().public_key().to_hex();
        let big = dir.path().join("album.png");
        picture(1600, 1200).save(&big).unwrap();

        // Every slot is taken, as by the other photos of an album.
        let taken = rt.photo_slots.clone().acquire_many_owned(PHOTO_SLOTS as u32).await.unwrap();
        let ph = rt.dm_send_file(&peer, &big, None, None, false).await.unwrap();
        let t = transfer_of(&ph);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let waiting = rt.media.transfer(&t).await.unwrap().unwrap();
        assert_eq!(waiting.status, "queued");
        assert_eq!(waiting.stage, TransferStage::Preparing, "it waits in the preparing stage");
        assert_eq!(waiting.local_path.as_deref(), Some(big.to_string_lossy().as_ref()), "not made smaller yet");

        drop(taken);
        until(async || status_of(&rt, &t).await == "done").await;
        assert_eq!(sent(&rt, &t).await["name"].as_str(), Some("album.jpg"));
        rt.shutdown().await;
    }

    /// A copy the app made to send a file (a pick on Android) goes once
    /// nothing refers to it: when a photo made smaller takes its place,
    /// and when its upload is cancelled. A file picked where it is stays.
    #[tokio::test]
    async fn a_copy_the_app_made_goes_once_nothing_refers_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let peer = Keys::generate().public_key().to_hex();
        let outgoing = cfg.data_dir().join("outgoing");

        // As `messenger_media_import` copies a `content://` pick.
        let copy = outgoing.join("1a2b").join("IMG_1.png");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        picture(1600, 1200).save(&copy).unwrap();
        let ph = rt.dm_send_file(&peer, &copy, None, None, false).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "done").await;
        let local = PathBuf::from(sent(&rt, &t).await["local_path"].as_str().unwrap().to_string());
        assert!(local.exists(), "the photo made smaller stays: the message shows it");
        assert!(!copy.parent().unwrap().exists(), "the full-size copy went");

        // A cancelled upload leaves nothing of its copy.
        let copy = outgoing.join("3c4d").join("notes.bin");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, b"0123456789").unwrap();
        let ph = placeholder(&rt, 7).await;
        let id = upload(&rt, &copy, &ph, transfers::ST_PAUSED).await;
        rt.media_cancel(&id).await.unwrap();
        assert_eq!(status_of(&rt, &id).await, "cancelled");
        assert!(!copy.parent().unwrap().exists(), "the copy went with the upload");

        // Never a file the user picked where it is.
        let picked = dir.path().join("mine.bin");
        std::fs::write(&picked, b"0123456789").unwrap();
        let ph = placeholder(&rt, 8).await;
        let id = upload(&rt, &picked, &ph, transfers::ST_PAUSED).await;
        rt.media_cancel(&id).await.unwrap();
        assert!(picked.exists(), "the user's file stays");
        rt.shutdown().await;
    }

    /// The app closed while files went: once a session runs again, an
    /// upload whose placeholder is here and a download the user started go
    /// on by themselves. A pause the user made stays, and so does a download
    /// the automatic path started, until "retry all" takes it up.
    #[tokio::test]
    async fn interrupted_transfers_go_on_by_themselves_once_a_session_runs() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        // The identity is here, the session not yet (as before an unlock).
        rt.identity().create("pw").await.unwrap();
        let keys = rt.identity().load_keys().await.unwrap();
        let peer = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        let path = dir.path().join("notes.bin");
        tokio::fs::write(&path, vec![7u8; 300_000]).await.unwrap();

        // Two uploads: one the closing of the app paused, one the user did.
        let mut up = Vec::new();
        for _ in 0..2 {
            let fields = serde_json::json!({ "name": "notes.bin", "mime": "application/octet-stream", "size": 300_000, "kind": "file" });
            let ph = rt.dm.media_placeholder(&keys, &peer, fields, None).await.unwrap();
            let t = rt.media.queue_upload(&path, &ph.chat_id, &ph.id).await.unwrap();
            rt.dm.media_set_status(&ph.id, "paused", None).await.unwrap();
            up.push((ph.id, t.id));
        }
        transfers::set_status(&rt.store, &up[0].1, transfers::ST_PAUSED, Some(transfers::REASON_INTERRUPTED)).await.unwrap();
        transfers::set_status(&rt.store, &up[1].1, transfers::ST_PAUSED, None).await.unwrap();

        // Two files the peer sent: the user started the download of one,
        // the automatic path that of the other; the closing of the app
        // paused both.
        let chat = messenger_store::chats::ensure_dm(&rt.store, peer.as_hex()).await.unwrap().id;
        let mut down = Vec::new();
        for (n, manual) in [(0, true), (1, false)] {
            let theirs = dir.path().join(format!("theirs-{n}.bin"));
            tokio::fs::write(&theirs, vec![n as u8; 200_000]).await.unwrap();
            let blob = rt.media.queue_upload(&theirs, "dm:x", "x").await.unwrap();
            let d = rt.media.run_upload(&blob.id, &keys, None, &UiSink(rt.ui.clone())).await.unwrap().unwrap();
            let id = format!("in-{n}");
            let m = NewMessage {
                id: id.clone(),
                chat_id: chat.clone(),
                wire_id: None,
                direction: messages::DIR_IN.into(),
                status: messages::STATUS_RECEIVED.into(),
                content_type: messages::CT_MEDIA.into(),
                text: None,
                envelope_json: "{}".into(),
                sender_pubkey: peer.as_hex().to_string(),
                reply_to_id: None,
                target_id: None,
                created_at: n,
                is_hidden: false,
                outbox_local_id: None,
                media_json: Some(serde_json::to_string(&d).unwrap()),
            };
            messages::insert(&rt.store, &m).await.unwrap();
            let row = transfers::TransferRow {
                id: format!("down-{n}"),
                direction: DIR_DOWN.into(),
                message_id: Some(id.clone()),
                chat_id: Some(chat.clone()),
                local_path: None,
                file_name: d.name.clone(),
                mime: d.mime.clone(),
                size: d.size as i64,
                sha256: Some(d.sha256.clone()),
                status: transfers::ST_PAUSED.into(),
                done_bytes: 0,
                attempts: 0,
                failure_reason: Some(transfers::REASON_INTERRUPTED.into()),
                state_json: if manual { r#"{"manual":true}"# } else { "{}" }.into(),
                created_at: 0,
                updated_at: 0,
            };
            transfers::insert_transfer(&rt.store, &row).await.unwrap();
            down.push((id, row.id));
        }

        assert!(rt.refresh_signer().await.unwrap(), "the session starts");
        until(async || status_of(&rt, &up[0].1).await == "done").await;
        until(async || status_of(&rt, &down[0].1).await == "done").await;
        assert!(rt.dm.message(&up[0].0).await.unwrap().is_none(), "its message took the place of the placeholder");
        assert!(rt.media_local_path(&down[0].0).await.unwrap().is_some());
        let paused = rt.media.transfer(&up[1].1).await.unwrap().unwrap();
        assert_eq!((paused.status.as_str(), paused.failure_reason), ("paused", None), "the user's pause stays");
        assert_eq!(rt.dm.message(&up[1].0).await.unwrap().unwrap().status, "paused");
        assert_eq!(status_of(&rt, &down[1].1).await, "paused", "started by itself, it waits to be seen");

        // "Retry all" takes up what the closing paused, never the user's pause.
        assert_eq!(rt.media_retry_failed().await.unwrap(), 1);
        until(async || status_of(&rt, &down[1].1).await == "done").await;
        assert_eq!(status_of(&rt, &up[1].1).await, "paused");
        let listed: Vec<String> = rt.media_transfers().await.unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(listed, [up[1].1.clone()], "only what is not over is listed");
        rt.shutdown().await;
    }

    /// A transfer that waits for its next attempt makes it as soon as the
    /// network is back, not when its wait is over.
    #[tokio::test]
    async fn a_transfer_waiting_for_its_next_attempt_makes_it_when_the_network_is_back() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, vec![1u8; 1000]).await.unwrap();
        // The storage answers nothing but "try again" until the network is back.
        *backend.flaky_puts.lock().unwrap() = u32::MAX;

        let ph = rt.dm_send_file(&Keys::generate().public_key().to_hex(), &path, None, None, false).await.unwrap();
        let t = transfer_of(&ph);
        until(async || status_of(&rt, &t).await == "waiting_retry").await;
        let waits = rt.media.transfer(&t).await.unwrap().unwrap().retry_at_ms.unwrap();
        *backend.flaky_puts.lock().unwrap() = 0;
        rt.media_network_back().await;
        until(async || status_of(&rt, &t).await == "done").await;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        assert!(now < waits, "made before its wait was over");
        rt.shutdown().await;
    }

    /// The `transfer.progress` events waiting in `events`: transfer and status.
    fn progress_told(events: &mut broadcast::Receiver<UiEvent>) -> Vec<(String, String)> {
        let mut told = Vec::new();
        while let Ok(ev) = events.try_recv() {
            if ev.name == UI_EVENT_TRANSFER {
                let p: Progress = serde_json::from_value(ev.payload).unwrap();
                told.push((p.transfer_id, p.status));
            }
        }
        told
    }

    /// A download row of the message `message_id` in `chat`, in `status`.
    async fn download_row(rt: &MessengerRuntime, id: &str, message_id: &str, chat: &str, status: &str) {
        let row = transfers::TransferRow {
            id: id.into(),
            direction: DIR_DOWN.into(),
            message_id: Some(message_id.into()),
            chat_id: Some(chat.into()),
            local_path: None,
            file_name: "f.bin".into(),
            mime: "application/octet-stream".into(),
            size: 10,
            sha256: Some("0".repeat(64)),
            status: status.into(),
            done_bytes: 0,
            attempts: 1,
            failure_reason: Some("err.not_found".into()),
            state_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        };
        transfers::insert_transfer(&rt.store, &row).await.unwrap();
    }

    /// A received file in `chat`.
    async fn received(rt: &MessengerRuntime, id: &str, chat: &str) {
        let m = NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: messages::DIR_IN.into(),
            status: messages::STATUS_RECEIVED.into(),
            content_type: messages::CT_MEDIA.into(),
            text: None,
            envelope_json: "{}".into(),
            sender_pubkey: "b".repeat(64),
            reply_to_id: None,
            target_id: None,
            created_at: 1,
            is_hidden: false,
            outbox_local_id: None,
            media_json: Some("{}".into()),
        };
        messages::insert(&rt.store, &m).await.unwrap();
    }

    /// A cancel of a transfer nothing runs (a paused upload, a failed
    /// download) is told to the bubble and the list, as one a run
    /// cancels is; so is an upload found sent, which is done.
    #[tokio::test]
    async fn a_cancel_where_nothing_runs_is_told() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"0123456789").await.unwrap();
        let ph = placeholder(&rt, 0).await;
        let up = upload(&rt, &path, &ph, transfers::ST_PAUSED).await;
        download_row(&rt, "down-0", "in-0", "dm:x", transfers::ST_FAILED).await;
        let gone = placeholder(&rt, 1).await;
        let sent = upload(&rt, &path, &gone, transfers::ST_PAUSED).await;
        messages::delete(&rt.store, &gone).await.unwrap();
        let mut events = rt.ui.subscribe();

        rt.media_cancel(&up).await.unwrap();
        rt.media_cancel("down-0").await.unwrap();
        rt.media_cancel(&sent).await.unwrap();
        let told = progress_told(&mut events);
        assert!(told.contains(&(up.clone(), "cancelled".into())), "{told:?}");
        assert!(told.contains(&("down-0".into(), "cancelled".into())), "{told:?}");
        assert!(told.contains(&(sent.clone(), "done".into())), "{told:?}");
        rt.shutdown().await;
    }

    /// The list of transfers shows none whose message is gone; deleting a
    /// chat or a message cancels its transfers.
    #[tokio::test]
    async fn the_transfers_of_a_deleted_chat_or_message_end() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        // Delete for me tells my other devices: it needs a session.
        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"0123456789").await.unwrap();
        let ph = placeholder(&rt, 0).await;
        let chat = rt.dm.message(&ph).await.unwrap().unwrap().chat_id;
        let up = rt.media.queue_upload(&path, &chat, &ph).await.unwrap().id;
        transfers::set_status(&rt.store, &up, transfers::ST_FAILED, Some("err.network")).await.unwrap();
        received(&rt, "in-0", &chat).await;
        download_row(&rt, "down-0", "in-0", &chat, transfers::ST_FAILED).await;
        received(&rt, "in-1", &chat).await;
        download_row(&rt, "down-1", "in-1", &chat, transfers::ST_PAUSED).await;
        // Its chat was deleted before transfers ended with it.
        download_row(&rt, "down-2", "in-2", "dm:gone", transfers::ST_FAILED).await;
        let listed = async || rt.media_transfers().await.unwrap().into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(listed().await.len(), 3, "not one whose message is gone");

        rt.dm_delete("in-1", false).await.unwrap();
        assert_eq!(status_of(&rt, "down-1").await, "cancelled");
        assert_eq!(listed().await.len(), 2);

        rt.chat_delete(&chat).await.unwrap();
        assert_eq!(status_of(&rt, &up).await, "cancelled");
        assert_eq!(status_of(&rt, "down-0").await, "cancelled");
        assert!(listed().await.is_empty());
        assert_eq!(rt.media_retry_failed().await.unwrap(), 0);
        rt.shutdown().await;
    }

    /// The automatic attempts of an upload ran out while the connection
    /// was lost: it starts again by itself when the network is back. One
    /// that failed for another reason waits for the user.
    #[tokio::test]
    async fn an_upload_failed_offline_goes_on_when_the_network_is_back() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let keys = rt.session_keys().await.unwrap();
        let peer = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        let path = dir.path().join("notes.bin");
        tokio::fs::write(&path, vec![7u8; 1000]).await.unwrap();
        let mut up = Vec::new();
        for reason in ["err.network", "err.timeout", "err.file_changed"] {
            let fields = serde_json::json!({ "name": "notes.bin", "mime": "application/octet-stream", "size": 1000, "kind": "file" });
            let ph = rt.dm.media_placeholder(&keys, &peer, fields, None).await.unwrap();
            let t = rt.media.queue_upload(&path, &ph.chat_id, &ph.id).await.unwrap();
            transfers::set_status(&rt.store, &t.id, transfers::ST_FAILED, Some(reason)).await.unwrap();
            up.push(t.id);
        }
        rt.media_network_back().await;
        until(async || status_of(&rt, &up[0]).await == "done").await;
        until(async || status_of(&rt, &up[1]).await == "done").await;
        assert_eq!(status_of(&rt, &up[2]).await, "failed", "left to the user");
        rt.shutdown().await;
    }

    /// An upload another identity left (its placeholder is not signed
    /// with the keys of this session) never goes on by itself.
    #[tokio::test]
    async fn an_upload_of_another_identity_is_not_taken_up() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"0123456789").await.unwrap();
        // Its sender is "aaa…", not this session.
        let ph = placeholder(&rt, 0).await;
        let id = upload(&rt, &path, &ph, transfers::ST_PAUSED).await;
        transfers::set_status(&rt.store, &id, transfers::ST_PAUSED, Some(transfers::REASON_INTERRUPTED)).await.unwrap();
        assert_eq!(rt.resume_interrupted().await.unwrap(), 0);
        assert_eq!(rt.media_retry_failed().await.unwrap(), 0);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(status_of(&rt, &id).await, "paused");
        rt.shutdown().await;
    }

    /// A pause and a resume while a photo waits to be made smaller leave
    /// it to the job that waits: it is made smaller once, and the row and
    /// the message name the same file.
    #[tokio::test]
    async fn a_photo_paused_and_resumed_while_it_waits_is_made_smaller_once() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let peer = Keys::generate().public_key().to_hex();
        let big = dir.path().join("album.png");
        picture(1600, 1200).save(&big).unwrap();

        let taken = rt.photo_slots.clone().acquire_many_owned(PHOTO_SLOTS as u32).await.unwrap();
        let ph = rt.dm_send_file(&peer, &big, None, None, false).await.unwrap();
        let t = transfer_of(&ph);
        until(async || rt.media.transfer(&t).await.unwrap().unwrap().stage == TransferStage::Preparing).await;
        for _ in 0..2 {
            rt.media_pause(&t).await.unwrap();
            assert_eq!(status_of(&rt, &t).await, "paused");
            rt.media_resume(&t).await.unwrap();
            until(async || status_of(&rt, &t).await == "queued").await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        drop(taken);
        until(async || status_of(&rt, &t).await == "done").await;
        let row = rt.media.transfer(&t).await.unwrap().unwrap();
        let m = sent(&rt, &t).await;
        assert_eq!(m["local_path"].as_str(), row.local_path.as_deref());
        let made = std::fs::read_dir(cfg.data_dir().join("outgoing")).unwrap().count();
        assert_eq!(made, 1, "made smaller once");
        rt.shutdown().await;
    }

    /// The app ended after the row of an upload took the photo made
    /// smaller and before its placeholder did: the next run tells the
    /// placeholder first, so the message says a JPEG of its real size.
    #[tokio::test]
    async fn a_placeholder_behind_its_row_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let mut rt = runtime(&cfg, &Arc::new(MemorySecretStore::unlocked())).await;
        let backend = MemoryBackend::new("https://mem.example/a");
        use_memory(&mut rt, &backend);
        with_session(&rt).await;
        let keys = rt.session_keys().await.unwrap();
        let peer = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        let big = dir.path().join("holiday.png");
        picture(1600, 1200).save(&big).unwrap();
        let jpeg = cfg.data_dir().join("outgoing").join("5e").join("holiday.jpg");
        std::fs::create_dir_all(jpeg.parent().unwrap()).unwrap();
        picture(640, 480).save(&jpeg).unwrap();

        let fields = serde_json::json!({
            "name": "holiday.png", "mime": "image/png", "kind": "image", "dim": [1600, 1200],
            "size": std::fs::metadata(&big).unwrap().len(), "local_path": big.to_string_lossy(),
        });
        let ph = rt.dm.media_placeholder(&keys, &peer, fields, None).await.unwrap();
        let t = rt.media.queue_upload(&big, &ph.chat_id, &ph.id).await.unwrap().id;
        assert!(rt.media.replace_file(&t, &jpeg).await.unwrap().is_some());
        transfers::set_status(&rt.store, &t, transfers::ST_PAUSED, Some(transfers::REASON_INTERRUPTED)).await.unwrap();

        assert_eq!(rt.resume_interrupted().await.unwrap(), 1);
        until(async || status_of(&rt, &t).await == "done").await;
        let m = sent(&rt, &t).await;
        assert_eq!((m["name"].as_str(), m["mime"].as_str()), (Some("holiday.jpg"), Some("image/jpeg")));
        assert_eq!(m["dim"], serde_json::json!([640, 480]));
        assert_eq!(m["size"].as_u64(), Some(std::fs::metadata(&jpeg).unwrap().len()));
        assert!(big.exists(), "the picked file stays");
        rt.shutdown().await;
    }
}

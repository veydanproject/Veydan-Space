// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Attachments: glue between the DM module (message rows), the media
//! module (blobs and transfers) and the outbox. Uploads run in background
//! tasks; progress reaches the host as `transfer.progress` events.


use crate::{MessengerRuntime, REGION_FALLBACK};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::traits::UiEvent;
use messenger_core::{MessengerError, PubKey, Result};
use messenger_dm::{DmService, MessageView};
use messenger_ingress::Outbox;
use messenger_media::service::SMALL_BYTES;
use messenger_media::{MediaDescriptor, MediaKind, MediaServerInput, MediaServerView, MediaService, Progress, ProgressSink, TransferView};
use messenger_store::media::{DIR_DOWN, DIR_UP};
use crate::relays::ServersMode;
use nostr::key::Keys;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;

pub const UI_EVENT_TRANSFER: &str = "transfer.progress";
/// Received files up to this size are fetched without asking.
pub const AUTO_DOWNLOAD_BYTES: u64 = SMALL_BYTES;
/// Largest file handed to the UI inline (previews).
pub const MAX_INLINE_BYTES: u64 = 24 * 1024 * 1024;

/// Largest recording accepted from the UI in one call.
pub const MAX_RECORDING_BYTES: u64 = 64 * 1024 * 1024;

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

/// Everything an upload task needs, detached from the runtime's lifetime.
#[derive(Clone)]
struct UploadJob {
    groups: messenger_groups::GroupService,
    dm: DmService,
    media: MediaService,
    outbox: Outbox,
    ui: broadcast::Sender<UiEvent>,
    keys: Keys,
}

impl UploadJob {
    fn updated(&self, chat_id: &str, message_id: &str) {
        let _ = self.ui.send(UiEvent {
            name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
        });
    }

    /// The blob is stored and the chat is a group: the message goes out
    /// under the group key.
    async fn finish_in_group(
        &self,
        transfer_id: &str,
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
                let _ = messenger_store::media::set_result(self.dm.store(), transfer_id, None, None, Some(&message.id)).await;
                if let Ok(local_id) = self.outbox.enqueue_message(out).await {
                    let _ = self.dm.attach_outbox(&message.id, &local_id).await;
                }
                self.outbox.kick();
                self.updated(chat_id, &message.id);
            }
            Err(e) => {
                let _ = self.ui.send(UiEvent {
                    name: "error".into(),
                    payload: serde_json::json!({ "family": "media", "error": e.to_string() }),
                });
                self.updated(chat_id, placeholder);
            }
        }
    }

    async fn run(self, transfer_id: String, placeholder: String, chat_id: String, local_path: String) {
        let sink = UiSink(self.ui.clone());
        let outcome = self.media.run_upload(&transfer_id, &self.keys, None, &sink).await;
        match outcome {
            Ok(Some(mut descriptor)) => {
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
                    self.finish_in_group(&transfer_id, &placeholder, &chat_id, envelope, local).await;
                    return;
                }
                match self.dm.media_finish(&self.keys, &placeholder, envelope, local).await {
                    Ok(p) => {
                        let message_id = p.message.id.clone();
                        let _ = messenger_store::media::set_result(self.dm.store(), &transfer_id, None, None, Some(&message_id)).await;
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
                        self.updated(&chat_id, &message_id);
                    }
                    Err(e) => {
                        let _ = self.ui.send(UiEvent {
                            name: "error".into(),
                            payload: serde_json::json!({ "family": "media", "error": e.to_string() }),
                        });
                        self.updated(&chat_id, &placeholder);
                    }
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
                } else {
                    let _ = self.dm.media_set_status(&placeholder, "paused", None).await;
                }
                self.updated(&chat_id, &placeholder);
            }
            Err(e) => {
                let reason = self
                    .media
                    .transfer(&transfer_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|t| t.failure_reason)
                    .unwrap_or_else(|| e.to_string());
                let _ = self.dm.media_set_status(&placeholder, "failed", Some(&reason)).await;
                self.updated(&chat_id, &placeholder);
            }
        }
    }
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
        for old in self.media.servers().await? {
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
        Ok(())
    }

    pub async fn media_servers(&self) -> Result<Vec<MediaServerView>> {
        self.media.servers().await
    }

    pub async fn media_server_put(&self, input: MediaServerInput) -> Result<MediaServerView> {
        self.media.put_server(input).await
    }

    pub async fn media_server_remove(&self, id: &str) -> Result<()> {
        self.media.remove_server(id).await
    }

    pub async fn media_server_set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.media.set_server_enabled(id, enabled).await
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
        }
    }

    /// Attach a file to the chat with `to` (a person, or `group:<id>`). Returns the placeholder
    /// message at once; the upload continues in the background.
    pub async fn dm_send_file(&self, to: &str, path: &Path, caption: Option<&str>, batch: Option<&str>) -> Result<MessageView> {
        if batch.is_some_and(|b| !messenger_media::descriptor::valid_batch(b)) {
            return Err(MessengerError::Invalid("err.bad_batch".into()));
        }
        self.send_attachment(to, path, caption, None, batch).await
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
        let result = self.send_attachment(to, &path, caption, Some(meta), None).await;
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
        result
    }

    async fn send_attachment(
        &self,
        to: &str,
        path: &Path,
        caption: Option<&str>,
        meta_override: Option<AttachmentMeta>,
        batch: Option<&str>,
    ) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let group = to.strip_prefix("group:").map(String::from);
        let peer = match &group {
            Some(_) => None,
            None => Some(messenger_contacts::book::parse_key(to)?),
        };
        // Refuse early: no server, no upload.
        self.media.upload_backend(&keys).await?;
        let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
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
        let (tid, pid, cid, lp) =
            (transfer.id, placeholder.id.clone(), placeholder.chat_id.clone(), path.to_string_lossy().into_owned());
        tokio::spawn(job.run(tid, pid, cid, lp));
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
        let sink = UiSink(self.ui.clone());
        let path = self.media.run_download(message_id, &m.chat_id, &d, manual, &sink).await?;
        if let Some(p) = &path {
            self.dm.media_set_local_path(message_id, &p.to_string_lossy()).await?;
            let _ = self.ui.send(UiEvent {
                name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
                payload: serde_json::json!({ "chat_id": m.chat_id, "message_id": message_id }),
            });
        }
        Ok(path)
    }

    /// The transfer of a message (upload for ours, download for theirs).
    pub async fn media_transfer(&self, message_id: &str) -> Result<Option<TransferView>> {
        match self.media.transfer_for_message(message_id, DIR_UP).await? {
            Some(t) => Ok(Some(t)),
            None => self.media.transfer_for_message(message_id, DIR_DOWN).await,
        }
    }

    pub async fn media_pause(&self, transfer_id: &str) -> Result<()> {
        self.media.pause(transfer_id);
        Ok(())
    }

    pub async fn media_cancel(&self, transfer_id: &str) -> Result<()> {
        let t = self.media.transfer(transfer_id).await?;
        self.media.cancel(transfer_id).await?;
        // An upload that is not running has nobody to clean up after it.
        if let Some(t) = t {
            if t.direction == DIR_UP && t.status != "running" && t.status != "queued" {
                if let Some(mid) = &t.message_id {
                    if mid.starts_with("local:") {
                        self.dm.media_discard(mid).await?;
                        let _ = self.ui.send(UiEvent {
                            name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
                            payload: serde_json::json!({ "chat_id": t.chat_id, "message_id": mid }),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Continue a paused or failed transfer.
    pub async fn media_resume(&self, transfer_id: &str) -> Result<()> {
        let t = self.media.transfer(transfer_id).await?.ok_or_else(|| MessengerError::Invalid("unknown transfer".into()))?;
        if t.status == "running" || t.status == "queued" {
            return Ok(());
        }
        let message_id = t.message_id.clone().ok_or_else(|| MessengerError::Invalid("transfer has no message".into()))?;
        if t.direction == DIR_UP {
            if !message_id.starts_with("local:") {
                return Ok(()); // already sent
            }
            let keys = self.session_keys().await?;
            self.dm.media_set_status(&message_id, "uploading", None).await?;
            let job = self.upload_job(keys);
            let chat_id = t.chat_id.clone().unwrap_or_default();
            let local = t.local_path.clone().unwrap_or_default();
            tokio::spawn(job.run(t.id, message_id, chat_id, local));
            Ok(())
        } else {
            self.media_download(&message_id, true).await.map(|_| ())
        }
    }

    /// Copy the attachment somewhere the user chose.
    pub async fn media_save_as(&self, message_id: &str, dest: &Path) -> Result<()> {
        let src = self
            .media_local_path(message_id)
            .await?
            .ok_or_else(|| MessengerError::Invalid("err.not_downloaded".into()))?;
        tokio::fs::copy(&src, dest).await?;
        Ok(())
    }

    /// The attachment as a `data:` url for inline previews.
    pub async fn media_data_url(&self, message_id: &str) -> Result<Option<String>> {
        let Some(path) = self.media_local_path(message_id).await? else { return Ok(None) };
        let meta = tokio::fs::metadata(&path).await?;
        if meta.len() > MAX_INLINE_BYTES {
            return Err(MessengerError::Invalid("err.too_large_for_preview".into()));
        }
        let m = self.dm.message(message_id).await?.and_then(|m| m.media);
        let mime = m
            .as_ref()
            .and_then(|f| f.get("mime"))
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();
        let mime = base_mime(&mime);
        // Only types a webview renders passively; never html or svg.
        let safe = matches!(
            mime.as_str(),
            "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/avif" | "image/bmp"
                | "video/mp4" | "video/webm" | "audio/mpeg" | "audio/ogg" | "audio/wav" | "audio/mp4" | "audio/flac" | "audio/webm"
        );
        if !safe {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await?;
        Ok(Some(format!("data:{mime};base64,{}", B64.encode(bytes))))
    }
}

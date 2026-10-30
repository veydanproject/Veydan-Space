// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Messages of one chat forwarded to contacts and groups of mine
//! (internal/messenger-wire.md §3, «Пересылка»). Each goes as a new message of mine,
//! the usual way of its chat; `fwd` says whom it is of, unless the sender
//! is hidden. What a message had besides its words and its file stays
//! behind: reactions, the quote of a reply, the earlier versions.
//!
//! A file is not fetched nor sent again: the new message names the blobs
//! of the old one. Only a chat of another server than the file's (a
//! private chat, a group of a private server) gets its own copy, uploaded
//! from this device when the file is here, and none otherwise. Such an
//! upload goes from a copy of the file of its own, and what follows it in
//! that chat waits until its message is out: the chat keeps the order.

use crate::media::AttachmentMeta;
use crate::MessengerRuntime;
use messenger_contacts::ContactCard;
use messenger_core::envelope::FIELD_FWD;
use messenger_core::{Envelope, Forwarded, MessengerError, PubKey, Result, TextFormat};
use messenger_dm::service::MAX_TEXT_BYTES;
use messenger_dm::MessageView;
use messenger_media::descriptor::MediaDescriptor;
use messenger_store::messages::{self as repo, MessageRow};
use messenger_store::chats;
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// The most messages one forward takes.
pub const MAX_FORWARD_MESSAGES: usize = 100;
/// The most chats one forward goes to.
pub const MAX_FORWARD_TARGETS: usize = 10;

/// A message one chat did not get, and why (`err.forward_media_unavailable`:
/// the file is on another server than the chat's, and not on this device).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardSkip {
    pub message_id: String,
    pub reason: String,
}

/// What one chat got of a forward. `sent`: the new messages in it, in
/// their order (the comment first). `error`: why the rest did not go;
/// a message whose sending failed stays in the chat as failed, with the
/// usual retry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForwardResult {
    pub target: String,
    pub sent: Vec<MessageView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<ForwardSkip>,
}

/// One message as it goes again.
enum Item {
    Text { text: String, format: TextFormat },
    Media { media: Box<MediaDescriptor>, caption: Option<String>, format: TextFormat, local: Option<PathBuf> },
    Card(ContactCard),
}

struct Source {
    id: String,
    item: Item,
    fwd: Option<Forwarded>,
}

fn unfit() -> MessengerError {
    MessengerError::Invalid("forward_message_unfit".into())
}

fn target_unfit() -> MessengerError {
    MessengerError::Invalid("forward_target_unfit".into())
}

/// A new album for the files of one album of the source, in each chat.
fn new_batch() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("fw{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

/// The private server of a chat; `None` for the home set.
async fn server_of_chat(store: &messenger_store::Store, chat_id: &str) -> Result<Option<String>> {
    match chat_id.strip_prefix("group:") {
        Some(group) => crate::scopes::server_of_group(store, group).await,
        None => Ok(chats::parse_dm_chat_id(chat_id).and_then(|(_, server)| server).map(String::from)),
    }
}

/// Whether a row may be forwarded: a text, a file or a card of the chat,
/// shown, sent or received, not removed, nothing a moderator holds back.
fn may_go(row: &MessageRow, chat_id: &str) -> bool {
    row.chat_id == chat_id
        && !row.is_hidden
        && row.deleted_at.is_none()
        && row.mod_state.is_empty()
        && (row.review == "ok" || row.review == "approved")
        && (row.status == repo::STATUS_SENT || row.status == repo::STATUS_RECEIVED)
        && matches!(row.content_type.as_str(), repo::CT_TEXT | repo::CT_MEDIA | repo::CT_CONTACT)
}

impl MessengerRuntime {
    /// Forward `message_ids` of `source_chat` to `targets` (chat ids: a
    /// direct chat with a contact, a group of mine where I may write).
    /// `comment` goes first, as a text of its own; the messages follow in
    /// the order of the chat. With `hide_sender` they are mine and say
    /// nothing of their author; otherwise each says the name of its
    /// author, or of the first author of one forwarded already.
    ///
    /// Refused as a whole: nothing to forward or nowhere to
    /// (`forward_nothing`), more than `MAX_FORWARD_MESSAGES` or
    /// `MAX_FORWARD_TARGETS` (`forward_too_many_messages`,
    /// `forward_too_many_targets`), a message that may not go
    /// (`forward_message_unfit`: of another chat, removed, not sent, a
    /// signal, a type this app does not know) or a chat it may not go to
    /// (`forward_target_unfit`: a stranger, a request, the bot of Veydan
    /// Cloud, the archive, a blocked person, a chat I cannot write in).
    pub async fn forward(
        &self,
        source_chat: &str,
        message_ids: &[String],
        targets: &[String],
        hide_sender: bool,
        comment: Option<&str>,
    ) -> Result<Vec<ForwardResult>> {
        let keys = self.session_keys().await?;
        let mut ids: Vec<&str> = Vec::new();
        for id in message_ids {
            if !ids.contains(&id.as_str()) {
                ids.push(id);
            }
        }
        let mut chats_to: Vec<&str> = Vec::new();
        for t in targets {
            if !chats_to.contains(&t.as_str()) {
                chats_to.push(t);
            }
        }
        if ids.is_empty() || chats_to.is_empty() {
            return Err(MessengerError::Invalid("forward_nothing".into()));
        }
        if ids.len() > MAX_FORWARD_MESSAGES {
            return Err(MessengerError::Invalid("forward_too_many_messages".into()));
        }
        if chats_to.len() > MAX_FORWARD_TARGETS {
            return Err(MessengerError::Invalid("forward_too_many_targets".into()));
        }
        let comment = comment.map(str::trim).filter(|c| !c.is_empty());
        if comment.is_some_and(|c| c.len() > MAX_TEXT_BYTES) {
            return Err(MessengerError::Invalid(format!("message is longer than {} KiB", MAX_TEXT_BYTES / 1024)));
        }
        let sources = self.forward_sources(source_chat, &ids, hide_sender).await?;
        for t in &chats_to {
            self.check_forward_target(t).await?;
        }
        let from = server_of_chat(&self.store, source_chat).await?;
        let mut out = Vec::with_capacity(chats_to.len());
        for t in chats_to {
            out.push(self.forward_to(&keys, t, from.as_deref(), &sources, comment).await);
        }
        Ok(out)
    }

    /// The rows, checked, in the order of the chat.
    async fn forward_sources(&self, chat_id: &str, ids: &[&str], hide_sender: bool) -> Result<Vec<Source>> {
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            let row = repo::get(&self.store, id).await?.filter(|r| may_go(r, chat_id)).ok_or_else(unfit)?;
            rows.push(row);
        }
        rows.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        let mut names: HashMap<String, Option<String>> = HashMap::new();
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let format = TextFormat::from_name(Some(&row.fmt));
            let item = match row.content_type.as_str() {
                repo::CT_TEXT => {
                    let text = row.text.clone().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or_else(unfit)?;
                    Item::Text { text, format }
                }
                repo::CT_MEDIA => {
                    let fields: serde_json::Value = row.media_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).ok_or_else(unfit)?;
                    // What only this device wrote, `fwd` and the format of
                    // the caption are no part of the file: the descriptor
                    // reads none of them.
                    let media = MediaDescriptor::from_fields(&fields).map_err(|_| unfit())?;
                    let caption = row.text.clone().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
                    let local = self.media_local_path(&row.id).await?;
                    Item::Media { media: Box::new(media), format: if caption.is_some() { format } else { TextFormat::Plain }, caption, local }
                }
                _ => {
                    let card: ContactCard = row.media_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).ok_or_else(unfit)?;
                    Item::Card(card)
                }
            };
            // A card is no text: it names its person and nobody else.
            let fwd = match (&item, hide_sender) {
                (Item::Card(_), _) | (_, true) => None,
                _ => match messenger_dm::view::forwarded(&row.content_type, &row.envelope_json) {
                    Some(first) => Some(first),
                    None => {
                        if !names.contains_key(&row.sender_pubkey) {
                            let name = self.author_name(&row.sender_pubkey).await?;
                            names.insert(row.sender_pubkey.clone(), name);
                        }
                        Some(Forwarded::named(names[&row.sender_pubkey].as_deref()))
                    }
                },
            };
            out.push(Source { id: row.id, item, fwd });
        }
        Ok(out)
    }

    /// What the author of a forwarded message is called in `fwd`: the
    /// name of their public profile, else nothing. Never their key, nor a
    /// part of it, nor my name for them: that is mine, not for the chat.
    async fn author_name(&self, author: &str) -> Result<Option<String>> {
        let Some(pk) = PubKey::parse(author) else { return Ok(None) };
        let named = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        Ok(self.profiles.get(&pk).await?.and_then(|p| named(&p.display_name).or_else(|| named(&p.name))))
    }

    /// A chat a forward may go to: a direct chat with a contact of mine
    /// that is open both ways, or a group where I may write; never the
    /// bot, the archive, or what the network closed.
    async fn check_forward_target(&self, chat_id: &str) -> Result<()> {
        let chat = self.chat(chat_id).await?.ok_or_else(target_unfit)?;
        if chat.archived || chat.banned || chat.bot || !chat.can_send {
            return Err(target_unfit());
        }
        let fits = if chat.id.starts_with("group:") {
            true
        } else {
            chats::parse_dm_chat_id(&chat.id).is_some() && chat.is_contact && chat.mode == "full_chat"
        };
        if fits {
            Ok(())
        } else {
            Err(target_unfit())
        }
    }

    async fn forward_to(&self, keys: &Keys, target: &str, from: Option<&str>, sources: &[Source], comment: Option<&str>) -> ForwardResult {
        let mut result = ForwardResult { target: target.to_string(), sent: vec![], error: None, skipped: vec![] };
        if let Some(c) = comment {
            match self.send_forwarded(keys, target, Envelope::text(c), repo::CT_TEXT, Some(c.to_string()), None).await {
                Ok(m) => result.sent.push(m),
                Err(e) => {
                    result.error = Some(e.to_string());
                    return result;
                }
            }
        }
        let to_server = match server_of_chat(&self.store, target).await {
            Ok(s) => s,
            Err(e) => {
                result.error = Some(e.to_string());
                return result;
            }
        };
        let mut batches: HashMap<String, String> = HashMap::new();
        for s in sources {
            let sent = match &s.item {
                Item::Text { text, format } => {
                    let envelope = Envelope::text(text).with_format(*format).with_fwd(s.fwd.as_ref());
                    self.send_forwarded(keys, target, envelope, repo::CT_TEXT, Some(text.clone()), None).await
                }
                Item::Card(card) => {
                    let json = self.forwarded_card(card, from, to_server.as_deref()).to_json();
                    self.send_forwarded(keys, target, Envelope::contact(json.clone()), repo::CT_CONTACT, None, Some(json.to_string())).await
                }
                Item::Media { media, caption, format, local } => {
                    let mut d = (**media).clone();
                    d.caption = caption.clone();
                    d.batch = d.batch.map(|b| batches.entry(b).or_insert_with(new_batch).clone());
                    match self.blobs_serve(&d, from, to_server.as_deref()).await {
                        Ok(true) => {
                            let envelope = d.to_envelope().with_format(*format).with_fwd(s.fwd.as_ref());
                            let mut kept = serde_json::to_value(&d).unwrap_or_else(|_| serde_json::json!({}));
                            // The file is here already: the new message opens it without a download.
                            if let Some(path) = local {
                                kept["local_path"] = path.to_string_lossy().into_owned().into();
                            }
                            self.send_forwarded(keys, target, envelope, repo::CT_MEDIA, caption.clone(), Some(kept.to_string())).await
                        }
                        Ok(false) => match local {
                            Some(path) => match self.upload_forwarded(target, path, &d, *format, s.fwd.as_ref()).await {
                                // Out, or failed or paused: then the chat shows it so, and the rest waits.
                                Ok((m, stopped)) => {
                                    result.sent.push(m);
                                    if stopped.is_some() {
                                        result.error = stopped;
                                        break;
                                    }
                                    continue;
                                }
                                Err(e) => Err(e),
                            },
                            None => {
                                result.skipped.push(ForwardSkip { message_id: s.id.clone(), reason: "err.forward_media_unavailable".into() });
                                continue;
                            }
                        },
                        Err(e) => Err(e),
                    }
                }
            };
            match sent {
                Ok(m) => result.sent.push(m),
                Err(e) => {
                    result.error = Some(e.to_string());
                    break;
                }
            }
        }
        result
    }

    /// The blobs of `d` serve a chat of `to`: the home set takes the
    /// file where it is, as a message of the home set came with it; a
    /// chat of a private server only what is on its own blob servers. A
    /// file of a private chat never leaves for the home set: its address
    /// names the server.
    async fn blobs_serve(&self, d: &MediaDescriptor, from: Option<&str>, to: Option<&str>) -> Result<bool> {
        let Some(server) = to else { return Ok(from.is_none()) };
        let bases: Vec<String> = self.media.bases_of(Some(server)).await.into_iter().map(|b| b.trim_end_matches('/').to_string()).collect();
        Ok(!bases.is_empty() && d.servers.iter().all(|s| bases.iter().any(|b| b == s.trim_end_matches('/'))))
    }

    /// A card as it is forwarded: nobody's phone and no server of mine go
    /// on with it, nor the relays of another server than the chat's.
    fn forwarded_card(&self, card: &ContactCard, from: Option<&str>, to: Option<&str>) -> ContactCard {
        let mut card = card.clone();
        card.phone = None;
        card.server = None;
        if from != to {
            card.relays.clear();
        }
        card
    }

    /// The file of `d`, here at `path`, uploaded anew for the chat
    /// `target`, and its message once it is out. The upload goes from a
    /// copy of its own: a cancel of it takes that copy, never the file of
    /// the message forwarded. With the reason, when it stopped short (the
    /// placeholder then, failed or paused, with the usual retry).
    async fn upload_forwarded(
        &self,
        target: &str,
        path: &std::path::Path,
        d: &MediaDescriptor,
        format: TextFormat,
        fwd: Option<&Forwarded>,
    ) -> Result<(MessageView, Option<String>)> {
        let to = target.strip_prefix("dm:").unwrap_or(target);
        let name = Some(messenger_media::descriptor::safe_name(&d.name)).filter(|n| !n.is_empty()).unwrap_or_else(|| "file".into());
        let copy = self.own_copy_of(path, &name).await?;
        let meta = AttachmentMeta {
            kind: d.kind,
            mime: d.mime.clone(),
            duration_ms: d.duration_ms,
            waveform: d.waveform.clone(),
            thumb: d.thumb.clone(),
            dim: d.dim,
        };
        let batch = d.batch.as_deref().map(|b| (b, d.pos));
        let (placeholder, transfer_id, run) = match self.start_attachment(to, &copy, d.caption.as_deref(), format, Some(meta), batch, true, None, fwd).await {
            Ok(started) => started,
            Err(e) => {
                self.drop_own_copy(&copy).await;
                return Err(e);
            }
        };
        let _ = run.await;
        let transfer = self.media.transfer(&transfer_id).await?;
        let sent = transfer.as_ref().and_then(|t| t.message_id.clone()).filter(|m| !m.starts_with("local:"));
        if let Some(m) = match &sent {
            Some(id) => self.dm.message(id).await?,
            None => None,
        } {
            return Ok((m, None));
        }
        let stopped = transfer.and_then(|t| t.failure_reason.or(Some(format!("err.upload_{}", t.status)))).unwrap_or_else(|| "err.upload_cancelled".into());
        match self.dm.message(&placeholder.id).await? {
            Some(p) => Ok((p, Some(stopped))),
            None => Err(MessengerError::Invalid(stopped)),
        }
    }

    /// One message into `target`, as any message of mine goes there.
    async fn send_forwarded(
        &self,
        keys: &Keys,
        target: &str,
        envelope: Envelope,
        content_type: &str,
        text: Option<String>,
        media_json: Option<String>,
    ) -> Result<MessageView> {
        debug_assert!(content_type != repo::CT_CONTACT || !envelope.fields.contains_key(FIELD_FWD));
        if let Some(group) = target.strip_prefix("group:") {
            let (message, out) = {
                let lock = self.groups().lock_of(group).await;
                let _guard = lock.lock().await;
                self.group_driver.renew_stale_key(keys, group).await?;
                self.groups().prepare_message(keys, group, envelope, content_type, text, None, media_json).await?
            };
            let id = message.id.clone();
            return self.publish_group_message(message, out, &id, true).await;
        }
        let to = target.strip_prefix("dm:").ok_or_else(target_unfit)?;
        let peer = self.peer_addr(to).await?;
        let name = media_json.as_deref().and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()).and_then(|m| m.get("name").and_then(|n| n.as_str()).map(String::from));
        let format = envelope.format();
        let prepared = self.dm.prepare_visible(keys, &peer, envelope, content_type, text.clone(), None, media_json).await?;
        // The list of chats says a file as an upload of it does.
        if content_type == repo::CT_MEDIA {
            let line = match &text {
                Some(c) => format!("📎 {}", messenger_dm::view::preview_of(c, format)),
                None => format!("📎 {}", name.as_deref().unwrap_or("file")),
            };
            chats::touch(&self.store, &prepared.message.chat_id, prepared.message.created_at, Some(&line), false).await?;
        }
        self.publish_prepared(prepared).await
    }
}

#[cfg(test)]
mod tests;

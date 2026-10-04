// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Chats and messages. Everything here is local state plus event
//! building; sending is the runtime's job (it enqueues the returned
//! `Outbound`s and reports the outbox id back with `attach_outbox`).

use crate::view::{preview, ChatView, MessageView, ReplyPreview};
use crate::wrap::{wrap_as, Wake};
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::envelope::{T_CONTROL, T_DELETE, T_EDIT, T_MEDIA, T_TEXT};
use messenger_core::traits::{Notice, UiEvent};
use messenger_core::{
    Clock, Context, DmInbound, Effect, Envelope, EventSource, MessengerError, Outbound, PubKey, RelayUrl, Result,
};
use messenger_store::chats::{self, ChatRow};
use messenger_store::messages::{self as repo, MessageRow, NewMessage};
use messenger_store::shared::{self, Counts, Section};
use messenger_store::{dm_routes, Store};
use nostr::key::Keys;
use nostr::nips::nip19::ToBech32;
use nostr::prelude::PublicKey;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

pub const UI_EVENT_DM_MESSAGE: &str = "dm.message";
pub const UI_EVENT_DM_UPDATED: &str = "dm.updated";
pub const UI_EVENT_CHATS_UPDATED: &str = "chats.updated";

/// Plaintext budget of one message. NIP-44 caps the padded plaintext at
/// 64 KiB and the rumor JSON wraps the text, so stay well below.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
/// An outgoing message is shown as failed after this many publish attempts;
/// the outbox itself keeps retrying.
pub const FAILED_AFTER_ATTEMPTS: i64 = 3;

/// A message that is stored locally and ready to be published.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// The visible message this action concerns (for edits and deletes:
    /// the target after the change).
    pub message: MessageView,
    /// Row that tracks the publish status (the message itself, or the
    /// hidden edit/delete row).
    pub tracking_id: String,
    pub to_peer: Outbound,
    pub to_self: Option<Outbound>,
    /// Control signals that accompany the message (a request's
    /// `dm_accept`), to be published after it.
    pub followups: Vec<Outbound>,
    /// The message was a request: the peer is now my contact and the host
    /// should put them in the address book.
    pub became_contact: bool,
    pub events: Vec<UiEvent>,
}

#[derive(Clone)]
pub struct DmService {
    pub(crate) store: Store,
    pub(crate) contacts: ContactService,
    pub(crate) profiles: ProfileService,
    pub(crate) clock: Arc<dyn Clock>,
    /// Incoming messages at or after this time count as unread even when
    /// they predate the session (they arrived while we were offline).
    /// Negative: not set, the session start is used.
    pub(crate) unread_floor: Arc<AtomicI64>,
    /// Relationship gate. On by default; off means every chat behaves as
    /// `full_chat` (tests of the plain message flow, emergency switch).
    pub(crate) gate: Arc<AtomicBool>,
    /// Keys of the running session, for answers the handler must sign
    /// itself (confirm-back). `None` while locked: no answers are sent.
    pub(crate) signer: Arc<RwLock<Option<Keys>>>,
}

impl DmService {
    pub fn new(store: Store, contacts: ContactService, profiles: ProfileService, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            contacts,
            profiles,
            clock,
            unread_floor: Arc::new(AtomicI64::new(-1)),
            gate: Arc::new(AtomicBool::new(true)),
            signer: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_gate(&self, on: bool) {
        self.gate.store(on, Ordering::SeqCst);
    }

    pub fn gate_enabled(&self) -> bool {
        self.gate.load(Ordering::SeqCst)
    }

    pub fn set_signer(&self, keys: Option<Keys>) {
        *self.signer.write().unwrap() = keys;
    }

    /// See `unread_floor`. The runtime sets it to the end of the previous
    /// session; a fresh database keeps the default, so restored history is
    /// not flagged as unread.
    pub fn set_unread_floor(&self, at: i64) {
        self.unread_floor.store(at, Ordering::SeqCst);
    }

    pub fn unread_floor(&self) -> i64 {
        self.unread_floor.load(Ordering::SeqCst)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    // ─── Chats ──────────────────────────────────────────────────────────────

    pub async fn list_chats(&self, include_archived: bool) -> Result<Vec<ChatView>> {
        let rows = chats::list(&self.store, include_archived).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(self.chat_view(r).await?);
        }
        Ok(out)
    }

    pub async fn chat(&self, chat_id: &str) -> Result<Option<ChatView>> {
        match chats::get(&self.store, chat_id).await? {
            Some(r) => Ok(Some(self.chat_view(r).await?)),
            None => Ok(None),
        }
    }

    /// Open (creating if needed) the DM chat with `peer`.
    pub async fn open_chat(&self, peer: &PubKey) -> Result<ChatView> {
        let row = chats::ensure_dm(&self.store, peer.as_hex()).await?;
        self.chat_view(row).await
    }

    pub async fn mark_read(&self, chat_id: &str) -> Result<()> {
        chats::mark_read(&self.store, chat_id).await
    }

    pub async fn set_pinned(&self, chat_id: &str, pinned: bool) -> Result<()> {
        chats::set_pinned(&self.store, chat_id, pinned).await
    }

    pub async fn set_archived(&self, chat_id: &str, archived: bool) -> Result<()> {
        chats::set_archived(&self.store, chat_id, archived).await
    }

    /// For direct chats and groups alike: nothing from a muted chat makes a sound.
    pub async fn set_muted(&self, chat_id: &str, muted: bool) -> Result<()> {
        chats::set_muted(&self.store, chat_id, muted).await
    }

    pub async fn delete_chat(&self, chat_id: &str) -> Result<()> {
        chats::delete(&self.store, chat_id).await
    }

    pub async fn total_unread(&self) -> Result<i64> {
        chats::total_unread(&self.store).await
    }

    async fn chat_view(&self, r: ChatRow) -> Result<ChatView> {
        let peer = r.peer_pubkey.as_deref().and_then(PubKey::parse);
        let (mut title, mut picture, mut is_contact) = (String::new(), None, false);
        let mut npub = None;
        if let Some(pk) = &peer {
            npub = PublicKey::from_hex(pk.as_hex()).ok().and_then(|p| p.to_bech32().ok());
            if let Some(c) = self.contacts.get(pk).await? {
                title = c.label();
                picture = c.profile.as_ref().and_then(|p| p.picture.clone());
                is_contact = true;
            } else if let Some(p) = self.profiles.get(pk).await? {
                title = p.label();
                picture = p.picture.clone();
            }
            if title.trim().is_empty() {
                let n = npub.clone().unwrap_or_else(|| pk.as_hex().to_string());
                title = format!("{}…{}", &n[..12.min(n.len())], &n[n.len().saturating_sub(4)..]);
            }
        }
        let (mode, can_send) = match &peer {
            Some(pk) => self.mode_of(&r.id, pk).await?,
            None => (crate::relationship::ScreenMode::FullChat, true),
        };
        Ok(ChatView {
            id: r.id,
            kind: r.kind,
            peer_pubkey: r.peer_pubkey,
            peer_npub: npub,
            title,
            picture,
            is_contact,
            is_muted: r.muted,
            unread: r.unread,
            last_message_at: r.last_message_at,
            last_preview: r.last_preview,
            pinned: r.pinned,
            archived: r.archived,
            mode: mode.as_str().into(),
            can_send,
        })
    }

    // ─── Messages ───────────────────────────────────────────────────────────

    pub async fn messages(&self, chat_id: &str, before: Option<i64>, limit: i64) -> Result<Vec<MessageView>> {
        let rows = repo::list(&self.store, chat_id, before, limit.clamp(1, 500)).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(self.message_view(r).await?);
        }
        Ok(out)
    }

    /// What a chat has shared in one section, newest first.
    pub async fn shared(&self, chat_id: &str, section: Section, before: Option<i64>, limit: i64) -> Result<Vec<MessageView>> {
        let rows = shared::list(&self.store, chat_id, section, before, limit.clamp(1, 500)).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(self.message_view(r).await?);
        }
        Ok(out)
    }

    /// How many messages each section of a chat holds.
    pub async fn shared_counts(&self, chat_id: &str) -> Result<Counts> {
        shared::counts(&self.store, chat_id).await
    }

    pub async fn message(&self, id: &str) -> Result<Option<MessageView>> {
        match repo::get(&self.store, id).await? {
            Some(r) => Ok(Some(self.message_view(r).await?)),
            None => Ok(None),
        }
    }

    async fn message_view(&self, r: MessageRow) -> Result<MessageView> {
        let reply = match &r.reply_to_id {
            Some(id) => repo::get(&self.store, id).await?.filter(|t| !t.is_hidden).map(|t| ReplyPreview {
                id: t.id,
                sender_pubkey: t.sender_pubkey,
                text: t.text.as_deref().map(preview),
            }),
            None => None,
        };
        Ok(MessageView::from_row(r, reply))
    }

    /// Where to deliver DMs for `peer`, best first.
    pub async fn hints(&self, peer: &PubKey) -> Result<Vec<RelayUrl>> {
        Ok(dm_routes::for_peer(&self.store, peer.as_hex())
            .await?
            .iter()
            .filter_map(|u| RelayUrl::parse(u))
            .collect())
    }

    /// Application time for the next outgoing rumor: never earlier than
    /// the newest message in the chat, so order survives clock skew and
    /// several sends within one second.
    async fn next_created_at(&self, chat_id: &str) -> Result<i64> {
        let now = self.clock.now().secs();
        Ok(match repo::last_created_at(&self.store, chat_id).await? {
            Some(last) if last >= now => last + 1,
            _ => now,
        })
    }

    async fn publish_pair(
        &self,
        keys: &Keys,
        peer: &PubKey,
        content: &str,
        created_at: i64,
        reply_to: Option<&str>,
        wake: Wake,
    ) -> Result<(String, String, Outbound, Option<Outbound>)> {
        let w = wrap_as(keys, peer, content, created_at, reply_to, wake)?;
        let hint_relays = self.hints(peer).await?;
        let wire_id = w.to_peer.id.as_hex().to_string();
        let to_peer = Outbound::PublishToInbox { recipient: peer.clone(), event: w.to_peer, hint_relays };
        let to_self = w.to_self.map(|event| Outbound::PublishOwn { event });
        Ok((w.rumor_id.as_hex().to_string(), wire_id, to_peer, to_self))
    }

    fn me(keys: &Keys) -> String {
        keys.public_key().to_hex()
    }

    /// Store an outgoing text message and build its wraps.
    pub async fn prepare_text(&self, keys: &Keys, peer: &PubKey, text: &str, reply_to: Option<&str>) -> Result<Prepared> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MessengerError::Invalid("message is empty".into()));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(MessengerError::Invalid(format!("message is longer than {} KiB", MAX_TEXT_BYTES / 1024)));
        }
        let envelope = Envelope::text(text);
        self.prepare_visible(keys, peer, envelope, repo::CT_TEXT, Some(text.to_string()), reply_to, None).await
    }

    /// Store an outgoing message of any visible type (media uses this).
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_visible(
        &self,
        keys: &Keys,
        peer: &PubKey,
        envelope: Envelope,
        content_type: &str,
        text: Option<String>,
        reply_to: Option<&str>,
        media_json: Option<String>,
    ) -> Result<Prepared> {
        let chat = chats::ensure_dm(&self.store, peer.as_hex()).await?;
        if let Some(id) = reply_to {
            match repo::get(&self.store, id).await? {
                Some(t) if t.chat_id == chat.id && !t.is_hidden => {}
                _ => return Err(MessengerError::Invalid("reply target is not in this chat".into())),
            }
        }
        let me_hex = Self::me(keys);
        let gate = self.gate_outbound(&chat.id, peer, &me_hex, content_type == repo::CT_TEXT).await?;
        let content = envelope.encode();
        let created_at = self.next_created_at(&chat.id).await?;
        let (id, wire_id, to_peer, to_self) =
            self.publish_pair(keys, peer, &content, created_at, reply_to, Wake::Peer).await?;
        repo::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat.id.clone(),
                wire_id: Some(wire_id),
                direction: repo::DIR_OUT.into(),
                status: repo::STATUS_QUEUED.into(),
                content_type: content_type.into(),
                text: text.clone(),
                envelope_json: content,
                sender_pubkey: Self::me(keys),
                reply_to_id: reply_to.map(String::from),
                target_id: None,
                created_at,
                is_hidden: false,
                outbox_local_id: None,
                media_json,
            },
        )
        .await?;
        let line = text.as_deref().map(preview).unwrap_or_else(|| format!("[{content_type}]"));
        chats::touch(&self.store, &chat.id, created_at, Some(&line), false).await?;
        let message = self.message(&id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        let (mut followups, mut events, mut became_contact) = (Vec::new(), Vec::new(), false);
        if gate.request {
            // The text is stored first, the accept follows it: a request
            // must never arrive as an accept without its message.
            let result = self.act_with(keys, peer, crate::relationship::Action::Request, true).await?;
            followups = result.outbounds;
            events = result.events;
            became_contact = true;
        }
        Ok(Prepared { message, tracking_id: id, to_peer, to_self, followups, became_contact, events })
    }

    async fn own_target(&self, keys: &Keys, message_id: &str) -> Result<(MessageRow, PubKey)> {
        let row = repo::get(&self.store, message_id)
            .await?
            .filter(|r| !r.is_hidden)
            .ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        if row.sender_pubkey != Self::me(keys) {
            return Err(MessengerError::Invalid("only your own messages can be changed for everyone".into()));
        }
        if self.gate_enabled() {
            if let Some(peer) = chats::get(&self.store, &row.chat_id).await?.and_then(|c| c.peer_pubkey).as_deref().and_then(PubKey::parse) {
                let r = self.load_relation(&peer).await?;
                if r.blocked || r.peer_signal == crate::relationship::PeerSignal::Blocked {
                    return Err(MessengerError::Invalid("dm_blocked".into()));
                }
            }
        }
        if row.deleted_at.is_some() {
            return Err(MessengerError::Invalid("message is deleted".into()));
        }
        let chat = chats::get(&self.store, &row.chat_id)
            .await?
            .ok_or_else(|| MessengerError::Storage("chat missing".into()))?;
        let peer = chat
            .peer_pubkey
            .as_deref()
            .and_then(PubKey::parse)
            .ok_or_else(|| MessengerError::Storage("chat has no peer".into()))?;
        Ok((row, peer))
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_hidden(
        &self,
        keys: &Keys,
        chat_id: &str,
        id: &str,
        wire_id: &str,
        content_type: &str,
        content: String,
        target: &str,
        created_at: i64,
    ) -> Result<()> {
        repo::insert(
            &self.store,
            &NewMessage {
                id: id.into(),
                chat_id: chat_id.into(),
                wire_id: Some(wire_id.into()),
                direction: repo::DIR_OUT.into(),
                status: repo::STATUS_QUEUED.into(),
                content_type: content_type.into(),
                text: None,
                envelope_json: content,
                sender_pubkey: Self::me(keys),
                reply_to_id: None,
                target_id: Some(target.into()),
                created_at,
                is_hidden: true,
                outbox_local_id: None,
                media_json: None,
            },
        )
        .await?;
        Ok(())
    }

    /// Replace the text of one of our messages, locally and for the peer.
    pub async fn prepare_edit(&self, keys: &Keys, message_id: &str, text: &str) -> Result<Prepared> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MessengerError::Invalid("message is empty".into()));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(MessengerError::Invalid("message is too long".into()));
        }
        let (row, peer) = self.own_target(keys, message_id).await?;
        if row.content_type != repo::CT_TEXT {
            return Err(MessengerError::Invalid("only text messages can be edited".into()));
        }
        let content = Envelope::edit(message_id, text).encode();
        let created_at = self.next_created_at(&row.chat_id).await?;
        // The peer's phone told of the message; the correction is for the app.
        let (id, wire_id, to_peer, to_self) =
            self.publish_pair(keys, &peer, &content, created_at, None, Wake::Nobody).await?;
        self.insert_hidden(keys, &row.chat_id, &id, &wire_id, repo::CT_EDIT, content, message_id, created_at).await?;
        repo::set_text(&self.store, message_id, text, created_at).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        let message = self.message(message_id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        Ok(Prepared { message, tracking_id: id, to_peer, to_self, followups: vec![], became_contact: false, events: vec![] })
    }

    /// Retract one of our messages for everyone.
    pub async fn prepare_delete(&self, keys: &Keys, message_id: &str) -> Result<Prepared> {
        let (row, peer) = self.own_target(keys, message_id).await?;
        let content = Envelope::delete(message_id).encode();
        let created_at = self.next_created_at(&row.chat_id).await?;
        let (id, wire_id, to_peer, to_self) =
            self.publish_pair(keys, &peer, &content, created_at, None, Wake::Nobody).await?;
        self.insert_hidden(keys, &row.chat_id, &id, &wire_id, repo::CT_DELETE, content, message_id, created_at).await?;
        repo::mark_deleted(&self.store, message_id, created_at).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        let message = self.message(message_id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        Ok(Prepared { message, tracking_id: id, to_peer, to_self, followups: vec![], became_contact: false, events: vec![] })
    }

    /// Hide a message on this device only (works for incoming ones too).
    pub async fn delete_local(&self, message_id: &str) -> Result<()> {
        let row = repo::get(&self.store, message_id)
            .await?
            .ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        repo::mark_deleted(&self.store, message_id, self.clock.now().secs()).await?;
        chats::recompute_last(&self.store, &row.chat_id).await
    }

    /// Link a row to the outbox entry that publishes it.
    pub async fn attach_outbox(&self, tracking_id: &str, local_id: &str) -> Result<()> {
        repo::set_outbox_local_id(&self.store, tracking_id, Some(local_id)).await
    }

    /// Outbox id of a failed/queued message, for a manual retry.
    pub async fn outbox_id_for_retry(&self, message_id: &str) -> Result<String> {
        let row = repo::get(&self.store, message_id)
            .await?
            .ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        let id = row.outbox_local_id.ok_or_else(|| MessengerError::Invalid("message was never queued".into()))?;
        repo::set_status(&self.store, message_id, repo::STATUS_QUEUED, None).await?;
        Ok(id)
    }

    /// Pull publish results from the outbox into message statuses.
    /// Returns UI events for every visible message that changed.
    pub async fn sync_statuses(&self) -> Result<Vec<UiEvent>> {
        let (sent, failed) = repo::sync_outbox_status(&self.store, FAILED_AFTER_ATTEMPTS).await?;
        let mut events = Vec::new();
        for id in sent.into_iter().chain(failed) {
            if let Some(row) = repo::get(&self.store, &id).await? {
                if !row.is_hidden {
                    events.push(updated(&row.chat_id, &row.id));
                }
            }
        }
        Ok(events)
    }

    // ─── Inbound ────────────────────────────────────────────────────────────

    pub async fn apply_inbound(&self, msg: DmInbound, ctx: &Context) -> Result<Vec<Effect>> {
        let me = &ctx.my_pubkey;
        let from_me = &msg.sender == me;
        let peer = if from_me {
            msg.recipients.iter().find(|p| *p != me).cloned().unwrap_or_else(|| me.clone())
        } else {
            msg.sender.clone()
        };
        let historical =
            msg.created_at < ctx.session_started_at || matches!(msg.envelope.source, EventSource::Sync { .. });
        let chat = chats::ensure_dm(&self.store, peer.as_hex()).await?;
        let id = msg.rumor_id.as_hex().to_string();
        let mut effects: Vec<Effect> = Vec::new();

        // Other NIP-17 clients send bare text; read it as a text message.
        let envelope = Envelope::parse(&msg.content).unwrap_or_else(|_| Envelope::text(&msg.content));
        let (content_type, text, hidden, target, media_json): (String, Option<String>, bool, Option<String>, Option<String>) =
            match envelope.t.as_str() {
                T_TEXT => (repo::CT_TEXT.into(), envelope.as_text().map(String::from), false, None, None),
                T_EDIT => (repo::CT_EDIT.into(), None, true, envelope.str_field("target").map(String::from), None),
                T_DELETE => (repo::CT_DELETE.into(), None, true, envelope.str_field("target").map(String::from), None),
                T_CONTROL => (repo::CT_CONTROL.into(), None, true, None, None),
                T_MEDIA => (
                    repo::CT_MEDIA.into(),
                    envelope.str_field("caption").map(String::from),
                    false,
                    None,
                    Some(serde_json::Value::Object(envelope.fields.clone()).to_string()),
                ),
                other => (other.to_string(), envelope.str_field("text").map(String::from), false, None, None),
            };

        // Relationship gate for regular messages of the peer. A copy of
        // something already stored skips it and dedups below.
        if !from_me && !hidden && repo::get(&self.store, &id).await?.is_none() {
            match self.gate_inbound(&chat.id, &peer, msg.created_at.secs(), historical).await? {
                Some(mut e) => effects.append(&mut e),
                None => return Ok(vec![]),
            }
        }

        let inserted = repo::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat.id.clone(),
                wire_id: Some(msg.envelope.wire_id.as_hex().to_string()),
                direction: if from_me { repo::DIR_OUT.into() } else { repo::DIR_IN.into() },
                status: if from_me { repo::STATUS_SENT.into() } else { repo::STATUS_RECEIVED.into() },
                content_type: content_type.clone(),
                text: text.clone(),
                envelope_json: msg.content.clone(),
                sender_pubkey: msg.sender.as_hex().to_string(),
                reply_to_id: msg.reply_to.as_ref().map(|e| e.as_hex().to_string()),
                target_id: target.clone(),
                created_at: msg.created_at.secs(),
                is_hidden: hidden,
                outbox_local_id: None,
                media_json,
            },
        )
        .await?;

        if !inserted {
            // Another copy of something we already hold. Our own self-copy
            // coming back proves a relay stored the message.
            if from_me {
                if let Some(row) = repo::get(&self.store, &id).await? {
                    if row.status == repo::STATUS_QUEUED || row.status == repo::STATUS_FAILED {
                        repo::set_status(&self.store, &id, repo::STATUS_SENT, None).await?;
                        if !row.is_hidden {
                            return Ok(vec![Effect::Emit(updated(&row.chat_id, &id))]);
                        }
                    }
                }
            }
            return Ok(vec![]);
        }

        if content_type == repo::CT_CONTROL {
            let action = envelope.str_field("action").map(String::from);
            return self
                .on_control(&chat.id, &peer, from_me, action.as_deref(), msg.created_at.secs(), historical)
                .await;
        }
        if hidden {
            let Some(target) = target else { return Ok(vec![]) };
            return self.apply_change(&chat.id, &content_type, &target, &msg, text_of(&envelope)).await;
        }

        // Edits or a delete that overtook this message on the wire.
        for p in repo::pending_for_target(&self.store, &id).await? {
            if p.sender_pubkey != msg.sender.as_hex() || p.created_at < msg.created_at.secs() {
                continue;
            }
            match p.content_type.as_str() {
                repo::CT_EDIT => {
                    if let Some(t) = Envelope::parse(&p.envelope_json).ok().and_then(|e| e.str_field("text").map(String::from)) {
                        repo::set_text(&self.store, &id, &t, p.created_at).await?;
                    }
                }
                repo::CT_DELETE => repo::mark_deleted(&self.store, &id, p.created_at).await?,
                _ => {}
            }
        }

        let line = if content_type == repo::CT_MEDIA {
            let name = envelope.str_field("name").unwrap_or("file");
            format!("📎 {}", text.as_deref().map(preview).unwrap_or_else(|| name.to_string()))
        } else {
            text.as_deref().map(preview).unwrap_or_else(|| format!("[{content_type}]"))
        };
        let live_incoming = !from_me && !historical;
        let floor = self.unread_floor.load(Ordering::SeqCst);
        let counts_unread = !from_me && (live_incoming || (floor >= 0 && msg.created_at.secs() >= floor));
        chats::touch(&self.store, &chat.id, msg.created_at.secs(), Some(&line), counts_unread).await?;
        chats::recompute_last(&self.store, &chat.id).await?;

        let view = self.message(&id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        effects.push(Effect::Emit(UiEvent {
            name: UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat.id, "message": view, "historical": historical }),
        }));
        if live_incoming && !view.deleted {
            if let Some(c) = self.chat(&chat.id).await?.filter(|c| !c.is_muted) {
                effects.push(Effect::Notify(Notice {
                    title: c.title,
                    body: crate::body::body_of(&envelope),
                    chat_id: Some(chat.id.clone()),
                    sender: Some(peer.as_hex().to_string()),
                    request: c.mode == "request_received",
                }));
            }
        }
        Ok(effects)
    }

    /// An edit or delete arrived: apply it when the target is here and
    /// belongs to the same author. Otherwise the hidden row waits.
    async fn apply_change(
        &self,
        chat_id: &str,
        content_type: &str,
        target: &str,
        msg: &DmInbound,
        new_text: Option<String>,
    ) -> Result<Vec<Effect>> {
        let Some(row) = repo::get(&self.store, target).await? else { return Ok(vec![]) };
        if row.is_hidden || row.chat_id != chat_id || row.sender_pubkey != msg.sender.as_hex() {
            return Ok(vec![]);
        }
        if row.deleted_at.is_some() {
            return Ok(vec![]);
        }
        let at = msg.created_at.secs();
        match content_type {
            repo::CT_EDIT => {
                // Last edit wins; an older edit replayed later changes nothing.
                if row.edited_at.is_some_and(|e| e > at) {
                    return Ok(vec![]);
                }
                let Some(t) = new_text.filter(|t| !t.trim().is_empty() && t.len() <= MAX_TEXT_BYTES) else {
                    return Ok(vec![]);
                };
                repo::set_text(&self.store, target, t.trim(), at).await?;
            }
            repo::CT_DELETE => repo::mark_deleted(&self.store, target, at).await?,
            _ => return Ok(vec![]),
        }
        chats::recompute_last(&self.store, chat_id).await?;
        Ok(vec![Effect::Emit(updated(chat_id, target))])
    }
}

fn text_of(e: &Envelope) -> Option<String> {
    e.str_field("text").map(String::from)
}

fn updated(chat_id: &str, message_id: &str) -> UiEvent {
    UiEvent {
        name: UI_EVENT_DM_UPDATED.into(),
        payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
    }
}

#[cfg(test)]
mod tests {
    use crate::wrap::wrap;
    use super::*;
    use messenger_core::inbound::Envelope as WireEnvelope;
    use messenger_core::outbound::WireEvent;
    use messenger_core::{EventId, Timestamp};
    use nostr::nips::nip59::UnwrappedGift;
    use nostr::prelude::Event;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0.load(Ordering::SeqCst))
        }
    }

    /// One participant: own store, own keys.
    struct Party {
        keys: Keys,
        dm: DmService,
        contacts: ContactService,
        clock: Arc<TestClock>,
        session_started_at: i64,
    }

    impl Party {
        async fn new() -> Self {
            Self::with_keys(Keys::generate()).await
        }

        async fn with_keys(keys: Keys) -> Self {
            let store = Store::open_in_memory().await.unwrap();
            let profiles = ProfileService::new(store.clone());
            let contacts = ContactService::new(store.clone(), profiles.clone());
            let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
            let dm = DmService::new(store, contacts.clone(), profiles, clock.clone());
            dm.set_gate(false);
            Self { keys, dm, contacts, clock, session_started_at: 999_000 }
        }

        fn pk(&self) -> PubKey {
            PubKey::parse(&self.keys.public_key().to_hex()).unwrap()
        }

        fn ctx(&self) -> Context {
            Context { my_pubkey: self.pk(), session_started_at: Timestamp(self.session_started_at), clock: self.clock.clone() }
        }

        /// Open a wrap the way ingress does and hand it to the service.
        async fn receive(&self, event: &WireEvent) -> Vec<Effect> {
            self.receive_from(event, false).await
        }

        async fn receive_from(&self, event: &WireEvent, via_sync: bool) -> Vec<Effect> {
            let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
            let u = UnwrappedGift::from_gift_wrap(&self.keys, &ev).expect("addressed to this party");
            let mut rumor = u.rumor.clone();
            rumor.ensure_id();
            let url = RelayUrl::parse("wss://r.example").unwrap();
            let msg = DmInbound {
                envelope: WireEnvelope {
                    wire_id: event.id.clone(),
                    source: if via_sync { EventSource::Sync { url } } else { EventSource::Relay { url } },
                    wire_created_at: Timestamp(ev.created_at.as_secs() as i64),
                },
                rumor_id: EventId::parse(&rumor.id.unwrap().to_hex()).unwrap(),
                sender: PubKey::parse(&u.sender.to_hex()).unwrap(),
                recipients: rumor
                    .tags
                    .iter()
                    .filter(|t| t.kind() == "p")
                    .filter_map(|t| t.as_slice().get(1))
                    .filter_map(|s| PubKey::parse(s))
                    .collect(),
                created_at: Timestamp(rumor.created_at.as_secs() as i64),
                content: rumor.content.clone(),
                reply_to: rumor
                    .tags
                    .iter()
                    .filter(|t| t.kind() == "e")
                    .filter_map(|t| t.as_slice().get(1))
                    .filter_map(|s| EventId::parse(s))
                    .next(),
            };
            self.dm.apply_inbound(msg, &self.ctx()).await.unwrap()
        }
    }

    fn peer_event(p: &Prepared) -> WireEvent {
        match &p.to_peer {
            Outbound::PublishToInbox { event, .. } => event.clone(),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn self_event(p: &Prepared) -> WireEvent {
        match p.to_self.as_ref().expect("self copy") {
            Outbound::PublishOwn { event } => event.clone(),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn names(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .map(|e| match e {
                Effect::Emit(u) => u.name.clone(),
                Effect::Notify { .. } => "notify".into(),
                Effect::Send(_) => "send".into(),
            })
            .collect()
    }

    #[tokio::test]
    async fn conversation_both_ways_with_reply_and_unread() {
        let alice = Party::new().await;
        let bob = Party::new().await;

        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "  hello bob ", None).await.unwrap();
        assert_eq!(p.message.text.as_deref(), Some("hello bob"));
        assert_eq!(p.message.status, "queued");
        assert_eq!(p.message.direction, "out");

        let fx = bob.receive(&peer_event(&p)).await;
        assert_eq!(names(&fx), vec!["dm.message", "notify"]);
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        assert_eq!(chat.unread, 1);
        assert_eq!(chat.last_preview.as_deref(), Some("hello bob"));
        let msgs = bob.dm.messages(&chat.id, None, 50).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, p.message.id, "same id on both sides");
        assert_eq!(msgs[0].direction, "in");

        // Same wrap from a second relay: nothing happens.
        assert!(bob.receive(&peer_event(&p)).await.is_empty());
        assert_eq!(bob.dm.open_chat(&alice.pk()).await.unwrap().unread, 1);

        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        let r = bob.dm.prepare_text(&bob.keys, &alice.pk(), "hi alice", Some(&p.message.id)).await.unwrap();
        assert_eq!(r.message.reply_to.as_ref().unwrap().text.as_deref(), Some("hello bob"));
        alice.receive(&peer_event(&r)).await;
        let a_chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        let a_msgs = alice.dm.messages(&a_chat.id, None, 50).await.unwrap();
        assert_eq!(a_msgs.iter().map(|m| m.text.clone().unwrap()).collect::<Vec<_>>(), vec!["hello bob", "hi alice"]);
        assert_eq!(a_msgs[1].reply_to.as_ref().unwrap().id, p.message.id);
        assert_eq!(a_chat.unread, 1);
        alice.dm.mark_read(&a_chat.id).await.unwrap();
        assert_eq!(alice.dm.total_unread().await.unwrap(), 0);

        assert!(alice.dm.prepare_text(&alice.keys, &bob.pk(), "x", Some(&"00".repeat(32))).await.is_err(), "unknown reply target");
        assert!(alice.dm.prepare_text(&alice.keys, &bob.pk(), "   ", None).await.is_err());
        let huge = "a".repeat(MAX_TEXT_BYTES + 1);
        assert!(alice.dm.prepare_text(&alice.keys, &bob.pk(), &huge, None).await.is_err());
    }

    #[tokio::test]
    async fn created_at_is_monotonic_within_a_chat() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let mut last = 0;
        for i in 0..5 {
            let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), &format!("m{i}"), None).await.unwrap();
            assert!(p.message.created_at > last, "strictly increasing within one second");
            last = p.message.created_at;
        }
        // A peer message from the "future" (clock skew) pushes ours after it.
        bob.clock.0.store(1_000_500, Ordering::SeqCst);
        let ahead = bob.dm.prepare_text(&bob.keys, &alice.pk(), "from the future", None).await.unwrap();
        alice.receive(&peer_event(&ahead)).await;
        let next = alice.dm.prepare_text(&alice.keys, &bob.pk(), "after", None).await.unwrap();
        assert!(next.message.created_at > ahead.message.created_at);
        let chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        let texts: Vec<_> = alice.dm.messages(&chat.id, None, 50).await.unwrap().into_iter().map(|m| m.text.unwrap()).collect();
        assert_eq!(texts.last().unwrap(), "after");
    }

    #[tokio::test]
    async fn edit_and_delete_reach_the_peer_and_forgeries_do_not() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "frist", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;

        let e = alice.dm.prepare_edit(&alice.keys, &p.message.id, "first").await.unwrap();
        assert_eq!(e.message.text.as_deref(), Some("first"));
        assert!(e.message.edited_at.is_some());
        assert_ne!(e.tracking_id, p.message.id);
        assert_eq!(names(&bob.receive(&peer_event(&e)).await), vec!["dm.updated"]);
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let m = &bob.dm.messages(&chat.id, None, 50).await.unwrap()[0];
        assert_eq!(m.text.as_deref(), Some("first"));
        assert!(m.edited_at.is_some());
        assert_eq!(chat.last_preview.as_deref(), Some("first"));
        assert_eq!(chat.unread, 1, "an edit is not a new message");

        // Bob cannot edit or delete Alice's message.
        assert!(bob.dm.prepare_edit(&bob.keys, &p.message.id, "hacked").await.is_err());
        let forged = wrap(&bob.keys, &alice.pk(), &Envelope::edit(&p.message.id, "hacked").encode(), 1_000_100, None).unwrap();
        assert!(alice.receive(&forged.to_peer).await.is_empty());
        let a_chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        assert_eq!(alice.dm.messages(&a_chat.id, None, 50).await.unwrap()[0].text.as_deref(), Some("first"));

        let d = alice.dm.prepare_delete(&alice.keys, &p.message.id).await.unwrap();
        assert!(d.message.deleted && d.message.text.is_none());
        bob.receive(&peer_event(&d)).await;
        let m = &bob.dm.messages(&chat.id, None, 50).await.unwrap()[0];
        assert!(m.deleted && m.text.is_none());
        assert!(bob.dm.open_chat(&alice.pk()).await.unwrap().last_preview.is_none());
        assert!(alice.dm.prepare_edit(&alice.keys, &p.message.id, "again").await.is_err(), "deleted stays deleted");

        // Bob may hide it locally regardless.
        let p2 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "second", None).await.unwrap();
        bob.receive(&peer_event(&p2)).await;
        bob.dm.delete_local(&p2.message.id).await.unwrap();
        assert!(bob.dm.message(&p2.message.id).await.unwrap().unwrap().deleted);
    }

    #[tokio::test]
    async fn edit_that_overtakes_its_target_is_applied_on_arrival() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "v1", None).await.unwrap();
        let e = alice.dm.prepare_edit(&alice.keys, &p.message.id, "v2").await.unwrap();
        assert!(bob.receive(&peer_event(&e)).await.is_empty(), "target unknown yet");
        bob.receive(&peer_event(&p)).await;
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let msgs = bob.dm.messages(&chat.id, None, 50).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("v2"));
    }

    #[tokio::test]
    async fn second_device_and_relogin_rebuild_history_from_copies() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let a1 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "one", None).await.unwrap();
        bob.receive(&peer_event(&a1)).await;
        bob.clock.0.store(1_000_020, Ordering::SeqCst);
        let b1 = bob.dm.prepare_text(&bob.keys, &alice.pk(), "two", None).await.unwrap();
        alice.receive(&peer_event(&b1)).await;

        // Our own self-copy coming back marks the message as sent.
        assert_eq!(names(&alice.receive(&self_event(&a1)).await), vec!["dm.updated"]);
        assert_eq!(alice.dm.message(&a1.message.id).await.unwrap().unwrap().status, "sent");
        assert!(alice.receive(&self_event(&a1)).await.is_empty(), "idempotent");

        // Alice's second device (same key, empty store) started later and
        // gets everything through history sync.
        let mut device2 = Party::with_keys(alice.keys.clone()).await;
        device2.session_started_at = 2_000_000;
        let fx = device2.receive_from(&self_event(&a1), true).await;
        assert_eq!(names(&fx), vec!["dm.message"], "history never notifies");
        device2.receive_from(&peer_event(&b1), true).await;
        let chat = device2.dm.open_chat(&bob.pk()).await.unwrap();
        assert_eq!(chat.unread, 0, "history does not count as unread");
        let msgs = device2.dm.messages(&chat.id, None, 50).await.unwrap();
        assert_eq!(msgs.iter().map(|m| (m.direction.as_str(), m.text.as_deref().unwrap())).collect::<Vec<_>>(), vec![("out", "one"), ("in", "two")]);
        assert_eq!(msgs[0].status, "sent");
        assert_eq!(msgs[0].id, a1.message.id);
    }

    #[tokio::test]
    async fn plain_text_from_other_clients_unknown_types_and_mute() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        bob.contacts.add(&bob.pk(), alice.pk().as_hex(), Some("Al")).await.unwrap();

        let bare = wrap(&alice.keys, &bob.pk(), "just text, no envelope", 1_000_001, None).unwrap();
        assert_eq!(names(&bob.receive(&bare.to_peer).await), vec!["dm.message", "notify"]);
        let sticker = wrap(&alice.keys, &bob.pk(), r#"{"v":1,"t":"sticker","pack":"p","text":"alt"}"#, 1_000_002, None).unwrap();
        bob.receive(&sticker.to_peer).await;
        let control = wrap(&alice.keys, &bob.pk(), &Envelope::control("dm_accept").encode(), 1_000_003, None).unwrap();
        assert!(bob.receive(&control.to_peer).await.is_empty(), "control rows are hidden");

        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        assert_eq!(chat.title, "Al");
        assert!(chat.is_contact);
        let msgs = bob.dm.messages(&chat.id, None, 50).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text.as_deref(), Some("just text, no envelope"));
        assert_eq!(msgs[1].content_type, "sticker");

        bob.dm.set_muted(&chat.id, true).await.unwrap();
        assert!(bob.dm.chat(&chat.id).await.unwrap().unwrap().is_muted);
        let quiet = wrap(&alice.keys, &bob.pk(), &Envelope::text("psst").encode(), 1_000_004, None).unwrap();
        assert_eq!(names(&bob.receive(&quiet.to_peer).await), vec!["dm.message"], "muted chats do not notify");
        assert_eq!(bob.dm.open_chat(&alice.pk()).await.unwrap().unread, 3);
    }

    #[tokio::test]
    async fn pagination_notes_to_self_and_chat_flags() {
        let me = Party::new().await;
        let memo = me.dm.prepare_text(&me.keys, &me.pk(), "note", None).await.unwrap();
        assert!(memo.to_self.is_none());
        assert!(me.receive(&peer_event(&memo)).await.len() <= 1);
        let chat = me.dm.open_chat(&me.pk()).await.unwrap();
        assert_eq!(me.dm.messages(&chat.id, None, 50).await.unwrap().len(), 1);
        assert_eq!(chat.unread, 0);

        let bob = Party::new().await;
        for i in 0..25 {
            me.dm.prepare_text(&me.keys, &bob.pk(), &format!("m{i}"), None).await.unwrap();
        }
        let c = me.dm.open_chat(&bob.pk()).await.unwrap();
        let page1 = me.dm.messages(&c.id, None, 10).await.unwrap();
        assert_eq!(page1.first().unwrap().text.as_deref(), Some("m15"));
        let page2 = me.dm.messages(&c.id, Some(page1[0].created_at), 10).await.unwrap();
        assert_eq!(page2.first().unwrap().text.as_deref(), Some("m5"));
        assert_eq!(page2.last().unwrap().text.as_deref(), Some("m14"));

        me.dm.set_pinned(&c.id, true).await.unwrap();
        assert_eq!(me.dm.list_chats(false).await.unwrap()[0].id, c.id);
        me.dm.set_archived(&c.id, true).await.unwrap();
        assert_eq!(me.dm.list_chats(false).await.unwrap().len(), 1);
        me.dm.delete_chat(&c.id).await.unwrap();
        assert!(me.dm.chat(&c.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn routes_become_hints() {
        use crate::handler::DmRoutesHandler;
        use messenger_core::{Handler, MetaInbound};
        let alice = Party::new().await;
        let bob = Party::new().await;
        let h = DmRoutesHandler::new(alice.dm.store().clone());
        h.handle(
            MetaInbound::DmRelays {
                author: bob.pk(),
                created_at: Timestamp(5),
                relays: vec!["wss://inbox.example/".into(), "not a url".into(), "wss://inbox.example".into()],
            },
            &alice.ctx(),
        )
        .await
        .unwrap();
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "x", None).await.unwrap();
        match p.to_peer {
            Outbound::PublishToInbox { hint_relays, recipient, .. } => {
                assert_eq!(recipient, bob.pk());
                assert_eq!(hint_relays.iter().map(|u| u.as_str()).collect::<Vec<_>>(), vec!["wss://inbox.example"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn messages_missed_while_offline_are_unread_but_silent() {
        let alice = Party::new().await;
        let mut bob = Party::new().await;
        // Bob was last online at 999_500; Alice wrote at 999_800; Bob's new
        // session starts at 1_000_000 and fetches it as history.
        bob.dm.set_unread_floor(999_500);
        bob.session_started_at = 1_000_000;
        let missed = wrap(&alice.keys, &bob.pk(), &Envelope::text("while you were away").encode(), 999_800, None).unwrap();
        let old = wrap(&alice.keys, &bob.pk(), &Envelope::text("ancient").encode(), 900_000, None).unwrap();
        assert_eq!(names(&bob.receive_from(&missed.to_peer, true).await), vec!["dm.message"], "no notification");
        bob.receive_from(&old.to_peer, true).await;
        assert_eq!(bob.dm.open_chat(&alice.pk()).await.unwrap().unread, 1, "only the missed one");
    }

    // ─── Stage 5b: the relationship matrix in motion ────────────────────────

    use crate::relationship::Action;

    async fn gated() -> Party {
        let p = Party::new().await;
        p.dm.set_gate(true);
        p.dm.set_signer(Some(p.keys.clone()));
        p
    }

    /// Wraps of a list of outbounds that the peer can open.
    fn for_peer(outs: &[Outbound]) -> Vec<WireEvent> {
        outs.iter()
            .filter_map(|o| match o {
                Outbound::PublishToInbox { event, .. } => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    fn sends(effects: &[Effect]) -> Vec<Outbound> {
        effects.iter().filter_map(|e| if let Effect::Send(o) = e { Some(o.clone()) } else { None }).collect()
    }

    async fn mode(p: &Party, peer: &Party) -> String {
        p.dm.relation(&peer.pk()).await.unwrap().mode
    }

    async fn visible(p: &Party, peer: &Party) -> Vec<String> {
        let chat = p.dm.open_chat(&peer.pk()).await.unwrap();
        p.dm.messages(&chat.id, None, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|m| if m.content_type == "system" { format!("[{}]", m.text.unwrap()) } else { m.text.unwrap_or_default() })
            .collect()
    }

    fn reason(e: MessengerError) -> String {
        match e {
            MessengerError::Invalid(s) => s,
            other => other.to_string(),
        }
    }

    /// Alice and Bob in a mutual chat.
    async fn mutual() -> (Party, Party) {
        let alice = gated().await;
        let bob = gated().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        for w in for_peer(&p.followups) {
            bob.receive(&w).await;
        }
        let acc = bob.dm.act(&bob.keys, &alice.pk(), Action::Accept).await.unwrap();
        for w in for_peer(&acc.outbounds) {
            for back in for_peer(&sends(&alice.receive(&w).await)) {
                bob.receive(&back).await;
            }
        }
        assert_eq!(mode(&alice, &bob).await, "full_chat");
        assert_eq!(mode(&bob, &alice).await, "full_chat");
        (alice, bob)
    }

    #[tokio::test]
    async fn request_accept_and_confirm_back() {
        let alice = gated().await;
        let bob = gated().await;
        assert_eq!(mode(&alice, &bob).await, "first_contact");

        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hello, may I?", None).await.unwrap();
        assert!(p.became_contact);
        let accepts = for_peer(&p.followups);
        assert_eq!(accepts.len(), 1, "the request carries one accept");
        assert_eq!(mode(&alice, &bob).await, "request_sent");
        assert_eq!(visible(&alice, &bob).await, vec!["hello, may I?", "[request_sent]"]);
        let chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        assert!(!chat.can_send, "one message until approval");
        assert_eq!(chat.last_preview.as_deref(), Some("hello, may I?"), "system lines are not previews");
        let err = alice.dm.prepare_text(&alice.keys, &bob.pk(), "and another", None).await.unwrap_err();
        assert_eq!(reason(err), "dm_waiting_approval");

        // Bob: the text, then the accept.
        let fx = bob.receive(&peer_event(&p)).await;
        assert!(names(&fx).contains(&"notify".to_string()));
        assert_eq!(mode(&bob, &alice).await, "request_received");
        assert!(bob.receive(&accepts[0]).await.iter().all(|e| !matches!(e, Effect::Send(_))), "a stranger's accept is not confirmed");
        assert_eq!(visible(&bob, &alice).await, vec!["[request_received]", "hello, may I?"], "one system line only");
        assert_eq!(reason(bob.dm.prepare_text(&bob.keys, &alice.pk(), "hi", None).await.unwrap_err()), "dm_answer_request_first");

        // A pushy sender forging a second message gets nowhere.
        let spam = wrap(&alice.keys, &bob.pk(), &Envelope::text("answer me!").encode(), 1_000_050, None).unwrap();
        assert!(bob.receive(&spam.to_peer).await.is_empty());
        assert_eq!(visible(&bob, &alice).await.len(), 2);

        // Bob accepts.
        let acc = bob.dm.act(&bob.keys, &alice.pk(), Action::Accept).await.unwrap();
        assert_eq!(acc.relation.mode, "full_chat");
        assert!(acc.relation.can_send);
        let bob_accept = for_peer(&acc.outbounds);
        assert_eq!(bob_accept.len(), 1);

        // Alice learns it, shows it once, and confirms back.
        let fx = alice.receive(&bob_accept[0]).await;
        assert_eq!(mode(&alice, &bob).await, "full_chat");
        let back = for_peer(&sends(&fx));
        assert_eq!(back.len(), 1, "confirm-back");
        assert!(visible(&alice, &bob).await.contains(&"[request_accepted]".to_string()));

        // Bob gets the confirm-back: no echo, no second line.
        let fx = bob.receive(&back[0]).await;
        assert!(sends(&fx).is_empty(), "no ping-pong");
        assert_eq!(visible(&bob, &alice).await.iter().filter(|l| *l == "[request_accepted]").count(), 1);

        // Now both talk freely.
        let m = bob.dm.prepare_text(&bob.keys, &alice.pk(), "sure", None).await.unwrap();
        assert!(m.followups.is_empty() && !m.became_contact);
        alice.receive(&peer_event(&m)).await;
        alice.dm.prepare_text(&alice.keys, &bob.pk(), "great", None).await.unwrap();
    }

    #[tokio::test]
    async fn accept_overtaking_the_request_text_does_not_lose_it() {
        let alice = gated().await;
        let bob = gated().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "the request", None).await.unwrap();
        bob.receive(&for_peer(&p.followups)[0]).await;
        assert_eq!(mode(&bob, &alice).await, "request_received");
        bob.receive(&peer_event(&p)).await;
        assert!(visible(&bob, &alice).await.contains(&"the request".to_string()));
    }

    #[tokio::test]
    async fn decline_then_a_new_request() {
        let alice = gated().await;
        let bob = gated().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        bob.receive(&for_peer(&p.followups)[0]).await;
        assert!(bob.dm.act(&alice.keys, &alice.pk(), Action::Accept).await.is_err(), "not with a foreign key on my own pubkey");

        let d = bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.unwrap();
        assert_eq!(d.relation.mode, "request_declined_by_me");
        assert!(bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.is_err(), "nothing left to decline");
        assert!(visible(&bob, &alice).await.contains(&"hi".to_string()), "the request stays visible");

        alice.receive(&for_peer(&d.outbounds)[0]).await;
        assert_eq!(mode(&alice, &bob).await, "request_declined");
        assert_eq!(reason(alice.dm.prepare_text(&alice.keys, &bob.pk(), "please", None).await.unwrap_err()), "dm_request_declined");

        // Bob changes his mind: adding Alice tells her at once.
        let again = bob.dm.act(&bob.keys, &alice.pk(), Action::Request).await.unwrap();
        assert_eq!(again.relation.mode, "request_sent");
        let fx = alice.receive(&for_peer(&again.outbounds)[0]).await;
        assert_eq!(mode(&alice, &bob).await, "full_chat", "she still has him as a contact");
        for back in for_peer(&sends(&fx)) {
            bob.receive(&back).await;
        }
        assert_eq!(mode(&bob, &alice).await, "full_chat");
    }

    #[tokio::test]
    async fn block_is_absolute_and_symmetric_and_unblock_restores() {
        let (alice, bob) = mutual().await;
        let b = alice.dm.act(&alice.keys, &bob.pk(), Action::Block).await.unwrap();
        assert_eq!(b.relation.mode, "blocked");
        assert_eq!(alice.dm.blocked_peers().await.unwrap(), vec![bob.pk().as_hex().to_string()]);
        assert_eq!(reason(alice.dm.prepare_text(&alice.keys, &bob.pk(), "x", None).await.unwrap_err()), "dm_blocked");

        // Before Bob learns about it his message is already dropped by Alice.
        let m = bob.dm.prepare_text(&bob.keys, &alice.pk(), "are you there?", None).await.unwrap();
        assert!(alice.receive(&peer_event(&m)).await.is_empty());
        assert!(!visible(&alice, &bob).await.contains(&"are you there?".to_string()));

        bob.receive(&for_peer(&b.outbounds)[0]).await;
        assert_eq!(mode(&bob, &alice).await, "blocked_by_peer");
        assert_eq!(reason(bob.dm.prepare_text(&bob.keys, &alice.pk(), "x", None).await.unwrap_err()), "dm_blocked_by_peer");
        assert_eq!(reason(bob.dm.prepare_edit(&bob.keys, &m.message.id, "y").await.unwrap_err()), "dm_blocked");

        let u = alice.dm.act(&alice.keys, &bob.pk(), Action::Unblock).await.unwrap();
        assert_eq!(u.relation.mode, "full_chat");
        let signals = for_peer(&u.outbounds);
        assert_eq!(signals.len(), 2, "unblock, then accept");
        for w in &signals {
            bob.receive(w).await;
        }
        assert_eq!(mode(&bob, &alice).await, "full_chat");
        bob.dm.prepare_text(&bob.keys, &alice.pk(), "welcome back", None).await.unwrap();
    }

    #[tokio::test]
    async fn removing_a_contact_tells_the_peer() {
        let (alice, bob) = mutual().await;
        let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
        assert_eq!(r.relation.mode, "mutual_reconnect");
        bob.receive(&for_peer(&r.outbounds)[0]).await;
        assert_eq!(mode(&bob, &alice).await, "removed_by_peer");
        assert!(visible(&bob, &alice).await.contains(&"[contact_left]".to_string()));
        assert_eq!(reason(bob.dm.prepare_text(&bob.keys, &alice.pk(), "why?", None).await.unwrap_err()), "dm_contact_removed_by_peer");
        // Bob removes her too: both sides are clean.
        let r2 = bob.dm.act(&bob.keys, &alice.pk(), Action::Remove).await.unwrap();
        assert_eq!(r2.relation.mode, "both_removed");
        assert!(for_peer(&r2.outbounds).len() == 1);
    }

    #[tokio::test]
    async fn pending_request_is_withdrawn_quietly_or_with_notice() {
        // Added silently, never wrote: removal says nothing.
        let alice = gated().await;
        let bob = gated().await;
        let add = alice.dm.act(&alice.keys, &bob.pk(), Action::Request).await.unwrap();
        assert!(add.outbounds.is_empty(), "adding a contact is silent");
        assert!(alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap().outbounds.is_empty());

        // Wrote a request, then withdrew it.
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        bob.receive(&for_peer(&p.followups)[0]).await;
        let w = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
        let sig = for_peer(&w.outbounds);
        assert_eq!(sig.len(), 1);
        // Bob already saw an explicit accept, so "cancelled" does not apply
        // over it; his request stays answerable until he decides.
        bob.receive(&sig[0]).await;
        assert_eq!(mode(&bob, &alice).await, "request_received");
    }

    #[tokio::test]
    async fn replayed_history_in_any_order_keeps_the_newest_signal() {
        let alice = gated().await;
        let mut bob = gated().await;
        bob.session_started_at = 2_000_000; // everything below is history
        bob.dm.act(&bob.keys, &alice.pk(), Action::Request).await.unwrap();
        let accept = wrap(&alice.keys, &bob.pk(), &Envelope::control("dm_accept").encode(), 100, None).unwrap();
        let block = wrap(&alice.keys, &bob.pk(), &Envelope::control("dm_block").encode(), 200, None).unwrap();
        // Newest first, as a relay may deliver.
        assert!(sends(&bob.receive_from(&block.to_peer, true).await).is_empty());
        let fx = bob.receive_from(&accept.to_peer, true).await;
        assert!(sends(&fx).is_empty(), "historical accepts are never answered");
        assert_eq!(mode(&bob, &alice).await, "blocked_by_peer");

        // And in the natural order a historical accept is applied, silently.
        let carol = gated().await;
        let mut dave = gated().await;
        dave.session_started_at = 2_000_000;
        dave.dm.act(&dave.keys, &carol.pk(), Action::Request).await.unwrap();
        let a = wrap(&carol.keys, &dave.pk(), &Envelope::control("dm_accept").encode(), 100, None).unwrap();
        assert!(sends(&dave.receive_from(&a.to_peer, true).await).is_empty());
        assert_eq!(mode(&dave, &carol).await, "full_chat");
    }

    #[tokio::test]
    async fn a_reply_from_a_client_without_signals_counts_as_approval() {
        let alice = gated().await;
        let bob = gated().await;
        alice.dm.prepare_text(&alice.keys, &bob.pk(), "hello from veydan", None).await.unwrap();
        assert_eq!(mode(&alice, &bob).await, "request_sent");
        let reply = wrap(&bob.keys, &alice.pk(), "hello from another app", 1_000_100, None).unwrap();
        alice.receive(&reply.to_peer).await;
        assert_eq!(mode(&alice, &bob).await, "full_chat");
        alice.dm.prepare_text(&alice.keys, &bob.pk(), "nice", None).await.unwrap();
    }

    #[tokio::test]
    async fn blocked_peer_cannot_slip_in_while_i_am_offline() {
        let (alice, mut bob) = mutual().await;
        bob.dm.act(&bob.keys, &alice.pk(), Action::Block).await.unwrap();
        // Bob goes offline at 1_000_200, Alice writes at 1_000_300, Bob's
        // next session starts at 1_001_000 and sees it as history.
        bob.dm.set_unread_floor(1_000_200);
        bob.session_started_at = 1_001_000;
        let m = wrap(&alice.keys, &bob.pk(), &Envelope::text("psst").encode(), 1_000_300, None).unwrap();
        assert!(bob.receive_from(&m.to_peer, true).await.is_empty());
        assert!(!visible(&bob, &alice).await.contains(&"psst".to_string()));
    }

    #[tokio::test]
    async fn my_other_device_learns_my_decisions_from_self_copies() {
        let alice = gated().await;
        let bob = gated().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        bob.receive(&for_peer(&p.followups)[0]).await;
        let acc = bob.dm.act(&bob.keys, &alice.pk(), Action::Accept).await.unwrap();
        let own_copy = acc.outbounds.iter().find_map(|o| match o {
            Outbound::PublishOwn { event } => Some(event.clone()),
            _ => None,
        });

        // Bob's second device saw the request and now sees Bob's accept.
        let device2 = Party::with_keys(bob.keys.clone()).await;
        device2.dm.set_gate(true);
        device2.receive(&peer_event(&p)).await;
        device2.receive(&for_peer(&p.followups)[0]).await;
        assert_eq!(device2.dm.relation(&alice.pk()).await.unwrap().mode, "request_received");
        device2.receive(&own_copy.unwrap()).await;
        assert_eq!(device2.dm.relation(&alice.pk()).await.unwrap().mode, "full_chat");
        // The first device ignores its own copy.
        assert!(bob.receive(acc.outbounds.iter().find_map(|o| match o {
            Outbound::PublishOwn { event } => Some(event),
            _ => None,
        }).unwrap()).await.is_empty());
    }

    // ─── Stage 6: attachments ───────────────────────────────────────────────

    fn media_fields(name: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "mime": "image/png", "size": 10, "kind": "image" })
    }

    #[tokio::test]
    async fn placeholder_becomes_a_message_and_reaches_the_peer() {
        let (alice, bob) = mutual().await;
        let ph = alice.dm.media_placeholder(&alice.keys, &bob.pk(), media_fields("cat.png"), Some(" look ")).await.unwrap();
        assert!(ph.id.starts_with("local:"));
        assert_eq!(ph.status, "uploading");
        assert_eq!(ph.text.as_deref(), Some("look"));
        assert_eq!(ph.media.as_ref().unwrap()["name"], "cat.png");
        assert_eq!(alice.dm.open_chat(&bob.pk()).await.unwrap().last_preview.as_deref(), Some("📎 cat.png"));

        let envelope = Envelope::new("media").with("name", "cat.png").with("caption", "look").with("key", "k");
        let mut local = media_fields("cat.png");
        local["local_path"] = "/home/a/cat.png".into();
        let p = alice.dm.media_finish(&alice.keys, &ph.id, envelope, local).await.unwrap();
        assert!(alice.dm.message(&ph.id).await.unwrap().is_none(), "placeholder is gone");
        assert_eq!(p.message.content_type, "media");
        assert_eq!(p.message.status, "queued");
        assert_eq!(p.message.media.as_ref().unwrap()["local_path"], "/home/a/cat.png");

        let fx = bob.receive(&peer_event(&p)).await;
        assert!(names(&fx).contains(&"dm.message".to_string()));
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let got = bob.dm.messages(&chat.id, None, 50).await.unwrap().pop().unwrap();
        assert_eq!(got.content_type, "media");
        assert_eq!(got.text.as_deref(), Some("look"), "caption is the text");
        assert_eq!(got.media.as_ref().unwrap()["key"], "k");
        assert!(got.media.as_ref().unwrap().get("local_path").is_none(), "paths never travel");

        bob.dm.media_set_local_path(&got.id, "/cache/cat.png").await.unwrap();
        assert_eq!(bob.dm.message(&got.id).await.unwrap().unwrap().media.unwrap()["local_path"], "/cache/cat.png");
    }

    #[tokio::test]
    async fn media_is_refused_as_a_first_message_and_discard_cleans_up() {
        let alice = gated().await;
        let bob = gated().await;
        let err = alice.dm.media_placeholder(&alice.keys, &bob.pk(), media_fields("a.png"), None).await.unwrap_err();
        assert_eq!(reason(err), "dm_first_message_must_be_text");

        let (alice, bob) = mutual().await;
        let ph = alice.dm.media_placeholder(&alice.keys, &bob.pk(), media_fields("a.png"), None).await.unwrap();
        assert!(alice.dm.media_discard("not-a-placeholder").await.is_ok());
        let real = alice.dm.prepare_text(&alice.keys, &bob.pk(), "text", None).await.unwrap();
        assert!(alice.dm.media_discard(&real.message.id).await.is_err(), "only placeholders can be discarded");
        alice.dm.media_discard(&ph.id).await.unwrap();
        assert!(alice.dm.message(&ph.id).await.unwrap().is_none());
        assert_eq!(alice.dm.open_chat(&bob.pk()).await.unwrap().last_preview.as_deref(), Some("text"));
    }
}

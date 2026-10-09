// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Chats and messages. Everything here is local state plus event
//! building; sending is the runtime's job (it enqueues the returned
//! `Outbound`s and reports the outbox id back with `attach_outbox`).

use crate::view::{preview, ChatView, MessageView, ReactionView, ReplyPreview};
use crate::wrap::{wrap_as, Wake};
use crate::view::CardView;
use crate::relations::InboundGate;
use messenger_contacts::{ContactCard, ContactService, ProfileService};
use messenger_core::envelope::{KIND_OWN_RUMOR, KIND_PEER_NOTE_RUMOR, T_CONTACT, T_CONTROL, T_DELETE, T_EDIT, T_MEDIA, T_TEXT};
use messenger_core::traits::{Notice, UiEvent};
use messenger_core::{
    Clock, Context, DmInbound, Effect, Envelope, EventSource, MessengerError, Outbound, PubKey, RelayUrl, Result,
};
use messenger_store::chats::{self, ChatRow};
use messenger_store::messages::{self as repo, MessageRow, NewMessage};
use messenger_store::shared::{self, Counts, Section};
use messenger_store::{dm_held, dm_routes, reactions, receipts, settings, Store};
use nostr::key::Keys;
use nostr::nips::nip19::ToBech32;
use nostr::prelude::PublicKey;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

pub const UI_EVENT_DM_MESSAGE: &str = "dm.message";
pub const UI_EVENT_DM_UPDATED: &str = "dm.updated";
pub const UI_EVENT_CHATS_UPDATED: &str = "chats.updated";

/// Plaintext budget of one message. NIP-44 caps the padded plaintext at
/// 64 KiB and the rumor JSON wraps the text, so stay well below.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
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
    /// A new message the user can see: its copies are tried for an hour
    /// and then given up. An edit or a deletion is tried until it leaves.
    pub expiring: bool,
}

/// What `land` shows: a message being stored, or a held one let through.
struct Landing<'a> {
    chat_id: &'a str,
    peer: &'a PubKey,
    id: &'a str,
    envelope: &'a Envelope,
    content_type: &'a str,
    text: Option<&'a str>,
    card_line: Option<String>,
    sender: &'a PubKey,
    at: i64,
    from_me: bool,
    historical: bool,
}

/// Up to when the peers of one chat have read it: the peer of a direct
/// chat (`peer`), the members of a group (`peer` is `None`).
struct Marks {
    peer: Option<String>,
    reads: Vec<(String, i64)>,
    show_read: bool,
    /// Whose reactions are shown as mine.
    me: Option<String>,
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
        chats::delete(&self.store, chat_id, self.clock.now().secs()).await
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
        self.views(chat_id, rows).await
    }

    /// What a chat has shared in one section, newest first.
    pub async fn shared(&self, chat_id: &str, section: Section, before: Option<i64>, limit: i64) -> Result<Vec<MessageView>> {
        let rows = shared::list(&self.store, chat_id, section, before, limit.clamp(1, 500)).await?;
        self.views(chat_id, rows).await
    }

    /// Views of rows of one chat: the marks and the reactions are read once
    /// for the whole list.
    async fn views(&self, chat_id: &str, rows: Vec<MessageRow>) -> Result<Vec<MessageView>> {
        let marks = self.marks(chat_id).await?;
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let mut reactions = self.reactions_of(chat_id, &ids, &marks).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let under = reactions.remove(&r.id).unwrap_or_default();
            out.push(self.message_view(r, &marks, under).await?);
        }
        Ok(out)
    }

    /// How many messages each section of a chat holds.
    pub async fn shared_counts(&self, chat_id: &str) -> Result<Counts> {
        shared::counts(&self.store, chat_id).await
    }

    pub async fn message(&self, id: &str) -> Result<Option<MessageView>> {
        match repo::get(&self.store, id).await? {
            Some(r) => {
                let chat_id = r.chat_id.clone();
                Ok(self.views(&chat_id, vec![r]).await?.pop())
            }
            None => Ok(None),
        }
    }

    /// What the peers told of a chat, read once for a whole list.
    async fn marks(&self, chat_id: &str) -> Result<Marks> {
        Ok(Marks {
            peer: chat_id.strip_prefix("dm:").map(String::from),
            reads: receipts::peer_reads(&self.store, chat_id).await?,
            show_read: settings::get_bool(&self.store, crate::notes::KEY_READ_RECEIPTS, true).await?,
            me: self.my_key().await?,
        })
    }

    /// My key: the session's, or the stored identity's while locked. `None`
    /// before there is an identity: then nothing is shown as mine.
    pub(crate) async fn my_key(&self) -> Result<Option<String>> {
        let signer = self.signer.read().unwrap().as_ref().map(Self::me);
        match signer {
            Some(me) => Ok(Some(me)),
            None => Ok(messenger_store::identity::get(&self.store).await?.map(|i| i.pubkey_hex)),
        }
    }

    /// The reactions under messages of one chat, in one read.
    async fn reactions_of(&self, chat_id: &str, ids: &[String], marks: &Marks) -> Result<HashMap<String, Vec<ReactionView>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        reactions::aggregate_many(&self.store, chat_id, ids, marks.me.as_deref().unwrap_or("")).await
    }

    async fn message_view(&self, r: MessageRow, marks: &Marks, reactions: Vec<ReactionView>) -> Result<MessageView> {
        // A message taken back for everyone or removed for me carries nothing.
        let reactions = if r.deleted_at.is_some() { vec![] } else { reactions };
        let (delivered_at, read_at, seen_by) =
            if r.direction == repo::DIR_OUT && r.content_type != repo::CT_SYSTEM { self.receipts_of(&r, marks).await? } else { (None, None, vec![]) };
        let reply = match &r.reply_to_id {
            Some(id) => repo::get(&self.store, id).await?.filter(|t| !t.is_hidden).map(|t| {
                let deleted = t.deleted_at.is_some();
                ReplyPreview {
                    media: reply_media(&t, deleted),
                    id: t.id,
                    sender_pubkey: t.sender_pubkey,
                    text: t.text.as_deref().map(preview),
                    deleted,
                    content_type: t.content_type,
                }
            }),
            None => None,
        };
        let queued_at = match (&r.outbox_local_id, r.status == repo::STATUS_QUEUED) {
            (Some(local), true) => messenger_store::outbox::get(&self.store, local).await?.map(|o| o.created_at),
            _ => None,
        };
        let card = match (r.content_type == repo::CT_CONTACT, r.media_json.as_deref()) {
            (true, Some(json)) => self.card_view(json, marks.me.as_deref()).await?,
            _ => None,
        };
        Ok(MessageView { queued_at, delivered_at, read_at, seen_by, reactions, card, ..MessageView::from_row(r, reply) })
    }

    /// A stored card as the UI shows it, with what this side knows of the
    /// person. `None` when the row holds no card.
    async fn card_view(&self, json: &str, me: Option<&str>) -> Result<Option<CardView>> {
        let Ok(card) = serde_json::from_str::<ContactCard>(json) else { return Ok(None) };
        let Some(pk) = PubKey::parse(&card.pubkey) else { return Ok(None) };
        let is_contact = self.contacts.is_contact(&pk).await?;
        let blocked = self.load_relation(&pk).await?.blocked;
        Ok(Some(CardView::new(&card, me == Some(card.pubkey.as_str()), is_contact, blocked)))
    }

    /// The marks of one message of mine. A read mark says delivered too.
    /// With read receipts off, nobody is shown to have read anything.
    async fn receipts_of(&self, r: &MessageRow, marks: &Marks) -> Result<(Option<i64>, Option<i64>, Vec<String>)> {
        match &marks.peer {
            Some(peer) => {
                let mark = marks.reads.iter().find(|(m, _)| m == peer).map(|(_, at)| *at).filter(|at| *at >= r.created_at);
                let delivered = receipts::delivered_at(&self.store, &r.id, peer).await?.or(mark);
                Ok((delivered, mark.filter(|_| marks.show_read), vec![]))
            }
            None => {
                let seen: Vec<&(String, i64)> =
                    marks.reads.iter().filter(|(m, at)| *m != r.sender_pubkey && *at >= r.created_at).collect();
                let newest = seen.iter().map(|(_, at)| *at).max();
                if !marks.show_read {
                    return Ok((newest, None, vec![]));
                }
                Ok((newest, newest, seen.into_iter().map(|(m, _)| m.clone()).collect()))
            }
        }
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
    /// several sends within one second. After the chat was deleted here, it
    /// is later than the deletion, which history sync drops.
    pub(crate) async fn next_created_at(&self, chat_id: &str) -> Result<i64> {
        let now = self.clock.now().secs();
        let last = repo::last_created_at(&self.store, chat_id).await?;
        let cleared = Some(chats::cleared_at(&self.store, chat_id).await?).filter(|c| *c > 0);
        Ok(match last.max(cleared) {
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
        let card = media_json.as_deref().filter(|_| content_type == repo::CT_CONTACT).and_then(|j| serde_json::from_str(j).ok());
        let line = match card {
            Some(card) => crate::view::card_line(&card),
            None => text.as_deref().map(preview).unwrap_or_else(|| format!("[{content_type}]")),
        };
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
        Ok(Prepared { message, tracking_id: id, to_peer, to_self, followups, became_contact, events, expiring: true })
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
        if !editable_row(&row) {
            return Err(MessengerError::Invalid("only text messages and captions can be edited".into()));
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
        Ok(Prepared { message, tracking_id: id, to_peer, to_self, followups: vec![], became_contact: false, events: vec![], expiring: false })
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
        Ok(Prepared { message, tracking_id: id, to_peer, to_self, followups: vec![], became_contact: false, events: vec![], expiring: false })
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
        let (sent, failed) = repo::sync_outbox_status(&self.store).await?;
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
        // A note of my other device: not a message of any chat.
        if msg.rumor_kind == KIND_OWN_RUMOR {
            if !from_me || msg.recipients.iter().any(|p| p != me) {
                return Ok(vec![]);
            }
            let Ok(envelope) = Envelope::parse(&msg.content) else { return Ok(vec![]) };
            return self.apply_own(&msg, &envelope).await;
        }
        // A note between the peer and me: not a message of the chat either.
        if msg.rumor_kind == KIND_PEER_NOTE_RUMOR {
            return self.apply_peer_note(&msg, ctx).await;
        }
        let peer = if from_me {
            msg.recipients.iter().find(|p| *p != me).cloned().unwrap_or_else(|| me.clone())
        } else {
            msg.sender.clone()
        };
        let historical =
            msg.created_at < ctx.session_started_at || matches!(msg.envelope.source, EventSource::Sync { .. });
        // The chat row is made only when something visible lands in it, so
        // a copy, a dropped message or a hidden row does not bring back a
        // chat deleted here.
        let chat_id = chats::dm_chat_id(peer.as_hex());
        let id = msg.rumor_id.as_hex().to_string();
        let mut effects: Vec<Effect> = Vec::new();

        // Other NIP-17 clients send bare text; read it as a text message.
        let mut envelope = Envelope::parse(&msg.content).unwrap_or_else(|_| Envelope::text(&msg.content));
        let mut card_line = None;
        let (content_type, text, hidden, target, media_json): (String, Option<String>, bool, Option<String>, Option<String>) =
            match envelope.t.as_str() {
                // A card that is no card is dropped; one about somebody else
                // loses its phone here, before anything keeps it.
                T_CONTACT => {
                    let Some(card) = crate::cards::received_card(&envelope, msg.sender.as_hex()) else { return Ok(vec![]) };
                    card_line = Some(crate::view::card_line(&card));
                    (repo::CT_CONTACT.into(), None, false, None, Some(card.to_json().to_string()))
                }
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
        // A card is kept as it was checked, the envelope too.
        let content = match (&card_line, &media_json) {
            (Some(_), Some(json)) => {
                envelope = Envelope::contact(serde_json::from_str(json)?);
                envelope.encode()
            }
            _ => msg.content.clone(),
        };
        // A deleted chat stays deleted: what it held (re-synced history, a
        // re-wrapped copy) is not taken again, nor my copies written up to
        // the deletion (from my other devices too). A message of the peer
        // this device never had is new whatever its clock says, so theirs
        // are dropped by id only. A signal still counts for the
        // relationship (one this device never saw); its line, dated
        // before, is not shown.
        if content_type != repo::CT_CONTROL
            && ((from_me && msg.created_at.secs() <= chats::cleared_at(&self.store, &chat_id).await?)
                || chats::was_deleted(&self.store, &id).await?)
        {
            return Ok(vec![]);
        }

        // Relationship gate for regular messages of the peer. A copy of
        // something already stored skips it and dedups below.
        let mut held = false;
        let mut regate = false;
        if !from_me && !hidden && repo::get(&self.store, &id).await?.is_none() {
            match self.gate_inbound(&chat_id, &peer, msg.created_at.secs(), historical).await? {
                InboundGate::Pass { effects: mut e, regate: r } => {
                    effects.append(&mut e);
                    regate = r;
                }
                InboundGate::Hold => held = true,
                InboundGate::Drop => return Ok(vec![]),
            }
        }

        let inserted = repo::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: Some(msg.envelope.wire_id.as_hex().to_string()),
                direction: if from_me { repo::DIR_OUT.into() } else { repo::DIR_IN.into() },
                status: if from_me { repo::STATUS_SENT.into() } else { repo::STATUS_RECEIVED.into() },
                content_type: content_type.clone(),
                text: text.clone(),
                envelope_json: content,
                sender_pubkey: msg.sender.as_hex().to_string(),
                reply_to_id: msg.reply_to.as_ref().map(|e| e.as_hex().to_string()),
                target_id: target.clone(),
                created_at: msg.created_at.secs(),
                is_hidden: hidden || held,
                outbox_local_id: None,
                media_json,
            },
        )
        .await?;

        if !inserted {
            // Another copy of something we already hold. Our own self-copy
            // coming back proves a relay stored it, but not that the peer's
            // copy left: a message the outbox tracks follows the outbox.
            // One it does not (written on another device of mine) is sent.
            if from_me {
                if let Some(row) = repo::get(&self.store, &id).await? {
                    let waiting = row.status == repo::STATUS_QUEUED || row.status == repo::STATUS_FAILED;
                    if waiting && row.outbox_local_id.is_none() {
                        repo::set_status(&self.store, &id, repo::STATUS_SENT, None).await?;
                        if !row.is_hidden {
                            return Ok(vec![Effect::Emit(updated(&row.chat_id, &id))]);
                        }
                    }
                }
            }
            return Ok(vec![]);
        }
        // Old news to the peer (history on a new login): no receipt, ever.
        if !from_me && !hidden && msg.created_at.secs() < self.clock.now().secs() - crate::notes::RECEIPT_WINDOW_SECS {
            receipts::set_acked(&self.store, &id, msg.created_at.secs()).await?;
        }
        // Held: nothing shows, no chat is made; a later end of the episode
        // may show it (`regate`). The wrap counts as seen, as for a drop.
        if held {
            dm_held::hold(&self.store, &id, &chat_id, msg.created_at.secs()).await?;
            return Ok(vec![]);
        }

        if content_type == repo::CT_CONTROL {
            let action = envelope.str_field("action").map(String::from);
            return self
                .on_control(&chat_id, &peer, from_me, action.as_deref(), msg.created_at.secs(), historical)
                .await;
        }
        if hidden {
            let Some(target) = target else { return Ok(vec![]) };
            return self.apply_change(&chat_id, &content_type, &target, &msg, text_of(&envelope)).await;
        }

        effects.extend(
            self.land(Landing {
                chat_id: &chat_id,
                peer: &peer,
                id: &id,
                envelope: &envelope,
                content_type: &content_type,
                text: text.as_deref(),
                card_line,
                sender: &msg.sender,
                at: msg.created_at.secs(),
                from_me,
                historical,
            })
            .await?,
        );
        // The floor went up under this message (the end of an episode that
        // waited for it): a held one above it may be the new request.
        if regate {
            effects.extend(self.regate(&chat_id, &peer, !historical).await?);
        }
        Ok(effects)
    }

    /// Show a stored visible row: what overtook it, the chat row and its
    /// preview, the event and the notification. For a message as it comes
    /// and for a held one the gate lets through later, alike.
    async fn land(&self, l: Landing<'_>) -> Result<Vec<Effect>> {
        let Landing { chat_id, peer, id, envelope, content_type, text, card_line, sender, at, from_me, historical } = l;
        let mut effects = Vec::new();
        // Edits or a delete that overtook this message on the wire.
        for p in repo::pending_for_target(&self.store, id).await? {
            if p.sender_pubkey != sender.as_hex() || p.created_at < at {
                continue;
            }
            match p.content_type.as_str() {
                repo::CT_EDIT if editable_type(content_type) => {
                    if let Some(t) = Envelope::parse(&p.envelope_json).ok().and_then(|e| e.str_field("text").map(String::from)) {
                        repo::set_text(&self.store, id, &t, p.created_at).await?;
                    }
                }
                repo::CT_DELETE => repo::mark_deleted(&self.store, id, p.created_at).await?,
                _ => {}
            }
        }
        // Removed for me on another device before it came here.
        let hidden_before = self.hide_if_hidden(id).await?;

        let line = if let Some(line) = card_line {
            line
        } else if content_type == repo::CT_MEDIA {
            let name = envelope.str_field("name").unwrap_or("file");
            format!("📎 {}", text.map(preview).unwrap_or_else(|| name.to_string()))
        } else {
            text.map(preview).unwrap_or_else(|| format!("[{content_type}]"))
        };
        let live_incoming = !from_me && !historical;
        let floor = self.unread_floor.load(Ordering::SeqCst);
        let counts_unread = !from_me && !hidden_before && (live_incoming || (floor >= 0 && at >= floor));
        chats::ensure_dm(&self.store, peer.as_hex()).await?;
        chats::touch(&self.store, chat_id, at, Some(&line), counts_unread).await?;
        chats::recompute_last(&self.store, chat_id).await?;
        if from_me {
            effects.extend(self.read_elsewhere(chat_id, at).await?);
        }

        let view = self.message(id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        effects.push(Effect::Emit(UiEvent {
            name: UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": view, "historical": historical }),
        }));
        if live_incoming && !view.deleted {
            if let Some(c) = self.chat(chat_id).await?.filter(|c| !c.is_muted) {
                effects.push(Effect::Notify(Notice {
                    title: c.title,
                    body: crate::body::body_of(envelope),
                    chat_id: Some(chat_id.to_string()),
                    sender: Some(peer.as_hex().to_string()),
                    request: c.mode == "request_received",
                }));
            }
        }
        Ok(effects)
    }

    /// A held row of the peer, made visible by the gate: shown as it would
    /// have been when it came. `live` = the end of the episode that lets it
    /// through came with the running session.
    pub(crate) async fn land_row(&self, row: MessageRow, live: bool) -> Result<Vec<Effect>> {
        let Some(sender) = PubKey::parse(&row.sender_pubkey) else { return Ok(vec![]) };
        // Stored as `apply_inbound` checked it: bare text of other clients
        // as it came, a card as it was cleaned.
        let envelope = Envelope::parse(&row.envelope_json).unwrap_or_else(|_| Envelope::text(&row.envelope_json));
        let card_line = if row.content_type == repo::CT_CONTACT {
            crate::cards::received_card(&envelope, sender.as_hex()).map(|c| crate::view::card_line(&c))
        } else {
            None
        };
        self.land(Landing {
            chat_id: &row.chat_id,
            peer: &sender,
            id: &row.id,
            envelope: &envelope,
            content_type: &row.content_type,
            text: row.text.as_deref(),
            card_line,
            sender: &sender,
            at: row.created_at,
            from_me: false,
            historical: !live,
        })
        .await
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
            // Text and captions only: a card or a sticker has no text to replace.
            repo::CT_EDIT if !editable_type(&row.content_type) => return Ok(vec![]),
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

/// Rows whose text an `edit` may replace: a message or a caption.
pub fn editable_type(content_type: &str) -> bool {
    matches!(content_type, repo::CT_TEXT | repo::CT_MEDIA)
}

/// Whether my own row may be edited now. A caption is the text of a media
/// row, but not while the file uploads: the placeholder gets a new id once
/// the file is up, so nobody would ever see the one an edit names.
pub fn editable_row(row: &MessageRow) -> bool {
    match row.content_type.as_str() {
        repo::CT_TEXT => true,
        repo::CT_MEDIA => {
            row.wire_id.is_some() && row.status != repo::STATUS_UPLOADING && row.status != repo::STATUS_FAILED
        }
        _ => false,
    }
}

fn text_of(e: &Envelope) -> Option<String> {
    e.str_field("text").map(String::from)
}

/// What a quote of a media message says of it: kind, name, length. Not the
/// paths and keys of the row. Nothing for a deleted one.
fn reply_media(t: &MessageRow, deleted: bool) -> Option<serde_json::Value> {
    if deleted || t.content_type != repo::CT_MEDIA {
        return None;
    }
    let full: serde_json::Value = serde_json::from_str(t.media_json.as_deref()?).ok()?;
    let mut out = serde_json::Map::new();
    for key in ["kind", "name", "duration_ms"] {
        if let Some(v) = full.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    // The small preview of a photo or a video (at most 16 KB, checked when it
    // came), so the quote shows the picture, not only the word for it.
    let kind = full.get("kind").and_then(|v| v.as_str());
    if matches!(kind, Some("image" | "video" | "circle")) {
        if let Some(thumb) = full.get("thumb").and_then(|v| v.as_str()).filter(|t| quotable_thumb(t)) {
            out.insert("thumb".into(), thumb.into());
        }
    }
    Some(out.into())
}

/// A preview fit for a quote: base64 of a JPEG (it starts with FF D8 FF,
/// "/9j/" in base64) no longer than the media crate lets one travel (16 KB).
/// The dm crate does not depend on the media crate, so the check is repeated
/// here in its cheap form; the UI shows it only as an image.
fn quotable_thumb(thumb: &str) -> bool {
    const MAX_B64: usize = (16 * 1024usize).div_ceil(3) * 4;
    thumb.len() <= MAX_B64 && thumb.starts_with("/9j/") && thumb.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
}

pub(crate) fn updated(chat_id: &str, message_id: &str) -> UiEvent {
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
                rumor_kind: rumor.kind.as_u16(),
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

    /// The self-copy lands on my relays; the peer's copy goes elsewhere. A
    /// message whose peer copy the outbox tracks follows the outbox, not
    /// the echo: the echo would show it sent when nothing reached the peer.
    #[tokio::test]
    async fn the_echo_of_my_copy_does_not_speak_for_the_peers_copy() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        alice.dm.attach_outbox(&p.tracking_id, "outbox-1").await.unwrap();
        assert!(alice.receive(&self_event(&p)).await.is_empty());
        let m = alice.dm.message(&p.message.id).await.unwrap().unwrap();
        assert_eq!(m.status, "queued", "still waiting for the peer's copy");
        assert!(p.expiring, "a new message has a deadline");
        let e = alice.dm.prepare_edit(&alice.keys, &p.message.id, "hello").await.unwrap();
        assert!(!e.expiring, "an edit is tried until it leaves");
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

    /// A note of mine as my other device gets it.
    fn own_note(keys: &Keys, note: &Envelope, at: i64) -> WireEvent {
        crate::wrap::wrap_own(keys, &note.encode(), at).unwrap()
    }

    #[tokio::test]
    async fn what_one_device_reads_or_hides_the_other_does_too() {
        let alice = Party::new().await;
        let phone = Party::with_keys(alice.keys.clone()).await;
        let bob = Party::new().await;
        let b1 = bob.dm.prepare_text(&bob.keys, &alice.pk(), "one", None).await.unwrap();
        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        let b2 = bob.dm.prepare_text(&bob.keys, &alice.pk(), "two", None).await.unwrap();
        for p in [&b1, &b2] {
            alice.receive(&peer_event(p)).await;
            phone.receive(&peer_event(p)).await;
        }
        let chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        assert_eq!((chat.unread, phone.dm.open_chat(&bob.pk()).await.unwrap().unread), (2, 2));

        // Read on the computer: the phone is told, once.
        let read = alice.dm.mark_read(&chat.id).await.unwrap().expect("a note for the phone");
        assert_eq!(read, Envelope::own_read(&chat.id, b2.message.created_at));
        assert!(alice.dm.mark_read(&chat.id).await.unwrap().is_none(), "nothing new to tell");
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &read, 1_000_020)).await), vec!["chat.read"]);
        assert_eq!(phone.dm.open_chat(&bob.pk()).await.unwrap().unread, 0);

        // Removed for me on the computer: gone on the phone, and the list
        // shows what is left.
        let hide = alice.dm.delete_local(&b2.message.id).await.unwrap();
        assert_eq!(alice.dm.open_chat(&bob.pk()).await.unwrap().last_preview.as_deref(), Some("one"));
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &hide, 1_000_021)).await), vec!["dm.updated"]);
        assert!(phone.dm.message(&b2.message.id).await.unwrap().unwrap().deleted);
        let on_phone = phone.dm.open_chat(&bob.pk()).await.unwrap();
        assert_eq!(on_phone.last_preview.as_deref(), Some("one"));
        assert_eq!(phone.dm.list_chats(true).await.unwrap().len(), 1, "a note makes no chat with myself");
    }

    #[tokio::test]
    async fn notes_hold_for_what_comes_after_them() {
        let alice = Party::new().await;
        let phone = Party::with_keys(alice.keys.clone()).await;
        let bob = Party::new().await;
        let b1 = bob.dm.prepare_text(&bob.keys, &alice.pk(), "one", None).await.unwrap();
        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        let b2 = bob.dm.prepare_text(&bob.keys, &alice.pk(), "two", None).await.unwrap();
        bob.clock.0.store(1_000_020, Ordering::SeqCst);
        let b3 = bob.dm.prepare_text(&bob.keys, &alice.pk(), "three", None).await.unwrap();
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());

        // The phone was away: the notes come before the messages they name.
        let read = Envelope::own_read(&chat_id, b2.message.created_at);
        let hide = Envelope::own_hide(&chat_id, &b1.message.id);
        assert!(phone.receive(&own_note(&alice.keys, &read, 1_000_030)).await.is_empty());
        assert!(phone.receive(&own_note(&alice.keys, &hide, 1_000_031)).await.is_empty());
        assert!(phone.dm.list_chats(true).await.unwrap().is_empty(), "no chat before its messages");
        for p in [&b3, &b1, &b2] {
            phone.receive(&peer_event(p)).await;
        }
        let chat = phone.dm.open_chat(&bob.pk()).await.unwrap();
        assert_eq!(chat.unread, 1, "only what came after the read");
        assert!(phone.dm.message(&b1.message.id).await.unwrap().unwrap().deleted);
        assert_eq!(chat.last_preview.as_deref(), Some("three"));

        // An answer written on the computer: what came before is read.
        alice.clock.0.store(1_000_040, Ordering::SeqCst);
        alice.receive(&peer_event(&b3)).await;
        let answer = alice.dm.prepare_text(&alice.keys, &bob.pk(), "ok", None).await.unwrap();
        assert_eq!(names(&phone.receive(&self_event(&answer)).await), vec!["chat.read", "dm.message"]);
        assert_eq!(phone.dm.open_chat(&bob.pk()).await.unwrap().unread, 0);
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
        gated_with(Keys::generate()).await
    }

    /// A device as the app runs it: the relationship gate on.
    async fn gated_with(keys: Keys) -> Party {
        let p = Party::with_keys(keys).await;
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
        assert!(!d.relation.can_send, "no composer for the one who declined");
        assert!(!bob.dm.open_chat(&alice.pk()).await.unwrap().can_send);
        assert!(bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.is_err(), "nothing left to decline");
        assert!(visible(&bob, &alice).await.contains(&"hi".to_string()), "the request stays visible");

        alice.receive(&for_peer(&d.outbounds)[0]).await;
        assert_eq!(mode(&alice, &bob).await, "request_declined");
        assert_eq!(reason(alice.dm.prepare_text(&alice.keys, &bob.pk(), "please", None).await.unwrap_err()), "dm_request_declined");

        // A message from Bob must not undo his decline behind his back.
        assert_eq!(reason(bob.dm.prepare_text(&bob.keys, &alice.pk(), "changed my mind", None).await.unwrap_err()), "dm_declined_by_me");
        assert_eq!(mode(&bob, &alice).await, "request_declined_by_me");
        assert!(!visible(&bob, &alice).await.contains(&"changed my mind".to_string()));
        assert_eq!(mode(&alice, &bob).await, "request_declined", "no dm_accept leaked");

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

    /// `from` writes `text` as a request; `to` gets the text and its accept
    /// in the given order (the outbox sends them at once, and offline sync
    /// brings wraps back in any order).
    async fn deliver_request(from: &Party, to: &Party, text: &str, accept_first: bool) {
        let p = from.dm.prepare_text(&from.keys, &to.pk(), text, None).await.unwrap();
        assert!(p.became_contact, "{text}: goes out as a request");
        let accepts = for_peer(&p.followups);
        assert_eq!(accepts.len(), 1);
        if accept_first {
            to.receive(&accepts[0]).await;
            to.receive(&peer_event(&p)).await;
        } else {
            to.receive(&peer_event(&p)).await;
            to.receive(&accepts[0]).await;
        }
    }

    /// A pair with a past: Alice's new request reaches Bob in either order,
    /// stays one message, and survives his accept.
    async fn new_request_lands(alice: &Party, bob: &Party, text: &str, accept_first: bool) {
        deliver_request(alice, bob, text, accept_first).await;
        new_request_stands(alice, bob, text, &format!("{text} (accept first: {accept_first})")).await;
    }

    /// The new request reached Bob: it is shown, stays one message (more is
    /// held, not shown), and survives his accept, which purges what is held.
    async fn new_request_stands(alice: &Party, bob: &Party, text: &str, case: &str) {
        assert_eq!(mode(bob, alice).await, "request_received", "{case}");
        assert!(visible(bob, alice).await.contains(&text.to_string()), "{case}: the request text is shown");
        let spam = wrap(&alice.keys, &bob.pk(), &Envelope::text("and more").encode(), 2_000_000, None).unwrap();
        assert!(bob.receive(&spam.to_peer).await.is_empty(), "{case}: still one message per request");
        assert!(dm_held::is_held(bob.dm.store(), spam.rumor_id.as_hex()).await.unwrap(), "{case}: held");
        bob.dm.act(&bob.keys, &alice.pk(), Action::Accept).await.unwrap();
        assert_eq!(mode(bob, alice).await, "full_chat", "{case}");
        let shown = visible(bob, alice).await;
        assert!(shown.contains(&text.to_string()) && !shown.contains(&"and more".to_string()), "{case}: {shown:?}");
        assert_eq!(held_count(bob, alice).await, 0, "{case}: my accept purges what is held");
        assert!(repo::get(bob.dm.store(), spam.rumor_id.as_hex()).await.unwrap().is_none(), "{case}");
    }

    async fn held_count(p: &Party, peer: &Party) -> i64 {
        dm_held::count(p.dm.store(), &chats::dm_chat_id(peer.pk().as_hex())).await.unwrap()
    }

    #[tokio::test]
    async fn a_new_request_after_both_removed_is_delivered() {
        for accept_first in [false, true] {
            let (alice, bob) = mutual().await;
            let m = alice.dm.prepare_text(&alice.keys, &bob.pk(), "old news", None).await.unwrap();
            bob.receive(&peer_event(&m)).await;
            let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
            for w in for_peer(&r.outbounds) {
                bob.receive(&w).await;
            }
            let r = bob.dm.act(&bob.keys, &alice.pk(), Action::Remove).await.unwrap();
            for w in for_peer(&r.outbounds) {
                alice.receive(&w).await;
            }
            assert_eq!(mode(&alice, &bob).await, "both_removed");
            assert_eq!(mode(&bob, &alice).await, "both_removed");
            assert!(alice.dm.open_chat(&bob.pk()).await.unwrap().can_send, "the composer promises a new request");

            // Written before she left: still not delivered.
            let late = wrap(&alice.keys, &bob.pk(), &Envelope::text("from before").encode(), m.message.created_at + 1, None).unwrap();
            assert!(bob.receive(&late.to_peer).await.is_empty());

            new_request_lands(&alice, &bob, "again", accept_first).await;
        }
    }

    #[tokio::test]
    async fn a_new_request_after_a_withdrawn_one_is_delivered() {
        for accept_first in [false, true] {
            let alice = gated().await;
            let bob = gated().await;
            deliver_request(&alice, &bob, "hi", false).await;
            let w = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
            for s in for_peer(&w.outbounds) {
                bob.receive(&s).await;
            }
            // The withdrawal over her accept leaves his request answerable
            // (see pending_request_is_withdrawn_quietly_or_with_notice).
            assert_eq!(mode(&bob, &alice).await, "request_received");
            new_request_lands(&alice, &bob, "second try", accept_first).await;
            assert!(visible(&bob, &alice).await.contains(&"hi".to_string()), "the old request stays");
        }
    }

    #[tokio::test]
    async fn a_new_request_after_decline_and_removal_is_delivered() {
        for accept_first in [false, true] {
            let alice = gated().await;
            let bob = gated().await;
            deliver_request(&alice, &bob, "hi", false).await;
            let d = bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.unwrap();
            for s in for_peer(&d.outbounds) {
                alice.receive(&s).await;
            }
            assert_eq!(mode(&alice, &bob).await, "request_declined");
            let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
            for s in for_peer(&r.outbounds) {
                bob.receive(&s).await;
            }
            assert_eq!(mode(&bob, &alice).await, "request_declined_by_me");
            new_request_lands(&alice, &bob, "please reconsider", accept_first).await;
        }
    }

    /// A raw text of `from` to `to` (no `dm_accept`, as a plain NIP-17 or a
    /// modified client sends it) at the rumor time `at`.
    async fn bare_text(from: &Party, to: &Party, text: &str, at: i64) -> Vec<Effect> {
        let w = wrap(&from.keys, &to.pk(), &Envelope::text(text).encode(), at, None).unwrap();
        to.receive(&w.to_peer).await
    }

    fn count(shown: &[String], texts: &[&str]) -> usize {
        shown.iter().filter(|s| texts.contains(&s.as_str())).count()
    }

    const YEAR_2100: i64 = 4_102_444_800;

    #[tokio::test]
    async fn a_far_future_signal_opens_at_most_one_more_message() {
        let alice = gated().await;
        let bob = gated().await;
        bare_text(&alice, &bob, "hello", 1_000_001).await;
        assert_eq!(mode(&bob, &alice).await, "request_received");
        // A withdrawal dated 2100 over her own (implied) approval: a no-op
        // that must not lift the limit for good.
        let cancel = wrap(&alice.keys, &bob.pk(), &Envelope::control("request_cancelled").encode(), YEAR_2100, None).unwrap();
        bob.receive(&cancel.to_peer).await;
        assert_eq!(mode(&bob, &alice).await, "request_received");
        bare_text(&alice, &bob, "one", 1_000_002).await;
        bare_text(&alice, &bob, "two", 1_000_003).await;
        let shown = visible(&bob, &alice).await;
        assert_eq!(count(&shown, &["one", "two"]), 1, "{shown:?}");
        assert!(shown.contains(&"hello".to_string()));
        // "two" is held, not shown; her accept ends nothing and frees nothing.
        assert_eq!(held_count(&bob, &alice).await, 1);
        let accept = wrap(&alice.keys, &bob.pk(), &Envelope::control("dm_accept").encode(), YEAR_2100, None).unwrap();
        bob.receive(&accept.to_peer).await;
        assert_eq!(count(&visible(&bob, &alice).await, &["one", "two"]), 1);

        // Withdrawn first, before anything of hers is here: the end waits for
        // her first message and is used once, so a far-future one opens one
        // more message, not a run of backdated ones.
        let alice = gated().await;
        let bob = gated().await;
        let cancel = wrap(&alice.keys, &bob.pk(), &Envelope::control("request_cancelled").encode(), YEAR_2100, None).unwrap();
        bob.receive(&cancel.to_peer).await;
        for (i, text) in ["a", "b", "c", "d"].into_iter().enumerate() {
            bare_text(&alice, &bob, text, 1_000_001 + i as i64).await;
        }
        let shown = visible(&bob, &alice).await;
        assert_eq!(count(&shown, &["a", "b", "c", "d"]), 2, "{shown:?}");
        // Backdated below the floor: an ended episode, dropped, not held.
        let held = held_count(&bob, &alice).await;
        bare_text(&alice, &bob, "older", 1_000_000).await;
        assert_eq!(held_count(&bob, &alice).await, held);
        assert!(!visible(&bob, &alice).await.contains(&"older".to_string()));
    }

    #[tokio::test]
    async fn a_far_future_request_then_my_decline_or_removal_lets_nothing_more_through() {
        for action in [Action::Decline, Action::Remove] {
            let alice = gated().await;
            let bob = gated().await;
            bare_text(&alice, &bob, "from 2100", YEAR_2100).await;
            assert_eq!(mode(&bob, &alice).await, "request_received", "{action:?}");
            bob.dm.act(&bob.keys, &alice.pk(), action).await.unwrap();
            // After a removal the floor is capped at Bob's clock plus the
            // skew, so the row from 2100 still counts.
            bare_text(&alice, &bob, "one", 1_000_002).await;
            bare_text(&alice, &bob, "two", 1_000_003).await;
            let shown = visible(&bob, &alice).await;
            assert_eq!(count(&shown, &["from 2100", "one", "two"]), 1, "{action:?}: {shown:?}");
            assert!(shown.contains(&"from 2100".to_string()), "{action:?}");
        }
    }

    #[tokio::test]
    async fn a_bare_text_after_my_decline_is_dropped() {
        let alice = gated().await;
        let bob = gated().await;
        let device2 = gated_with(bob.keys.clone()).await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        for d in [&bob, &device2] {
            d.receive(&peer_event(&p)).await;
            d.receive(&for_peer(&p.followups)[0]).await;
        }
        let decline = bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.unwrap();
        // My other device learns the no from its copy: no floor there either.
        device2.receive(&own_copy(&decline.outbounds)).await;
        for d in [&bob, &device2] {
            assert_eq!(mode(d, &alice).await, "request_declined_by_me");
            // Her client sends no control signals: a second message is not a
            // new request, and it does not undo my no.
            assert!(bare_text(&alice, d, "but listen", 1_000_050).await.is_empty());
            assert!(!visible(d, &alice).await.contains(&"but listen".to_string()));
            assert_eq!(mode(d, &alice).await, "request_declined_by_me");
        }
    }

    /// One sync brings her new request before the withdrawal it follows
    /// (wraps come in any order): the text is held, not lost, and the late
    /// `request_cancelled` (older than the new accept) lets it through.
    #[tokio::test]
    async fn a_new_request_before_the_withdrawal_it_follows_is_delivered() {
        for accept_first in [false, true] {
            let case = format!("accept first: {accept_first}");
            let alice = gated().await;
            let bob = gated().await;
            deliver_request(&alice, &bob, "hi", false).await;
            let w = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
            let withdrawal = for_peer(&w.outbounds);
            assert_eq!(withdrawal.len(), 1, "{case}");
            deliver_request(&alice, &bob, "second try", accept_first).await;
            assert!(!visible(&bob, &alice).await.contains(&"second try".to_string()), "{case}: held until it is known to be new");
            assert_eq!(held_count(&bob, &alice).await, 1, "{case}");
            let fx = bob.receive(&withdrawal[0]).await;
            assert!(names(&fx).contains(&"dm.message".to_string()), "{case}: {:?}", names(&fx));
            assert!(fx.iter().any(|e| matches!(e, Effect::Notify(n) if n.request)), "{case}: told as a request");
            assert_eq!(held_count(&bob, &alice).await, 0, "{case}");
            assert!(visible(&bob, &alice).await.contains(&"hi".to_string()), "{case}: the old request stays");
            new_request_stands(&alice, &bob, "second try", &case).await;
        }
    }

    /// She removed me after my no; her new request comes before her
    /// removal: held, then shown when the removal comes.
    #[tokio::test]
    async fn a_new_request_before_the_removal_after_my_no_is_delivered() {
        for accept_first in [false, true] {
            let case = format!("accept first: {accept_first}");
            let alice = gated().await;
            let bob = gated().await;
            deliver_request(&alice, &bob, "hi", false).await;
            let d = bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.unwrap();
            for s in for_peer(&d.outbounds) {
                alice.receive(&s).await;
            }
            let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
            let removal = for_peer(&r.outbounds);
            assert_eq!(removal.len(), 1, "{case}");
            deliver_request(&alice, &bob, "please reconsider", accept_first).await;
            assert!(!visible(&bob, &alice).await.contains(&"please reconsider".to_string()), "{case}");
            bob.receive(&removal[0]).await;
            new_request_stands(&alice, &bob, "please reconsider", &case).await;
        }
    }

    /// Her withdrawal comes first, while nothing of hers is stored here: the
    /// first message stored after it (the old "hi", dated before it) says
    /// where that episode ended, and the new request after it is shown.
    #[tokio::test]
    async fn a_withdrawal_before_the_request_it_withdraws_still_lets_the_next_one_through() {
        for new_accept_first in [false, true] {
            let case = format!("new accept first: {new_accept_first}");
            let alice = gated().await;
            let bob = gated().await;
            let hi = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
            let w = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
            let again = alice.dm.prepare_text(&alice.keys, &bob.pk(), "second try", None).await.unwrap();
            assert!(again.became_contact, "{case}");
            bob.receive(&for_peer(&w.outbounds)[0]).await;
            if new_accept_first {
                bob.receive(&for_peer(&again.followups)[0]).await;
            }
            bob.receive(&peer_event(&hi)).await;
            bob.receive(&for_peer(&hi.followups)[0]).await;
            bob.receive(&peer_event(&again)).await;
            if !new_accept_first {
                bob.receive(&for_peer(&again.followups)[0]).await;
            }
            let shown = visible(&bob, &alice).await;
            assert!(shown.contains(&"hi".to_string()) && shown.contains(&"second try".to_string()), "{case}: {shown:?}");
            assert_eq!(held_count(&bob, &alice).await, 0, "{case}");
            new_request_stands(&alice, &bob, "second try", &case).await;
        }
    }

    /// The one message per episode holds with held rows: a third text in
    /// the same episode stays held, and a signal that ends nothing (her
    /// accept, her block and unblock) lets nothing more through.
    #[tokio::test]
    async fn a_third_text_of_the_episode_stays_held_through_signals_that_end_nothing() {
        let alice = gated().await;
        let bob = gated().await;
        deliver_request(&alice, &bob, "hi", false).await;
        let d = bob.dm.act(&bob.keys, &alice.pk(), Action::Decline).await.unwrap();
        for s in for_peer(&d.outbounds) {
            alice.receive(&s).await;
        }
        let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
        let again = alice.dm.prepare_text(&alice.keys, &bob.pk(), "please reconsider", None).await.unwrap();
        let third = wrap(&alice.keys, &bob.pk(), &Envelope::text("and also").encode(), again.message.created_at + 5, None).unwrap();
        bob.receive(&peer_event(&again)).await;
        bob.receive(&third.to_peer).await;
        assert_eq!(held_count(&bob, &alice).await, 2);
        bob.receive(&for_peer(&r.outbounds)[0]).await;
        let shown = visible(&bob, &alice).await;
        assert_eq!(count(&shown, &["please reconsider", "and also"]), 1, "{shown:?}");
        assert!(shown.contains(&"please reconsider".to_string()), "the oldest held one is the request");
        assert!(dm_held::is_held(bob.dm.store(), third.rumor_id.as_hex()).await.unwrap());

        let t = again.message.created_at + 5;
        for (i, action) in ["dm_accept", "dm_block", "dm_unblock", "dm_accept"].into_iter().enumerate() {
            let s = wrap(&alice.keys, &bob.pk(), &Envelope::control(action).encode(), t + 10 + i as i64, None).unwrap();
            bob.receive(&s.to_peer).await;
        }
        let shown = visible(&bob, &alice).await;
        assert!(!shown.contains(&"and also".to_string()), "{shown:?}");
        assert!(dm_held::is_held(bob.dm.store(), third.rumor_id.as_hex()).await.unwrap());
    }

    /// The new request came first and is shown; a text after it is held.
    /// Her late withdrawal raises the floor only to itself, the request
    /// stays above it, so nothing more is let through; her old "hi", dated
    /// before the withdrawal, is from the ended episode and is dropped.
    #[tokio::test]
    async fn a_late_withdrawal_after_the_new_request_frees_nothing_more() {
        let alice = gated().await;
        let bob = gated().await;
        let hi = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        let w = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
        let again = alice.dm.prepare_text(&alice.keys, &bob.pk(), "second try", None).await.unwrap();
        bob.receive(&peer_event(&again)).await;
        bob.receive(&for_peer(&again.followups)[0]).await;
        let more = wrap(&alice.keys, &bob.pk(), &Envelope::text("more").encode(), again.message.created_at + 5, None).unwrap();
        bob.receive(&more.to_peer).await;
        assert_eq!(held_count(&bob, &alice).await, 1);
        bob.receive(&for_peer(&w.outbounds)[0]).await;
        bob.receive(&peer_event(&hi)).await;
        let shown = visible(&bob, &alice).await;
        assert!(shown.contains(&"second try".to_string()), "{shown:?}");
        assert_eq!(count(&shown, &["hi", "more"]), 0, "{shown:?}");
        assert!(dm_held::is_held(bob.dm.store(), more.rumor_id.as_hex()).await.unwrap());
        new_request_stands(&alice, &bob, "second try", "late withdrawal").await;
    }

    /// A stranger's second text is held; my approval (here or from my other
    /// device), my block and deleting the chat purge it.
    #[tokio::test]
    async fn held_rows_are_purged_on_my_accept_block_and_chat_deletion() {
        for how in ["accept", "accept elsewhere", "block", "block elsewhere", "delete"] {
            let alice = gated().await;
            let bob = gated().await;
            let device2 = gated_with(bob.keys.clone()).await;
            for d in [&bob, &device2] {
                bare_text(&alice, d, "hello", 1_000_001).await;
                bare_text(&alice, d, "hello?", 1_000_002).await;
                assert_eq!(held_count(d, &alice).await, 1, "{how}");
                assert_eq!(mode(d, &alice).await, "request_received", "{how}");
            }
            let target = if how.ends_with("elsewhere") { &device2 } else { &bob };
            match how {
                "delete" => bob.dm.delete_chat(&chats::dm_chat_id(alice.pk().as_hex())).await.unwrap(),
                _ => {
                    let action = if how.starts_with("accept") { Action::Accept } else { Action::Block };
                    let out = bob.dm.act(&bob.keys, &alice.pk(), action).await.unwrap();
                    device2.receive(&own_copy(&out.outbounds)).await;
                }
            }
            assert_eq!(held_count(target, &alice).await, 0, "{how}");
            assert_eq!(held_count(&bob, &alice).await, 0, "{how}");
            let rows = repo::list(target.dm.store(), &chats::dm_chat_id(alice.pk().as_hex()), None, 100).await.unwrap();
            assert!(rows.iter().all(|m| m.text.as_deref() != Some("hello?")), "{how}: never shown");
        }
    }

    /// My own removal of a stranger ends the episode as theirs does: what
    /// they held back is the next request.
    #[tokio::test]
    async fn my_removal_lets_the_held_text_through_as_the_next_request() {
        let alice = gated().await;
        let bob = gated().await;
        bare_text(&alice, &bob, "hello", 1_000_001).await;
        bare_text(&alice, &bob, "hello?", 1_000_002).await;
        bare_text(&alice, &bob, "hello??", 1_000_003).await;
        let out = bob.dm.act(&bob.keys, &alice.pk(), Action::Remove).await.unwrap();
        assert!(out.events.iter().any(|e| e.name == UI_EVENT_DM_MESSAGE), "{:?}", out.events);
        let shown = visible(&bob, &alice).await;
        assert_eq!(count(&shown, &["hello", "hello?", "hello??"]), 2, "{shown:?}");
        assert!(shown.contains(&"hello?".to_string()), "the oldest held: {shown:?}");
        assert_eq!(held_count(&bob, &alice).await, 1);
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

    /// Mutual chat with history: three texts of Alice, a reply of Bob.
    async fn talked() -> (Party, Party, Vec<Prepared>, Prepared) {
        let (alice, bob) = mutual().await;
        let mut mine = Vec::new();
        for t in ["one", "two", "three"] {
            let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), t, None).await.unwrap();
            bob.receive(&peer_event(&p)).await;
            mine.push(p);
        }
        let reply = bob.dm.prepare_text(&bob.keys, &alice.pk(), "four", None).await.unwrap();
        alice.receive(&peer_event(&reply)).await;
        (alice, bob, mine, reply)
    }

    #[tokio::test]
    async fn a_deleted_chat_stays_deleted_when_its_history_comes_back() {
        let (alice, bob, mine, reply) = talked().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        alice.dm.delete_chat(&chat_id).await.unwrap();

        // A new session re-syncs my copies and Bob's wrap; a copy can also
        // come live (another device of mine).
        for p in &mine {
            assert!(alice.receive_from(&self_event(p), true).await.is_empty());
            assert!(alice.receive(&self_event(p)).await.is_empty());
        }
        assert!(alice.receive_from(&peer_event(&reply), true).await.is_empty());
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none());
        assert!(alice.dm.list_chats(true).await.unwrap().is_empty());

        // What I write after the deletion is later than it.
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "again", None).await.unwrap();
        assert!(p.message.created_at > chats::cleared_at(alice.dm.store(), &chat_id).await.unwrap());
        alice.dm.delete_chat(&chat_id).await.unwrap();

        // Bob writes after it: the chat is back with that message only.
        bob.clock.0.store(1_000_100, Ordering::SeqCst);
        let new = bob.dm.prepare_text(&bob.keys, &alice.pk(), "five", None).await.unwrap();
        alice.receive_from(&peer_event(&new), true).await;
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_some());
        assert_eq!(visible(&alice, &bob).await, vec!["five"]);
    }

    #[tokio::test]
    async fn a_message_this_device_never_had_comes_in_after_the_deletion() {
        let (alice, bob, _, reply) = talked().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        // Bob writes while Alice is away; she deletes the chat before
        // catch-up brings his message.
        let missed = bob.dm.prepare_text(&bob.keys, &alice.pk(), "are you there?", None).await.unwrap();
        alice.clock.0.store(1_000_500, Ordering::SeqCst);
        alice.dm.delete_chat(&chat_id).await.unwrap();
        assert!(missed.message.created_at < chats::cleared_at(alice.dm.store(), &chat_id).await.unwrap());

        assert!(alice.receive_from(&peer_event(&reply), true).await.is_empty(), "one she had stays gone");
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none());
        alice.receive_from(&peer_event(&missed), true).await;
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_some(), "one she never had brings the chat back");
        assert_eq!(visible(&alice, &bob).await, vec!["are you there?"]);
    }

    #[tokio::test]
    async fn a_message_dated_far_ahead_does_not_push_the_deletion_into_the_future() {
        let (alice, bob, _, _) = talked().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        bob.clock.0.store(2_000_000, Ordering::SeqCst);
        let ahead = bob.dm.prepare_text(&bob.keys, &alice.pk(), "from a clock far ahead", None).await.unwrap();
        alice.receive(&peer_event(&ahead)).await;
        alice.dm.delete_chat(&chat_id).await.unwrap();
        let cleared = chats::cleared_at(alice.dm.store(), &chat_id).await.unwrap();
        assert!(cleared <= 1_000_000 + chats::CLEARED_SKEW_SECS, "{cleared}");
        assert!(alice.receive_from(&peer_event(&ahead), true).await.is_empty(), "that message stays gone");

        // What I write next is dated by my clock, and Bob's history with a
        // right clock still comes in.
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "back", None).await.unwrap();
        assert!(p.message.created_at <= 1_000_000 + chats::CLEARED_SKEW_SECS + 1);
        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        let late = wrap(&bob.keys, &alice.pk(), &Envelope::text("on time").encode(), 1_000_010, None).unwrap();
        alice.receive_from(&late.to_peer, true).await;
        let mut seen = visible(&alice, &bob).await;
        seen.sort();
        assert_eq!(seen, vec!["back", "on time"]);
    }

    #[tokio::test]
    async fn a_request_right_after_the_deletion_keeps_its_line() {
        let alice = gated().await;
        let bob = gated().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        alice.dm.delete_chat(&chat_id).await.unwrap();
        let at = chats::cleared_at(alice.dm.store(), &chat_id).await.unwrap() + 1;
        let p = wrap(&bob.keys, &alice.pk(), &Envelope::text("hello").encode(), at, None).unwrap();
        alice.receive(&p.to_peer).await;
        assert_eq!(visible(&alice, &bob).await, vec!["[request_received]", "hello"]);
    }

    #[tokio::test]
    async fn removing_a_contact_does_not_bring_back_a_deleted_chat() {
        let (alice, bob, mine, _) = talked().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        alice.dm.delete_chat(&chat_id).await.unwrap();

        let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none());
        assert_eq!(
            (r.relation.my_contact.as_str(), r.relation.peer_signal.as_str(), r.relation.was_ever_mutual),
            ("none", "none", true)
        );
        let told = for_peer(&r.outbounds);
        assert_eq!(told.len(), 1, "the peer is still told");
        bob.receive(&told[0]).await;
        assert_eq!(mode(&bob, &alice).await, "removed_by_peer");

        // The copy of that signal and of my texts come back: still no chat.
        assert!(alice.receive_from(&own_copy(&r.outbounds), true).await.is_empty());
        for p in &mine {
            alice.receive_from(&self_event(p), true).await;
        }
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none());
        alice.dm.delete_chat(&chat_id).await.unwrap();
        assert!(alice.dm.list_chats(true).await.unwrap().is_empty());
        assert_eq!(alice.dm.relation(&bob.pk()).await.unwrap().my_contact, "none");
    }

    #[tokio::test]
    async fn a_chat_deleted_after_removing_the_contact_stays_deleted() {
        let (alice, bob, mine, reply) = talked().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        let r = alice.dm.act(&alice.keys, &bob.pk(), Action::Remove).await.unwrap();
        assert!(visible(&alice, &bob).await.contains(&"[contact_removed]".to_string()));
        alice.dm.delete_chat(&chat_id).await.unwrap();

        alice.receive_from(&own_copy(&r.outbounds), true).await;
        for p in &mine {
            alice.receive_from(&self_event(p), true).await;
        }
        alice.receive_from(&peer_event(&reply), true).await;
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_signal_from_before_the_deletion_counts_and_shows_nothing() {
        let (alice, bob, _, _) = talked().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        // Bob removes Alice while she is away; she deletes the chat later.
        let r = bob.dm.act(&bob.keys, &alice.pk(), Action::Remove).await.unwrap();
        alice.clock.0.store(1_000_500, Ordering::SeqCst);
        alice.dm.delete_chat(&chat_id).await.unwrap();
        alice.receive_from(&for_peer(&r.outbounds)[0], true).await;
        assert_eq!(mode(&alice, &bob).await, "removed_by_peer");
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none(), "its line is of the deleted past");
    }

    #[tokio::test]
    async fn a_message_the_gate_drops_makes_no_chat() {
        let alice = gated().await;
        let bob = gated().await;
        let chat_id = chats::dm_chat_id(bob.pk().as_hex());
        alice.dm.act(&alice.keys, &bob.pk(), Action::Block).await.unwrap();
        alice.dm.delete_chat(&chat_id).await.unwrap();
        bob.clock.0.store(1_000_100, Ordering::SeqCst);
        let p = wrap(&bob.keys, &alice.pk(), &Envelope::text("let me in").encode(), 1_000_100, None).unwrap();
        assert!(alice.receive(&p.to_peer).await.is_empty());
        assert!(alice.dm.chat(&chat_id).await.unwrap().is_none());
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

    fn own_copy(outs: &[Outbound]) -> WireEvent {
        outs.iter()
            .find_map(|o| match o {
                Outbound::PublishOwn { event } => Some(event.clone()),
                _ => None,
            })
            .expect("a copy for my devices")
    }

    #[tokio::test]
    async fn what_my_other_device_approves_or_removes_is_in_this_ones_book() {
        use crate::relations::UI_EVENT_DM_RELATIONSHIP;
        use messenger_contacts::UI_EVENT_CONTACTS_UPDATED;
        let alice = gated().await;
        let bob = gated().await;
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        bob.receive(&for_peer(&p.followups)[0]).await;
        let acc = bob.dm.act(&bob.keys, &alice.pk(), Action::Accept).await.unwrap();

        // Bob's desktop: the same key, a book of its own, nothing done there.
        let desktop = gated_with(bob.keys.clone()).await;
        desktop.receive(&peer_event(&p)).await;
        desktop.receive(&for_peer(&p.followups)[0]).await;
        assert!(!desktop.contacts.is_contact(&alice.pk()).await.unwrap());
        let fx = desktop.receive(&own_copy(&acc.outbounds)).await;
        assert_eq!(names(&fx), vec![UI_EVENT_DM_RELATIONSHIP, UI_EVENT_CONTACTS_UPDATED]);
        assert_eq!(mode(&desktop, &alice).await, "full_chat");
        assert!(desktop.contacts.is_contact(&alice.pk()).await.unwrap(), "in the book as on the phone");
        assert!(desktop.dm.presence_allowed(&alice.pk()).await.unwrap(), "her presence is watched and shown here too");
        assert!(desktop.receive(&own_copy(&acc.outbounds)).await.is_empty(), "a replay");

        // Removed on the phone: gone from the desktop's book too.
        bob.clock.0.store(1_000_100, Ordering::SeqCst);
        let rm = bob.dm.act(&bob.keys, &alice.pk(), Action::Remove).await.unwrap();
        let fx = desktop.receive(&own_copy(&rm.outbounds)).await;
        assert_eq!(names(&fx), vec![UI_EVENT_DM_RELATIONSHIP, UI_EVENT_CONTACTS_UPDATED]);
        assert!(!desktop.contacts.is_contact(&alice.pk()).await.unwrap());
        assert!(!desktop.dm.presence_allowed(&alice.pk()).await.unwrap());
    }

    #[tokio::test]
    async fn the_book_is_filled_once_with_whom_i_approved_before() {
        // A device from before the mirror: Bob approved, not in the book.
        let (alice, bob) = mutual().await;
        let carol = gated().await;
        alice.dm.act(&alice.keys, &carol.pk(), Action::Request).await.unwrap();
        alice.dm.act(&alice.keys, &carol.pk(), Action::Block).await.unwrap();
        assert!(!alice.contacts.is_contact(&bob.pk()).await.unwrap());

        assert_eq!(alice.dm.fill_book_once().await.unwrap(), vec![bob.pk()], "the blocked one stays out");
        assert!(alice.contacts.is_contact(&bob.pk()).await.unwrap());
        assert!(!alice.contacts.is_contact(&carol.pk()).await.unwrap());

        // Once: who is taken out of the book later stays out.
        alice.contacts.remove(&bob.pk()).await.unwrap();
        assert!(alice.dm.fill_book_once().await.unwrap().is_empty());
        assert!(!alice.contacts.is_contact(&bob.pk()).await.unwrap());
    }

    #[tokio::test]
    async fn the_fill_leaves_out_whom_i_removed_from_the_book() {
        use messenger_store::dm_relations;
        // Removed from Contacts before the matrix, the chat kept: migration
        // 007 inferred an approval from the chat (no signal of mine behind it).
        let (alice, bob) = mutual().await;
        let carol = gated().await;
        alice.dm.act(&alice.keys, &carol.pk(), Action::Request).await.unwrap();
        alice.contacts.add(&alice.pk(), bob.pk().as_hex(), None).await.unwrap();
        alice.contacts.remove(&bob.pk()).await.unwrap();
        let mut inferred = dm_relations::get(&alice.dm.store, bob.pk().as_hex()).await.unwrap().unwrap();
        assert_eq!(inferred.my_contact, "approved");
        inferred.last_my_signal_at = 0;
        dm_relations::put(&alice.dm.store, &inferred).await.unwrap();
        // Carol: removed here, then approved again on my other device.
        alice.contacts.add(&alice.pk(), carol.pk().as_hex(), None).await.unwrap();
        alice.contacts.remove(&carol.pk()).await.unwrap();
        let removed_at = messenger_store::contacts::get(&alice.dm.store, carol.pk().as_hex()).await.unwrap().unwrap().deleted_at.unwrap();
        let mut again = dm_relations::get(&alice.dm.store, carol.pk().as_hex()).await.unwrap().unwrap();
        again.last_my_signal_at = removed_at + 60;
        dm_relations::put(&alice.dm.store, &again).await.unwrap();

        assert_eq!(alice.dm.fill_book_once().await.unwrap(), vec![carol.pk()], "only the approval sent after the removal");
        assert!(!alice.contacts.is_contact(&bob.pk()).await.unwrap(), "the removal holds");
        assert!(!alice.dm.presence_allowed(&bob.pk()).await.unwrap(), "and nothing is shared with him");
        assert!(alice.contacts.is_contact(&carol.pk()).await.unwrap());
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
    async fn the_caption_of_a_sent_photo_can_be_edited_but_not_while_it_uploads() {
        let (alice, bob) = mutual().await;
        let ph = alice.dm.media_placeholder(&alice.keys, &bob.pk(), media_fields("cat.png"), Some("look")).await.unwrap();
        // The placeholder's id never reaches Bob: an edit of it would be lost.
        assert!(alice.dm.prepare_edit(&alice.keys, &ph.id, "later").await.is_err());

        let envelope = Envelope::new("media").with("kind", "image").with("name", "cat.png").with("caption", "look").with("key", "k");
        let p = alice.dm.media_finish(&alice.keys, &ph.id, envelope, media_fields("cat.png")).await.unwrap();
        bob.receive(&peer_event(&p)).await;

        let e = alice.dm.prepare_edit(&alice.keys, &p.message.id, " a cat ").await.unwrap();
        assert_eq!(e.message.text.as_deref(), Some("a cat"));
        assert_eq!(e.message.content_type, "media");
        assert!(e.message.edited_at.is_some());
        assert_eq!(names(&bob.receive(&peer_event(&e)).await), vec!["dm.updated"]);
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let got = bob.dm.message(&p.message.id).await.unwrap().unwrap();
        assert_eq!(got.text.as_deref(), Some("a cat"));
        assert!(got.edited_at.is_some());
        assert_eq!(got.media.as_ref().unwrap()["key"], "k", "the file stays");
        assert_eq!(chat.last_preview.as_deref(), Some("📎 a cat"));
    }

    #[tokio::test]
    async fn a_quote_of_a_captionless_photo_says_it_is_a_photo_not_deleted() {
        let (alice, bob) = mutual().await;
        let ph = alice.dm.media_placeholder(&alice.keys, &bob.pk(), media_fields("cat.png"), None).await.unwrap();
        // A real small JPEG preview travels with the photo.
        let thumb = "/9j/4AECAw==".to_string(); // FF D8 FF E0 01 02 03
        let envelope = Envelope::new("media").with("kind", "image").with("name", "cat.png").with("key", "k").with("thumb", thumb.as_str());
        let mut local = media_fields("cat.png");
        local["local_path"] = "/home/a/cat.png".into();
        local["thumb"] = thumb.clone().into();
        let p = alice.dm.media_finish(&alice.keys, &ph.id, envelope, local).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        assert!(p.message.text.is_none());

        // Bob quotes it; Alice receives the quote.
        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        let r = bob.dm.prepare_text(&bob.keys, &alice.pk(), "nice", Some(&p.message.id)).await.unwrap();
        alice.receive(&peer_event(&r)).await;
        let b_chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let a_chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        for quote in [
            r.message.reply_to.clone().unwrap(),
            bob.dm.messages(&b_chat.id, None, 50).await.unwrap().pop().unwrap().reply_to.unwrap(),
            alice.dm.messages(&a_chat.id, None, 50).await.unwrap().pop().unwrap().reply_to.unwrap(),
        ] {
            assert_eq!(quote.id, p.message.id);
            assert!(quote.text.is_none() && !quote.deleted);
            assert_eq!(quote.content_type, "media");
            let media = quote.media.unwrap();
            assert_eq!(media["kind"], "image");
            assert_eq!(media["name"], "cat.png");
            assert!(media.get("local_path").is_none() && media.get("key").is_none(), "only what the quote says");
            assert_eq!(media["thumb"], thumb.as_str(), "the quote shows the picture");
        }
    }

    #[test]
    fn only_a_small_jpeg_preview_goes_into_a_quote() {
        assert!(quotable_thumb("/9j/4AECAw=="));
        assert!(!quotable_thumb("iVBORw0KGgo="), "not a JPEG");
        assert!(!quotable_thumb("/9j/<script>"), "not base64");
        assert!(!quotable_thumb(&format!("/9j/{}", "A".repeat(22_000))), "too big");
    }

    #[tokio::test]
    async fn a_quote_of_a_message_taken_back_is_marked_deleted_and_shows_no_media() {
        let (alice, bob) = mutual().await;
        let ph = alice.dm.media_placeholder(&alice.keys, &bob.pk(), media_fields("cat.png"), None).await.unwrap();
        let p = alice.dm.media_finish(&alice.keys, &ph.id, Envelope::new("media").with("kind", "image").with("name", "cat.png"), media_fields("cat.png")).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        bob.dm.prepare_text(&bob.keys, &alice.pk(), "nice", Some(&p.message.id)).await.unwrap();

        let d = alice.dm.prepare_delete(&alice.keys, &p.message.id).await.unwrap();
        bob.receive(&peer_event(&d)).await;
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let quote = bob.dm.messages(&chat.id, None, 50).await.unwrap().pop().unwrap().reply_to.unwrap();
        assert!(quote.deleted && quote.text.is_none() && quote.media.is_none());
    }

    #[test]
    fn an_old_reply_preview_without_the_new_fields_still_reads() {
        let q: ReplyPreview = serde_json::from_str(r#"{"id":"a","sender_pubkey":"b","text":null}"#).unwrap();
        assert!(!q.deleted && q.content_type.is_empty() && q.media.is_none());
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

    // ─── Stage A: receipts ──────────────────────────────────────────────────

    /// A receipt as the runtime builds it: to the peer only, out of sight.
    fn receipt(from: &Party, to: &Party, note: &Envelope) -> WireEvent {
        let now = from.clock.now().secs();
        let w = crate::wrap::wrap_note(&from.keys, &to.pk(), &note.encode(), now, false, Some(now + 7 * 86_400)).unwrap();
        assert!(w.to_self.is_none(), "my devices are not told of my receipts");
        w.to_peer
    }

    async fn take_delivered(p: &Party) -> Vec<(String, String, Vec<String>)> {
        p.dm.take_due_delivered(p.clock.now().secs()).await.unwrap()
    }

    #[tokio::test]
    async fn delivered_receipt_paints_the_second_tick() {
        let (alice, bob) = mutual().await;
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "did it come?", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;

        let due = take_delivered(&bob).await;
        assert_eq!(due.len(), 1, "one note per peer");
        let (chat_id, peer, ids) = &due[0];
        assert_eq!((chat_id.as_str(), peer.as_str()), (chats::dm_chat_id(alice.pk().as_hex()).as_str(), alice.pk().as_hex()));
        assert!(ids.contains(&p.message.id));
        assert!(!ids.iter().any(|id| id.starts_with("sys:")), "system lines are nobody's messages");
        assert!(take_delivered(&bob).await.is_empty(), "taken once");

        let note = receipt(&bob, &alice, &Envelope::receipt_delivered(ids));
        assert!(alice.dm.message(&p.message.id).await.unwrap().unwrap().delivered_at.is_none());
        let fx = alice.receive(&note).await;
        assert!(names(&fx).iter().all(|n| n == "dm.updated") && !fx.is_empty());
        let m = alice.dm.message(&p.message.id).await.unwrap().unwrap();
        assert_eq!(m.delivered_at, Some(bob.clock.now().secs()));
        assert_eq!(m.read_at, None, "delivered is not read");
        assert!(alice.receive(&note).await.is_empty(), "the same receipt again changes nothing");

        // Alice's second device hears of the receipt before its copy of the message.
        let phone = Party::with_keys(alice.keys.clone()).await;
        assert!(phone.receive(&note).await.is_empty());
        phone.receive(&self_event(&p)).await;
        assert!(phone.dm.message(&p.message.id).await.unwrap().unwrap().delivered_at.is_some());

        // Bob's own messages are never marked by a receipt about them.
        let b = bob.dm.prepare_text(&bob.keys, &alice.pk(), "mine", None).await.unwrap();
        alice.receive(&peer_event(&b)).await;
        let forged = receipt(&bob, &alice, &Envelope::receipt_delivered(std::slice::from_ref(&b.message.id)));
        assert!(alice.receive(&forged).await.is_empty());
        assert!(messenger_store::receipts::delivered_at(alice.dm.store(), &b.message.id, bob.pk().as_hex()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn read_receipt_is_a_watermark() {
        let (alice, bob) = mutual().await;
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let m1 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "one", None).await.unwrap();
        let m2 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "two", None).await.unwrap();
        for p in [&m1, &m2] {
            bob.receive(&peer_event(p)).await;
        }
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        bob.dm.mark_read(&chat.id).await.unwrap();
        let m3 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "three, not read yet", None).await.unwrap();

        let due = bob.dm.take_due_read().await.unwrap();
        assert_eq!(due, vec![(chat.id.clone(), m2.message.created_at)]);
        assert!(bob.dm.take_due_read().await.unwrap().is_empty(), "taken once");

        bob.clock.0.store(1_000_200, Ordering::SeqCst);
        let note = receipt(&bob, &alice, &Envelope::receipt_read(m2.message.created_at));
        assert_eq!(names(&alice.receive(&note).await), vec!["chat.receipt"]);
        let a_chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        let list = alice.dm.messages(&a_chat.id, None, 50).await.unwrap();
        let of = |id: &str| list.iter().find(|m| m.id == id).unwrap().clone();
        assert_eq!(of(&m1.message.id).read_at, Some(m2.message.created_at));
        assert_eq!(of(&m2.message.id).read_at, Some(m2.message.created_at));
        assert_eq!(of(&m1.message.id).delivered_at, Some(m2.message.created_at), "read says delivered");
        assert_eq!(of(&m3.message.id).read_at, None);
        assert_eq!(of(&m3.message.id).delivered_at, None);
        assert!(list.iter().filter(|m| m.direction == "in").all(|m| m.read_at.is_none() && m.delivered_at.is_none()));

        // An older mark that comes late moves nothing back; a mark from the
        // future covers what is here, not what I write next.
        let late = receipt(&bob, &alice, &Envelope::receipt_read(m1.message.created_at));
        assert!(alice.receive(&late).await.is_empty());
        let ahead = receipt(&bob, &alice, &Envelope::receipt_read(9_999_999));
        assert_eq!(names(&alice.receive(&ahead).await), vec!["chat.receipt"]);
        assert!(alice.dm.message(&m3.message.id).await.unwrap().unwrap().read_at.is_some());
        alice.clock.0.store(1_000_300, Ordering::SeqCst);
        let m4 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "four", None).await.unwrap();
        assert_eq!(m4.message.read_at, None);
        assert_eq!(alice.dm.message(&m4.message.id).await.unwrap().unwrap().read_at, None);
    }

    #[tokio::test]
    async fn a_read_mark_dated_ahead_covers_nothing_i_write_later() {
        let (alice, bob) = mutual().await;
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let m1 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "one", None).await.unwrap();
        bob.receive(&peer_event(&m1)).await;

        // Bob's note says it read everything up to 2286, and is dated 2286 too.
        bob.clock.0.store(9_999_999_999, Ordering::SeqCst);
        let ahead = receipt(&bob, &alice, &Envelope::receipt_read(9_999_999_999));
        assert_eq!(names(&alice.receive(&ahead).await), vec!["chat.receipt"]);
        assert!(alice.dm.message(&m1.message.id).await.unwrap().unwrap().read_at.is_some(), "what is here is read");
        let chat = chats::dm_chat_id(bob.pk().as_hex());
        let marks = messenger_store::receipts::peer_reads(alice.dm.store(), &chat).await.unwrap();
        assert!(marks[0].1 <= 1_000_100, "no further than my clock: {marks:?}");

        alice.clock.0.store(1_000_300, Ordering::SeqCst);
        let m2 = alice.dm.prepare_text(&alice.keys, &bob.pk(), "two", None).await.unwrap();
        let shown = alice.dm.message(&m2.message.id).await.unwrap().unwrap();
        assert_eq!((shown.read_at, shown.delivered_at), (None, None), "written after the note: not read");
    }

    #[tokio::test]
    async fn a_receipt_before_my_device_knows_the_peer_still_counts() {
        let (alice, bob) = mutual().await;
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "did it come?", None).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        let (_, _, ids) = take_delivered(&bob).await.remove(0);
        bob.clock.0.store(1_000_200, Ordering::SeqCst);
        let delivered = receipt(&bob, &alice, &Envelope::receipt_delivered(&ids));
        let read = receipt(&bob, &alice, &Envelope::receipt_read(p.message.created_at));

        // Alice logs in on a new phone, gate on as in the app. History comes
        // in any order: Bob's notes before the accept and before Alice's own
        // copy of the message they name.
        let phone = gated_with(alice.keys.clone()).await;
        phone.clock.0.store(1_000_300, Ordering::SeqCst);
        assert_eq!(mode(&phone, &bob).await, "first_contact");
        assert!(phone.receive(&delivered).await.is_empty());
        assert!(phone.receive(&read).await.is_empty(), "nothing to repaint yet");
        assert!(phone.dm.list_chats(true).await.unwrap().is_empty(), "a note makes no chat");
        phone.receive(&self_event(&p)).await;
        let m = phone.dm.message(&p.message.id).await.unwrap().unwrap();
        assert_eq!(m.delivered_at, Some(1_000_200));
        assert_eq!(m.read_at, Some(p.message.created_at));
    }

    #[tokio::test]
    async fn read_receipts_obey_the_toggle_both_ways() {
        let (alice, bob) = mutual().await;
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let m = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        bob.receive(&peer_event(&m)).await;
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();

        // Bob turned them off: what he reads is not told, and turning them on
        // again does not tell it later.
        settings::set_bool(bob.dm.store(), crate::notes::KEY_READ_RECEIPTS, false).await.unwrap();
        bob.dm.mark_read(&chat.id).await.unwrap();
        assert!(bob.dm.take_due_read().await.unwrap().is_empty());
        settings::set_bool(bob.dm.store(), crate::notes::KEY_READ_RECEIPTS, true).await.unwrap();
        assert!(bob.dm.take_due_read().await.unwrap().is_empty());
        assert_eq!(take_delivered(&bob).await.len(), 1, "delivery is told whatever the toggle says");

        // Alice turned them off: what Bob tells is not kept, and no one is
        // shown to have read anything; delivered stays.
        let note = receipt(&bob, &alice, &Envelope::receipt_read(m.message.created_at));
        settings::set_bool(alice.dm.store(), crate::notes::KEY_READ_RECEIPTS, false).await.unwrap();
        assert!(alice.receive(&note).await.is_empty());
        assert!(alice.dm.message(&m.message.id).await.unwrap().unwrap().read_at.is_none());
        settings::set_bool(alice.dm.store(), crate::notes::KEY_READ_RECEIPTS, true).await.unwrap();
        let again = receipt(&bob, &alice, &Envelope::receipt_read(m.message.created_at));
        assert_eq!(names(&alice.receive(&again).await), vec!["chat.receipt"]);
        settings::set_bool(alice.dm.store(), crate::notes::KEY_READ_RECEIPTS, false).await.unwrap();
        let shown = alice.dm.message(&m.message.id).await.unwrap().unwrap();
        assert_eq!((shown.read_at, shown.delivered_at), (None, Some(m.message.created_at)), "the mark is kept, not shown");
        settings::set_bool(alice.dm.store(), crate::notes::KEY_READ_RECEIPTS, true).await.unwrap();
        assert_eq!(alice.dm.message(&m.message.id).await.unwrap().unwrap().read_at, Some(m.message.created_at));
    }

    #[tokio::test]
    async fn stale_history_is_acked_silently() {
        let alice = Party::new().await;
        let mut bob = Party::new().await;
        bob.session_started_at = 1_000_000;
        let now = bob.clock.now().secs();
        let old = wrap(&alice.keys, &bob.pk(), &Envelope::text("last month").encode(), now - 8 * 86_400, None).unwrap();
        let recent = wrap(&alice.keys, &bob.pk(), &Envelope::text("yesterday").encode(), now - 86_400, None).unwrap();
        bob.receive_from(&old.to_peer, true).await;
        bob.receive_from(&recent.to_peer, true).await;
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let ids: Vec<String> = bob.dm.messages(&chat.id, None, 50).await.unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 2);

        // Acknowledged on arrival: not owed even to a take that looks further back.
        let owed = messenger_store::receipts::take_due_delivered(bob.dm.store(), now, 0).await.unwrap();
        assert_eq!(owed, vec![(chat.id.clone(), vec![ids[1].clone()])], "only the recent one");
    }

    #[tokio::test]
    async fn a_strangers_note_shows_nothing_and_a_blocked_ones_is_dropped() {
        let alice = gated().await;
        let bob = gated().await;
        let id = "ab".repeat(32);
        for note in [Envelope::receipt_delivered(std::slice::from_ref(&id)), Envelope::receipt_read(1_000_000)] {
            assert!(alice.receive(&receipt(&bob, &alice, &note)).await.is_empty(), "nothing to repaint");
        }
        assert!(alice.dm.list_chats(true).await.unwrap().is_empty(), "a note makes no chat");

        // Nor are receipts owed to someone whose request I have not accepted,
        // and what he said marks none of his own messages.
        let p = bob.dm.prepare_text(&bob.keys, &alice.pk(), "may I?", None).await.unwrap();
        alice.receive(&peer_event(&p)).await;
        assert!(take_delivered(&alice).await.is_empty());
        let chat = alice.dm.open_chat(&bob.pk()).await.unwrap();
        let list = alice.dm.messages(&chat.id, None, 50).await.unwrap();
        assert!(list.iter().all(|m| m.delivered_at.is_none() && m.read_at.is_none()));

        // Blocked: the notes of a former friend are dropped.
        let (carol, dave) = mutual().await;
        carol.dm.act(&carol.keys, &dave.pk(), Action::Block).await.unwrap();
        for note in [Envelope::receipt_delivered(std::slice::from_ref(&id)), Envelope::receipt_read(1_000_000)] {
            assert!(carol.receive(&receipt(&dave, &carol, &note)).await.is_empty());
        }
        let store = carol.dm.store();
        assert!(messenger_store::receipts::delivered_at(store, &id, dave.pk().as_hex()).await.unwrap().is_none());
        assert!(messenger_store::receipts::peer_reads(store, &chats::dm_chat_id(dave.pk().as_hex())).await.unwrap().is_empty());
    }

    // ─── Stage B: reactions ─────────────────────────────────────────────────

    use crate::reactions::PreparedReaction;
    use crate::view::ReactionView;

    /// A device that knows whose it is, so that it can tell its own reactions.
    async fn signed(keys: Keys) -> Party {
        let p = Party::with_keys(keys).await;
        p.dm.set_signer(Some(p.keys.clone()));
        p
    }

    fn reaction_to_peer(r: &PreparedReaction) -> WireEvent {
        match &r.to_peer {
            Outbound::PublishToInbox { event, .. } => event.clone(),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn reaction_to_self(r: &PreparedReaction) -> WireEvent {
        match r.to_self.as_ref().expect("a copy for my devices") {
            Outbound::PublishOwn { event } => event.clone(),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// A reaction note as another client could write it, unchecked.
    fn reaction_note(from: &Party, to: &Party, target: &str, emoji: &str, set: bool) -> WireEvent {
        let now = from.clock.now().secs();
        crate::wrap::wrap_note(&from.keys, &to.pk(), &Envelope::reaction(target, emoji, set).encode(), now, false, None).unwrap().to_peer
    }

    async fn shown(p: &Party, id: &str) -> Vec<ReactionView> {
        p.dm.message(id).await.unwrap().expect("the message is here").reactions
    }

    fn view(emoji: &str, count: i64, mine: bool) -> ReactionView {
        ReactionView { emoji: emoji.into(), count, mine }
    }

    #[tokio::test]
    async fn reactions_toggle_and_reach_the_other_device() {
        let alice = signed(Keys::generate()).await;
        let phone = signed(alice.keys.clone()).await;
        let bob = signed(Keys::generate()).await;
        let m = bob.dm.prepare_text(&bob.keys, &alice.pk(), "lunch?", None).await.unwrap();
        alice.receive(&peer_event(&m)).await;
        phone.receive(&peer_event(&m)).await;
        bob.receive(&self_event(&m)).await;

        let r = alice.dm.prepare_reaction(&alice.keys, &m.message.id, "👍").await.unwrap();
        assert_eq!(r.chat_id, chats::dm_chat_id(bob.pk().as_hex()));
        assert_eq!(r.message.reactions, vec![view("👍", 1, true)]);
        assert_eq!(names(&bob.receive(&reaction_to_peer(&r)).await), vec!["dm.updated"]);
        assert_eq!(shown(&bob, &m.message.id).await, vec![view("👍", 1, false)]);
        assert!(bob.receive(&reaction_to_peer(&r)).await.is_empty(), "the same note again changes nothing");
        assert_eq!(names(&phone.receive(&reaction_to_self(&r)).await), vec!["dm.updated"]);
        assert_eq!(shown(&phone, &m.message.id).await, vec![view("👍", 1, true)], "my copy says it is mine");
        assert!(alice.receive(&reaction_to_self(&r)).await.is_empty(), "my own echo changes nothing");

        // Bob joins with the same emoji; his list reads two.
        let b = bob.dm.prepare_reaction(&bob.keys, &m.message.id, "👍").await.unwrap();
        assert_eq!(b.message.reactions, vec![view("👍", 2, true)]);
        alice.receive(&reaction_to_peer(&b)).await;
        phone.receive(&reaction_to_peer(&b)).await;
        assert_eq!(shown(&alice, &m.message.id).await, vec![view("👍", 2, true)]);

        // A second tap in the same second takes it back, everywhere.
        let off = alice.dm.prepare_reaction(&alice.keys, &m.message.id, "👍").await.unwrap();
        assert_eq!(off.message.reactions, vec![view("👍", 1, false)]);
        assert_eq!(Envelope::parse(&opened_content(&bob, &reaction_to_peer(&off))).unwrap(), Envelope::reaction(&m.message.id, "👍", false));
        assert_eq!(names(&bob.receive(&reaction_to_peer(&off)).await), vec!["dm.updated"]);
        assert_eq!(names(&phone.receive(&reaction_to_self(&off)).await), vec!["dm.updated"]);
        for p in [&bob, &phone] {
            assert_eq!(shown(p, &m.message.id).await, vec![view("👍", 1, p.pk() == bob.pk())]);
        }
        // The put coming late does not bring it back.
        assert!(phone.receive(&reaction_to_self(&r)).await.is_empty());
        assert_eq!(shown(&phone, &m.message.id).await, vec![view("👍", 1, false)]);

        // Counted where it was made, and nowhere else.
        assert_eq!(alice.dm.emoji_top(6).await.unwrap(), vec!["👍"]);
        assert!(phone.dm.emoji_top(6).await.unwrap().is_empty(), "the map tells the phone, not the reaction");
    }

    fn opened_content(to: &Party, event: &WireEvent) -> String {
        let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
        let u = UnwrappedGift::from_gift_wrap(&to.keys, &ev).unwrap();
        assert_eq!(u.rumor.kind.as_u16(), KIND_PEER_NOTE_RUMOR);
        u.rumor.content
    }

    #[tokio::test]
    async fn a_fourth_emoji_is_refused_here_and_not_shown_there() {
        let alice = signed(Keys::generate()).await;
        let bob = signed(Keys::generate()).await;
        let m = bob.dm.prepare_text(&bob.keys, &alice.pk(), "news", None).await.unwrap();
        alice.receive(&peer_event(&m)).await;
        bob.receive(&self_event(&m)).await;
        let id = m.message.id.clone();

        for e in ["👍", "❤️", "😂"] {
            alice.clock.0.fetch_add(1, Ordering::SeqCst);
            let r = alice.dm.prepare_reaction(&alice.keys, &id, e).await.unwrap();
            bob.receive(&reaction_to_peer(&r)).await;
        }
        assert_eq!(reason(alice.dm.prepare_reaction(&alice.keys, &id, "🔥").await.unwrap_err()), "reaction_limit");
        assert_eq!(reason(alice.dm.prepare_reaction(&alice.keys, &id, "ok").await.unwrap_err()), "reaction_invalid");
        // Three different stand on it: Bob may join one of them, not bring a fourth.
        assert_eq!(reason(bob.dm.prepare_reaction(&bob.keys, &id, "🔥").await.unwrap_err()), "reaction_limit");
        bob.clock.0.store(1_000_010, Ordering::SeqCst);
        let joined = bob.dm.prepare_reaction(&bob.keys, &id, "❤️").await.unwrap();
        assert_eq!(joined.message.reactions, vec![view("👍", 1, false), view("❤️", 2, true), view("😂", 1, false)]);

        // A client that does not keep the rules: a fourth of hers is kept
        // but not shown, while three earlier ones stand; what is no
        // reaction at all is dropped without a word.
        alice.clock.0.fetch_add(1, Ordering::SeqCst);
        bob.receive(&reaction_note(&alice, &bob, &id, "🔥", true)).await;
        assert!(bob.receive(&reaction_note(&alice, &bob, &id, "hello", true)).await.is_empty());
        assert!(bob.receive(&reaction_note(&alice, &bob, "not-an-id", "🔥", true)).await.is_empty());
        let on_bob = shown(&bob, &id).await;
        assert_eq!(on_bob.iter().map(|r| r.emoji.as_str()).collect::<Vec<_>>(), vec!["👍", "❤️", "😂"]);

        // Taking one back frees a place.
        let off = alice.dm.prepare_reaction(&alice.keys, &id, "😂").await.unwrap();
        bob.receive(&reaction_to_peer(&off)).await;
        let fire = alice.dm.prepare_reaction(&alice.keys, &id, "🔥").await.unwrap();
        bob.receive(&reaction_to_peer(&fire)).await;
        assert_eq!(shown(&bob, &id).await, vec![view("👍", 1, false), view("❤️", 2, true), view("🔥", 1, false)]);
    }

    /// Bob has three on a message, takes one back and puts another; Alice
    /// and his own phone hear the two notes in either order and agree.
    #[tokio::test]
    async fn what_an_author_shows_does_not_depend_on_the_order_notes_come_in() {
        let alice = signed(Keys::generate()).await;
        let bob = signed(Keys::generate()).await;
        let phone = signed(bob.keys.clone()).await;
        let m = alice.dm.prepare_text(&alice.keys, &bob.pk(), "plans?", None).await.unwrap();
        for p in [&bob, &phone] {
            p.receive(&peer_event(&m)).await;
        }
        alice.receive(&self_event(&m)).await;
        let id = m.message.id.clone();
        let mut notes = Vec::new();
        for e in ["👍", "❤️", "😂", "😂", "🔥"] {
            bob.clock.0.fetch_add(1, Ordering::SeqCst);
            notes.push(bob.dm.prepare_reaction(&bob.keys, &id, e).await.unwrap());
        }
        let want = vec![view("👍", 1, true), view("❤️", 1, true), view("🔥", 1, true)];
        assert_eq!(shown(&bob, &id).await, want);

        // Alice: the 🔥 before the take-back of 😂.
        for i in [0, 1, 2, 4, 3] {
            alice.receive(&reaction_to_peer(&notes[i])).await;
        }
        // The phone: in the order they were made.
        for r in &notes {
            phone.receive(&reaction_to_self(r)).await;
        }
        assert_eq!(shown(&phone, &id).await, want);
        let on_alice: Vec<_> = want.iter().map(|r| view(&r.emoji, 1, false)).collect();
        assert_eq!(shown(&alice, &id).await, on_alice);
    }

    /// A stranger's notes name ids that never come: a week later they are
    /// gone. What stands on a message that is here stays.
    #[tokio::test]
    async fn reactions_to_nothing_here_are_swept_after_a_week() {
        let alice = signed(Keys::generate()).await;
        let bob = signed(Keys::generate()).await;
        let stranger = signed(Keys::generate()).await;
        let m = bob.dm.prepare_text(&bob.keys, &alice.pk(), "hi", None).await.unwrap();
        alice.receive(&peer_event(&m)).await;
        bob.receive(&self_event(&m)).await;
        let r = bob.dm.prepare_reaction(&bob.keys, &m.message.id, "👋").await.unwrap();
        alice.receive(&reaction_to_peer(&r)).await;
        let nowhere = "cd".repeat(32);
        assert!(alice.receive(&reaction_note(&stranger, &alice, &nowhere, "👍", true)).await.is_empty());
        let in_strangers_chat = chats::dm_chat_id(stranger.pk().as_hex());
        let store = alice.dm.store();
        assert!(messenger_store::reactions::get(store, &nowhere, stranger.pk().as_hex(), "👍").await.unwrap().is_some());

        let now = alice.clock.now().secs();
        assert_eq!(alice.dm.prune_reactions_if_due(now).await.unwrap(), 0, "it may still come");
        let later = now + crate::reactions::ORPHAN_KEEP_SECS + 1;
        assert_eq!(alice.dm.prune_reactions_if_due(later).await.unwrap(), 1);
        assert!(messenger_store::reactions::get(store, &nowhere, stranger.pk().as_hex(), "👍").await.unwrap().is_none());
        assert!(alice.dm.messages(&in_strangers_chat, None, 10).await.unwrap().is_empty());
        assert_eq!(shown(&alice, &m.message.id).await, vec![view("👋", 1, false)]);

        // Once a day at most.
        alice.receive(&reaction_note(&stranger, &alice, &nowhere, "🔥", true)).await;
        assert_eq!(alice.dm.prune_reactions_if_due(later + 3600).await.unwrap(), 0);
        assert_eq!(alice.dm.prune_reactions_if_due(later + crate::reactions::PRUNE_EVERY_SECS).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_reaction_before_its_message_waits_by_id() {
        let alice = signed(Keys::generate()).await;
        let bob = signed(Keys::generate()).await;
        let m = alice.dm.prepare_text(&alice.keys, &bob.pk(), "see you", None).await.unwrap();
        bob.receive(&peer_event(&m)).await;
        let r = bob.dm.prepare_reaction(&bob.keys, &m.message.id, "🔥").await.unwrap();

        // Alice's new phone hears the reaction before its copy of the message.
        let phone = signed(alice.keys.clone()).await;
        assert!(phone.receive(&reaction_to_peer(&r)).await.is_empty(), "nothing to repaint yet");
        assert!(phone.dm.list_chats(true).await.unwrap().is_empty(), "a reaction makes no chat");
        phone.receive(&self_event(&m)).await;
        assert_eq!(shown(&phone, &m.message.id).await, vec![view("🔥", 1, false)]);
    }

    #[tokio::test]
    async fn a_reaction_names_a_message_of_another_chat_and_shows_nowhere() {
        let alice = signed(Keys::generate()).await;
        let bob = signed(Keys::generate()).await;
        let carol = signed(Keys::generate()).await;
        let from_carol = carol.dm.prepare_text(&carol.keys, &alice.pk(), "a secret", None).await.unwrap();
        alice.receive(&peer_event(&from_carol)).await;
        let from_bob = bob.dm.prepare_text(&bob.keys, &alice.pk(), "hi", None).await.unwrap();
        alice.receive(&peer_event(&from_bob)).await;

        // Bob somehow knows the id of Carol's message.
        assert!(alice.receive(&reaction_note(&bob, &alice, &from_carol.message.id, "💩", true)).await.is_empty());
        assert!(shown(&alice, &from_carol.message.id).await.is_empty());
        let in_bobs_chat = alice.dm.messages(&chats::dm_chat_id(bob.pk().as_hex()), None, 50).await.unwrap();
        assert!(in_bobs_chat.iter().all(|m| m.reactions.is_empty()));
    }

    #[tokio::test]
    async fn a_blocked_peers_reaction_is_dropped() {
        let (alice, bob) = mutual().await;
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let m = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hello", None).await.unwrap();
        bob.receive(&peer_event(&m)).await;
        alice.dm.act(&alice.keys, &bob.pk(), Action::Block).await.unwrap();

        // Bob does not know yet; what he sends is dropped all the same.
        let r = bob.dm.prepare_reaction(&bob.keys, &m.message.id, "👎").await.unwrap();
        assert!(alice.receive(&reaction_to_peer(&r)).await.is_empty());
        assert!(shown(&alice, &m.message.id).await.is_empty());
        assert!(messenger_store::reactions::active(alice.dm.store(), &m.message.id, &chats::dm_chat_id(bob.pk().as_hex()))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(reason(alice.dm.prepare_reaction(&alice.keys, &m.message.id, "👍").await.unwrap_err()), "dm_blocked");

        // Nor does one go to someone I have not agreed to talk with.
        let stranger = gated().await;
        let ask = stranger.dm.prepare_text(&stranger.keys, &alice.pk(), "may I?", None).await.unwrap();
        alice.receive(&peer_event(&ask)).await;
        let refused = reason(alice.dm.prepare_reaction(&alice.keys, &ask.message.id, "👍").await.unwrap_err());
        assert!(refused.starts_with("dm_"), "{refused}");
        assert!(shown(&alice, &ask.message.id).await.is_empty());
    }

    #[tokio::test]
    async fn a_deleted_message_shows_no_reactions_and_takes_none() {
        let alice = signed(Keys::generate()).await;
        let bob = signed(Keys::generate()).await;
        let m = alice.dm.prepare_text(&alice.keys, &bob.pk(), "oops", None).await.unwrap();
        bob.receive(&peer_event(&m)).await;
        let r = bob.dm.prepare_reaction(&bob.keys, &m.message.id, "😮").await.unwrap();
        alice.receive(&reaction_to_peer(&r)).await;
        assert_eq!(shown(&alice, &m.message.id).await, vec![view("😮", 1, false)]);
        let d = alice.dm.prepare_delete(&alice.keys, &m.message.id).await.unwrap();
        assert!(d.message.reactions.is_empty());
        assert_eq!(reason(alice.dm.prepare_reaction(&alice.keys, &m.message.id, "👍").await.unwrap_err()), "message is deleted");
    }

    #[tokio::test]
    async fn emoji_usage_snapshot_merges_on_my_other_device() {
        let alice = signed(Keys::generate()).await;
        let phone = signed(alice.keys.clone()).await;
        let now = alice.clock.now().secs();
        for e in ["❤️", "👍", "❤️"] {
            alice.dm.emoji_used(e, now).await.unwrap();
        }
        assert_eq!(reason(alice.dm.emoji_used("abc", now).await.unwrap_err()), "reaction_invalid");
        phone.dm.emoji_used("🔥", now - 100).await.unwrap();

        let note = alice.dm.emoji_snapshot_if_due(now).await.unwrap().expect("uses were counted");
        assert_eq!(note, Envelope::own_emoji(&[("❤️".into(), 2, now), ("👍".into(), 1, now)]));
        assert!(alice.dm.emoji_snapshot_if_due(now).await.unwrap().is_none(), "nothing new");
        alice.dm.emoji_used("👍", now + 1).await.unwrap();
        assert!(alice.dm.emoji_snapshot_if_due(now + 599).await.unwrap().is_none(), "not twice in ten minutes");
        let later = alice.dm.emoji_snapshot_if_due(now + 600).await.unwrap().expect("due now");
        assert!(alice.dm.emoji_snapshot_now(now + 601).await.unwrap().is_none(), "taken");

        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &note, now)).await), vec!["emoji.updated"]);
        assert_eq!(phone.dm.emoji_top(6).await.unwrap(), vec!["❤️", "👍", "🔥"]);
        assert!(phone.receive(&own_note(&alice.keys, &note, now)).await.is_empty(), "the same map again");
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &later, now + 600)).await), vec!["emoji.updated"]);
        assert_eq!(phone.dm.emoji_top(2).await.unwrap(), vec!["👍", "❤️"], "a tie goes to the last used");
        // A map older than what the phone knows takes nothing away.
        assert!(phone.receive(&own_note(&alice.keys, &note, now + 700)).await.is_empty());
        assert_eq!(phone.dm.emoji_top(6).await.unwrap(), vec!["👍", "❤️", "🔥"]);

        // What is not an emoji, or not a pair of numbers, is skipped.
        let junk = Envelope::new(messenger_core::envelope::T_OWN_EMOJI)
            .with("usage", serde_json::json!({ "ok": [9, 1], "😂": "x", "😮": [1], "🎉": [50, now] }));
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &junk, now)).await), vec!["emoji.updated"]);
        assert_eq!(phone.dm.emoji_top(6).await.unwrap(), vec!["🎉", "👍", "❤️", "🔥"]);
        // The phone still owes the others its own 🔥: a map it heard does not
        // settle that.
        assert!(phone.dm.emoji_snapshot_if_due(now).await.unwrap().is_some(), "the phone's own 🔥");
        assert!(phone.dm.emoji_snapshot_if_due(now + 10_000).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_map_of_emoji_fits_its_size() {
        let alice = signed(Keys::generate()).await;
        // 64 long emoji weigh more than a map may: the least used fall off.
        let family = "👨\u{200d}👩\u{200d}👧";
        let base = 0x1f600u32;
        for i in 0..80u32 {
            let e = format!("{family}{}", char::from_u32(base + i).unwrap());
            alice.dm.emoji_used(&e, 1_000 + i as i64).await.unwrap();
        }
        let note = alice.dm.emoji_snapshot_now(2_000).await.unwrap().unwrap();
        assert!(note.encode().len() <= crate::own::EMOJI_SNAPSHOT_MAX_BYTES, "{}", note.encode().len());
        let kept = note.fields["usage"].as_object().unwrap().len();
        assert!(kept > 0 && kept < 64, "{kept}");
    }

    // ─── Stage C: presence keys ─────────────────────────────────────────────

    /// The note `from` tells its presence key with, as the runtime builds it.
    fn presence_key_note(from: &Party, key: &Keys, since: i64) -> Envelope {
        let pubkey = key.public_key().to_hex();
        Envelope::presence_key(Some((&pubkey, &crate::notes::presence_proof(key, &from.pk(), since))), since)
    }

    async fn key_of(p: &Party, peer: &Party) -> Option<(String, i64)> {
        messenger_store::presence::key_of(p.dm.store(), peer.pk().as_hex()).await.unwrap()
    }

    fn hex_of(k: &Keys) -> String {
        k.public_key().to_hex()
    }

    #[tokio::test]
    async fn a_presence_key_is_taken_with_its_proof_from_anyone_not_blocked() {
        let (alice, bob) = mutual().await;
        let now = alice.clock.now().secs();
        let k1 = Keys::generate();

        // Bob is not in Alice's book (yet): his key is kept, for when he is.
        let fx = alice.receive(&receipt(&bob, &alice, &presence_key_note(&bob, &k1, 5))).await;
        assert_eq!(names(&fx), vec![crate::notes::UI_EVENT_PRESENCE_KEYS_CHANGED]);
        let Effect::Emit(ui) = &fx[0] else { unreachable!() };
        assert_eq!(ui.payload, serde_json::json!({ "peer": bob.pk().as_hex() }));
        assert_eq!(key_of(&alice, &bob).await, Some((hex_of(&k1), 5)));
        assert!(!alice.dm.presence_allowed(&bob.pk()).await.unwrap(), "not watched nor shown");
        alice.contacts.add(&alice.pk(), bob.pk().as_hex(), None).await.unwrap();
        assert!(alice.dm.presence_allowed(&bob.pk()).await.unwrap());
        assert!(alice.receive(&receipt(&bob, &alice, &presence_key_note(&bob, &k1, 5))).await.is_empty(), "a replay");

        // An older epoch told late loses; a `since` past the note counts as a second past it.
        assert!(alice.receive(&receipt(&bob, &alice, &presence_key_note(&bob, &Keys::generate(), 4))).await.is_empty());
        let k2 = Keys::generate();
        assert_eq!(names(&alice.receive(&receipt(&bob, &alice, &presence_key_note(&bob, &k2, now + 3_600))).await).len(), 1);
        assert_eq!(key_of(&alice, &bob).await, Some((hex_of(&k2), now + 1)));

        // What is not a key, or not proven to be Bob's, is dropped.
        let k3 = Keys::generate();
        let t = messenger_core::envelope::T_PRESENCE_KEY;
        let proof = |owner: &PubKey, since| crate::notes::presence_proof(&k3, owner, since);
        let since = now + 2;
        let mut bad: Vec<Envelope> = [
            serde_json::json!("ff".repeat(32)),
            serde_json::json!(hex_of(&k3).to_uppercase()),
            serde_json::json!(&hex_of(&k3)[..63]),
            serde_json::json!(42),
        ]
        .into_iter()
        .map(|key| Envelope::new(t).with("pubkey", key).with("proof", proof(&bob.pk(), since)).with("since", since))
        .collect();
        bad.push(Envelope::new(t).with("pubkey", hex_of(&k3)).with("since", since));
        bad.push(Envelope::presence_key(Some((&hex_of(&k3), &proof(&alice.pk(), since))), since));
        bad.push(Envelope::presence_key(Some((&hex_of(&k3), &proof(&bob.pk(), since + 1))), since));
        bad.push(Envelope::presence_key(Some((&hex_of(&k3), &"00".repeat(64))), since));
        bad.push(Envelope::presence_key(Some((&hex_of(&k3), "cd")), since));
        bad.push(Envelope::new(t).with("pubkey", hex_of(&k3)).with("proof", proof(&bob.pk(), since)));
        bad.push(Envelope::new(t).with("pubkey", serde_json::Value::Null));
        for note in &bad {
            assert!(alice.receive(&receipt(&bob, &alice, note)).await.is_empty(), "{}", note.encode());
        }
        assert_eq!(key_of(&alice, &bob).await, Some((hex_of(&k2), now + 1)));

        // A contact I only asked: kept, not shown until we both chose to talk.
        let dave = gated().await;
        let ask = alice.dm.prepare_text(&alice.keys, &dave.pk(), "hi dave", None).await.unwrap();
        dave.receive(&peer_event(&ask)).await;
        alice.contacts.add(&alice.pk(), dave.pk().as_hex(), None).await.unwrap();
        assert_eq!(mode(&alice, &dave).await, "request_sent");
        let kd = Keys::generate();
        assert_eq!(names(&alice.receive(&receipt(&dave, &alice, &presence_key_note(&dave, &kd, 1))).await).len(), 1);
        assert_eq!(key_of(&alice, &dave).await, Some((hex_of(&kd), 1)));
        assert!(!alice.dm.presence_allowed(&dave.pk()).await.unwrap());

        // Somebody I blocked: dropped.
        let carol = gated().await;
        alice.dm.act(&alice.keys, &carol.pk(), Action::Block).await.unwrap();
        assert!(alice.receive(&receipt(&carol, &alice, &presence_key_note(&carol, &Keys::generate(), 1))).await.is_empty());
        assert_eq!(key_of(&alice, &carol).await, None);

        // Bob stopped sharing: the key goes, and a key of his told before it
        // and heard after it stays out.
        let fx = alice.receive(&receipt(&bob, &alice, &Envelope::presence_key(None, now + 1))).await;
        assert_eq!(names(&fx), vec![crate::notes::UI_EVENT_PRESENCE_KEYS_CHANGED]);
        assert_eq!(key_of(&alice, &bob).await, None);
        assert!(alice.receive(&receipt(&bob, &alice, &presence_key_note(&bob, &k1, 5))).await.is_empty());
        assert!(alice.receive(&receipt(&bob, &alice, &presence_key_note(&bob, &k2, now + 1))).await.is_empty());
        assert_eq!(key_of(&alice, &bob).await, None);
    }

    #[tokio::test]
    async fn a_presence_key_of_another_contact_is_not_taken() {
        let (alice, bob) = mutual().await;
        let carol = gated_with(Keys::generate()).await;
        // Carol becomes a contact Alice talks with too.
        let p = carol.dm.prepare_text(&carol.keys, &alice.pk(), "hi", None).await.unwrap();
        alice.receive(&peer_event(&p)).await;
        for w in for_peer(&p.followups) {
            alice.receive(&w).await;
        }
        let acc = alice.dm.act(&alice.keys, &carol.pk(), Action::Accept).await.unwrap();
        for w in for_peer(&acc.outbounds) {
            for back in for_peer(&sends(&carol.receive(&w).await)) {
                alice.receive(&back).await;
            }
        }
        assert_eq!(mode(&alice, &carol).await, "full_chat");
        for peer in [&bob, &carol] {
            alice.contacts.add(&alice.pk(), peer.pk().as_hex(), None).await.unwrap();
        }
        // Bob told Carol his key; she tells it to Alice first, with Bob's proof.
        let k = Keys::generate();
        let bobs_note = presence_key_note(&bob, &k, 1);
        assert!(alice.receive(&receipt(&carol, &alice, &bobs_note)).await.is_empty(), "the proof names Bob");
        assert_eq!(key_of(&alice, &carol).await, None);
        assert_eq!(names(&alice.receive(&receipt(&bob, &alice, &bobs_note)).await).len(), 1);
        assert_eq!(key_of(&alice, &bob).await, Some((hex_of(&k), 1)));
        // Even with the secret of the key, a key Bob holds is not Carol's.
        assert!(alice.receive(&receipt(&carol, &alice, &presence_key_note(&carol, &k, 2))).await.is_empty());
        assert_eq!(key_of(&alice, &carol).await, None);
        assert_eq!(key_of(&alice, &bob).await, Some((hex_of(&k), 1)));
    }

    #[test]
    fn the_proof_of_a_presence_key_is_fixed() {
        // SHA-256 of "veydan-presence-key-v1", 32 bytes 01, and since 1759700000 as i64 LE.
        let owner = PubKey::parse(&"01".repeat(32)).unwrap();
        let digest = crate::notes::presence_proof_digest(&owner, 1_759_700_000);
        assert_eq!(hex::encode(digest), "eb949e1e5c60c048a2e78d382252be61a4a6aa698c125a2cada28824b101ed7b");
        let k = Keys::generate();
        let proof = crate::notes::presence_proof(&k, &owner, 1_759_700_000);
        assert_eq!(proof.len(), 128);
        assert!(crate::notes::presence_proof_ok(&hex_of(&k), &owner, 1_759_700_000, &proof));
        assert!(!crate::notes::presence_proof_ok(&hex_of(&k), &owner, 1_759_700_001, &proof));
        assert!(!crate::notes::presence_proof_ok(&hex_of(&Keys::generate()), &owner, 1_759_700_000, &proof));
    }

    #[tokio::test]
    async fn own_presence_carries_the_epoch_its_since_and_the_switch() {
        use crate::own::PresenceState;
        let alice = Party::new().await;
        let phone = Party::with_keys(alice.keys.clone()).await;
        let now = phone.clock.now().secs();
        let state = |epoch, since, sharing| PresenceState { epoch, since, sharing };
        assert_eq!(phone.dm.presence_state().await.unwrap(), state(0, 0, true));
        // The phone told Bob the key of epoch 0.
        let bob = "bb".repeat(32);
        messenger_store::presence::mark_told(phone.dm.store(), &bob, 0).await.unwrap();

        let fx = phone.receive(&own_note(&alice.keys, &Envelope::own_presence(2, 1_000, true), now)).await;
        assert_eq!(names(&fx), vec![crate::own::UI_EVENT_PRESENCE_EPOCH_CHANGED]);
        assert_eq!(phone.dm.presence_state().await.unwrap(), state(2, 1_000, true), "the since of the device that rotated");
        assert_eq!(
            messenger_store::presence::told(phone.dm.store(), &bob).await.unwrap(),
            Some(2),
            "the device that rotated tells the new key"
        );
        assert_eq!(phone.dm.presence_devices_owed().await.unwrap(), None, "my devices know: one of them told me");

        // Older epochs, the same note again, and what is not a note change nothing.
        for note in [Envelope::own_presence(1, 2_000, false), Envelope::own_presence(2, 1_000, true)] {
            assert!(phone.receive(&own_note(&alice.keys, &note, now + 5)).await.is_empty());
        }
        let t = messenger_core::envelope::T_OWN_PRESENCE;
        for junk in [
            Envelope::new(t).with("epoch", u64::from(u32::MAX) + 1).with("since", 5),
            Envelope::new(t).with("epoch", -3).with("since", 5),
            Envelope::new(t).with("epoch", 9),
            Envelope::new(t).with("epoch", 9).with("since", -1),
        ] {
            assert!(phone.receive(&own_note(&alice.keys, &junk, now)).await.is_empty(), "{}", junk.encode());
        }
        assert_eq!(phone.dm.presence_state().await.unwrap(), state(2, 1_000, true));

        // Two devices that moved to one epoch at once end alike: the later
        // since, and off if either is.
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &Envelope::own_presence(2, 900, false), now)).await).len(), 1);
        assert_eq!(phone.dm.presence_state().await.unwrap(), state(2, 1_000, false));
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &Envelope::own_presence(2, 1_100, true), now)).await).len(), 1);
        assert_eq!(phone.dm.presence_state().await.unwrap(), state(2, 1_100, false));
        // A newer epoch turns it on again.
        phone.receive(&own_note(&alice.keys, &Envelope::own_presence(3, now + 50, true), now)).await;
        assert_eq!(phone.dm.presence_state().await.unwrap(), state(3, now + 50, true));

        // This device moving: up by one, after the since before it, owed to my other devices once.
        let moved = phone.dm.rotate_presence(false).await.unwrap();
        assert_eq!(moved, state(4, now + 51, false));
        assert_eq!(phone.dm.presence_state().await.unwrap(), moved);
        assert_eq!(phone.dm.presence_devices_owed().await.unwrap(), Some(moved));
        phone.dm.presence_devices_told(4).await.unwrap();
        assert_eq!(phone.dm.presence_devices_owed().await.unwrap(), None);
        assert_eq!(phone.dm.rotate_presence(true).await.unwrap().since, now + 52);
    }

    // ─── Contact cards ──────────────────────────────────────────────────────

    fn card_of(pubkey: &str, name: &str, phone: Option<&str>) -> ContactCard {
        let mut raw = serde_json::json!({ "pubkey": pubkey, "display_name": name, "about": "**hi** there", "at": 1_000_000 });
        if let Some(p) = phone {
            raw["phone"] = p.into();
        }
        messenger_contacts::card::validate(&raw).unwrap()
    }

    fn notice_body(effects: &[Effect]) -> Option<messenger_core::Body> {
        effects.iter().find_map(|e| match e {
            Effect::Notify(n) => n.body.clone(),
            _ => None,
        })
    }

    #[tokio::test]
    async fn my_own_card_reaches_the_peer_with_its_phone() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        alice.dm.set_signer(Some(alice.keys.clone()));
        let card = card_of(alice.pk().as_hex(), "Alice  A.", Some("+7 999 123-45-67"));
        let p = alice.dm.prepare_card(&alice.keys, &bob.pk(), &card).await.unwrap();
        assert_eq!(p.message.content_type, "contact");
        assert!(p.message.text.is_none() && p.message.media.is_none());
        let mine = p.message.card.as_ref().expect("a card");
        assert!(mine.is_me && !mine.is_contact);
        assert_eq!(alice.dm.open_chat(&bob.pk()).await.unwrap().last_preview.as_deref(), Some("👤 Alice  A."));

        let fx = bob.receive(&peer_event(&p)).await;
        assert_eq!(names(&fx), vec!["dm.message", "notify"]);
        assert_eq!(
            notice_body(&fx),
            Some(messenger_core::Body::Link { link: messenger_core::LinkKind::Contact, title: "Alice A.".into() })
        );
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        assert_eq!(chat.last_preview.as_deref(), Some("👤 Alice  A."));
        let m = &bob.dm.messages(&chat.id, None, 50).await.unwrap()[0];
        assert_eq!((m.content_type.as_str(), m.direction.as_str()), ("contact", "in"));
        let seen = m.card.as_ref().expect("a card");
        assert_eq!(seen.pubkey, alice.pk().as_hex());
        assert_eq!(seen.phone.as_deref(), Some("+79991234567"));
        assert_eq!(seen.label, "Alice  A.");
        assert!(!seen.is_me && !seen.is_contact && !seen.blocked);
        assert!(!seen.bio.is_empty());
        let kept = bob.dm.stored_card(&m.id).await.unwrap();
        assert_eq!((kept.incoming, kept.sender.as_str(), kept.card.phone.as_deref()), (true, alice.pk().as_hex(), Some("+79991234567")));

        // Known as a contact, the card says so; removed, it is gone.
        bob.contacts.add(&bob.pk(), alice.pk().as_hex(), None).await.unwrap();
        assert!(bob.dm.message(&m.id).await.unwrap().unwrap().card.unwrap().is_contact);
        bob.dm.delete_local(&m.id).await.unwrap();
        assert!(bob.dm.message(&m.id).await.unwrap().unwrap().card.is_none());
        assert!(bob.dm.stored_card(&m.id).await.is_err());
    }

    #[tokio::test]
    async fn a_card_has_no_text_to_edit() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let card = card_of(alice.pk().as_hex(), "Alice", None);
        let p = alice.dm.prepare_card(&alice.keys, &bob.pk(), &card).await.unwrap();
        assert!(alice.dm.prepare_edit(&alice.keys, &p.message.id, "text").await.is_err());

        // An edit from another client is not applied, live or replayed.
        bob.receive(&peer_event(&p)).await;
        let edit = wrap(&alice.keys, &bob.pk(), &Envelope::edit(&p.message.id, "text").encode(), 1_000_100, None).unwrap();
        assert!(bob.receive(&edit.to_peer).await.is_empty());
        let m = bob.dm.message(&p.message.id).await.unwrap().unwrap();
        assert!(m.text.is_none() && m.edited_at.is_none() && m.card.is_some());

        let p2 = alice.dm.prepare_card(&alice.keys, &bob.pk(), &card).await.unwrap();
        let early = wrap(&alice.keys, &bob.pk(), &Envelope::edit(&p2.message.id, "text").encode(), 1_000_200, None).unwrap();
        assert!(bob.receive(&early.to_peer).await.is_empty(), "target unknown yet");
        bob.receive(&peer_event(&p2)).await;
        let m = bob.dm.message(&p2.message.id).await.unwrap().unwrap();
        assert!(m.text.is_none() && m.edited_at.is_none());
    }

    #[tokio::test]
    async fn a_card_of_somebody_else_never_keeps_a_phone() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        let carol = Keys::generate().public_key().to_hex();
        // A client that put Carol's phone in her card anyway.
        let card = card_of(&carol, "Carol", Some("+15550001111"));
        let p = alice.dm.prepare_card(&alice.keys, &bob.pk(), &card).await.unwrap();
        bob.receive(&peer_event(&p)).await;
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        let m = &bob.dm.messages(&chat.id, None, 50).await.unwrap()[0];
        assert_eq!(m.card.as_ref().unwrap().pubkey, carol);
        assert_eq!(m.card.as_ref().unwrap().phone, None);
        let row = repo::get(&bob.dm.store, &m.id).await.unwrap().unwrap();
        assert!(!row.envelope_json.contains("+1555") && !row.media_json.unwrap().contains("+1555"), "kept nowhere");
    }

    #[tokio::test]
    async fn a_card_that_is_no_card_is_dropped() {
        let alice = Party::new().await;
        let bob = Party::new().await;
        for (i, junk) in [
            Envelope::contact(serde_json::json!({ "pubkey": "zz", "name": "x" })),
            Envelope::contact(serde_json::json!("just a string")),
            Envelope::new(T_CONTACT),
            Envelope::contact(serde_json::json!({ "pubkey": "ab".repeat(32), "about": "x".repeat(30_000) })),
        ]
        .iter()
        .enumerate()
        {
            let w = wrap(&alice.keys, &bob.pk(), &junk.encode(), 1_000_001 + i as i64, None).unwrap();
            assert!(bob.receive(&w.to_peer).await.is_empty(), "{}", junk.encode());
        }
        let chat = bob.dm.open_chat(&alice.pk()).await.unwrap();
        assert!(bob.dm.messages(&chat.id, None, 50).await.unwrap().is_empty());
        assert!(chat.last_preview.is_none());
    }

    // ─── Phones between my devices ──────────────────────────────────────────

    #[tokio::test]
    async fn my_phone_goes_to_my_other_devices_and_the_later_wins() {
        let alice = Party::new().await;
        let phone = Party::with_keys(alice.keys.clone()).await;
        assert_eq!(alice.dm.own_profile_due(1_000_000).await.unwrap(), None, "nothing set, nothing to tell");
        assert!(matches!(alice.dm.set_own_private(Some("12"), true).await, Err(MessengerError::Invalid(c)) if c == "phone_invalid"));

        let (kept, note) = alice.dm.set_own_private(Some(" +7 (999) 123-45-67 "), true).await.unwrap();
        assert_eq!((kept.phone.as_deref(), kept.share_phone, kept.updated_at), (Some("+79991234567"), true, 1_000_000));
        assert_eq!(note, Envelope::own_profile(Some("+79991234567"), true, 1_000_000));
        assert_eq!(names(&phone.receive(&own_note(&alice.keys, &note, 1_000_000)).await), vec!["own_private.updated"]);
        assert_eq!(phone.dm.own_private().await.unwrap(), kept);
        assert!(phone.receive(&own_note(&alice.keys, &note, 1_000_001)).await.is_empty(), "the same again is no news");

        // An older note, or one that is no number, changes nothing.
        let older = Envelope::own_profile(None, false, 999_999);
        assert!(phone.receive(&own_note(&alice.keys, &older, 1_000_002)).await.is_empty());
        let junk = Envelope::own_profile(None, false, 1_000_500).with("phone", "call me");
        assert!(phone.receive(&own_note(&alice.keys, &junk, 1_000_003)).await.is_empty());
        let no_time = Envelope::own_profile(None, false, 0);
        assert!(phone.receive(&own_note(&alice.keys, &no_time, 1_000_004)).await.is_empty());
        assert_eq!(phone.dm.own_private().await.unwrap(), kept);

        // Removed on the phone, with a clock behind: still the later.
        phone.clock.0.store(900_000, Ordering::SeqCst);
        let (gone, note) = phone.dm.set_own_private(Some("  "), false).await.unwrap();
        assert_eq!((gone.phone, gone.updated_at), (None, 1_000_001));
        alice.receive(&own_note(&alice.keys, &note, 1_000_005)).await;
        assert_eq!(alice.dm.own_private().await.unwrap().phone, None);

        // Told again once a week.
        let due = alice.dm.own_profile_due(1_000_010).await.unwrap().expect("never sent from here");
        assert_eq!(due, Envelope::own_profile(None, false, 1_000_001));
        alice.dm.own_profile_sent(1_000_010).await.unwrap();
        assert_eq!(alice.dm.own_profile_due(1_000_010 + crate::own::OWN_PROFILE_EVERY_SECS - 1).await.unwrap(), None);
        assert!(alice.dm.own_profile_due(1_000_010 + crate::own::OWN_PROFILE_EVERY_SECS).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn the_phones_of_my_contacts_go_to_my_other_devices() {
        let alice = Party::new().await;
        let phone = Party::with_keys(alice.keys.clone()).await;
        let (bob, carol) = (Keys::generate().public_key().to_hex(), Keys::generate().public_key().to_hex());
        let note = alice.dm.keep_contact_phone(&bob, Some("+1 555 000 1111"), 999_000).await.unwrap().expect("news");
        assert_eq!(note, Envelope::own_card(&bob, Some("+15550001111"), 999_000), "the time of the card");
        assert_eq!(alice.dm.keep_contact_phone(&bob, Some("+15550001111"), 999_500).await.unwrap(), None, "kept already");
        assert!(alice.dm.keep_contact_phone(&bob, Some("nope"), 999_500).await.is_err());
        // An older card does not take the place of a newer one.
        assert_eq!(alice.dm.keep_contact_phone(&bob, Some("+1 555 000 9999"), 998_000).await.unwrap(), None);
        assert_eq!(alice.dm.contact_private(&bob).await.unwrap().unwrap().phone.as_deref(), Some("+15550001111"));
        // A card from the future counts as made now.
        let later = alice.dm.keep_contact_phone(&carol, Some("+1 555 000 3333"), i64::MAX).await.unwrap().unwrap();
        assert_eq!(later, Envelope::own_card(&carol, Some("+15550003333"), 1_000_000));

        let fx = phone.receive(&own_note(&alice.keys, &note, 1_000_000)).await;
        assert_eq!(names(&fx), vec!["contact_private.updated"]);
        let Effect::Emit(ev) = &fx[0] else { panic!() };
        assert_eq!(ev.payload["pubkey"], bob.as_str());
        assert_eq!(phone.dm.contact_private(&bob).await.unwrap().unwrap().phone.as_deref(), Some("+15550001111"));
        assert_eq!(phone.dm.contact_private(&carol).await.unwrap(), None, "per contact");

        // Taken back later; an older note does not bring it back.
        alice.clock.0.store(1_000_100, Ordering::SeqCst);
        let gone = alice.dm.keep_contact_phone(&bob, None, 1_000_100).await.unwrap().unwrap();
        phone.receive(&own_note(&alice.keys, &gone, 1_000_100)).await;
        assert!(phone.receive(&own_note(&alice.keys, &note, 1_000_101)).await.is_empty());
        assert_eq!(phone.dm.contact_private(&bob).await.unwrap().unwrap().phone, None);
        // A note about no key is dropped.
        let junk = Envelope::own_card("not-a-key", Some("+15550001111"), 1_000_200);
        assert!(phone.receive(&own_note(&alice.keys, &junk, 1_000_102)).await.is_empty());
    }
}

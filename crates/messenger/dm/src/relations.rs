// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Side effects around the relationship matrix: loading and saving the
//! state, building control messages, system lines, and the hooks the
//! message flow calls. Decisions themselves are in `relationship`.

use crate::relationship::{
    apply_my_action, apply_own_signal, apply_peer_signal, decided_now, floor_after_pending_end, inbound_decision,
    merge_decision, outbound_permission, raise_floor, screen_mode, signal_implied_by_message, Action, DmFrom, DropReason,
    InboundDecision, InboundKind, MyContact, OutboundPermission, OwnSignal, PeerSignal, Relationship, ScreenMode, Signal,
    SystemLine, FLOOR_SKEW_SECS,
};
use crate::peer::DmPeer;
use crate::snapshot::Known;
use crate::service::{DmService, UI_EVENT_DM_MESSAGE};
use crate::wrap::{wrap_as, Wake};
use messenger_core::traits::UiEvent;
use messenger_contacts::UI_EVENT_CONTACTS_UPDATED;
use messenger_core::{Effect, Envelope, MessengerError, Outbound, PubKey, Result, TextFormat};
use messenger_store::dm_relations::{self, RelationRow};
use messenger_store::{banned, dm_held};
use messenger_store::{chats, settings};
use messenger_store::messages::{self as repo, NewMessage};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

pub const UI_EVENT_DM_RELATIONSHIP: &str = "dm.relationship";

/// Set once `fill_book_once` has run on this device.
const KEY_BOOK_FILLED: &str = "contacts.filled_from_relations";

/// `contacts.updated` for one peer, or for several (`None`).
pub fn contacts_updated(peer: Option<&PubKey>) -> UiEvent {
    UiEvent {
        name: UI_EVENT_CONTACTS_UPDATED.into(),
        payload: match peer {
            Some(p) => serde_json::json!({ "pubkey": p.as_hex() }),
            None => serde_json::json!({}),
        },
    }
}

/// Relationship facts for the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationView {
    pub peer_pubkey: String,
    /// The private server of the chat this relation is of; absent for the
    /// home chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    pub mode: String,
    pub my_contact: String,
    pub blocked: bool,
    pub peer_signal: String,
    pub was_ever_mutual: bool,
    pub can_send: bool,
    /// The network bans the peer (`DmService::is_banned`): the mode is
    /// `blocked` whatever `blocked` says, and the screen may say that the
    /// person was removed from the network rather than blocked by me.
    #[serde(default)]
    pub banned: bool,
}

/// What one of my actions produced.
#[derive(Clone, Debug)]
pub struct ActionResult {
    pub relation: RelationView,
    /// Control messages to publish, in order (each with its self-copy).
    pub outbounds: Vec<Outbound>,
    pub events: Vec<UiEvent>,
}

/// Result of the outbound gate for a regular message.
pub(crate) struct OutboundGate {
    pub request: bool,
}

/// Result of the inbound gate for a regular message of the peer.
pub(crate) enum InboundGate {
    /// Store and show; the effects (an implied signal) go first.
    /// `regate`: the floor went up under this message, so a held row above
    /// it may follow once it is stored.
    Pass { effects: Vec<Effect>, regate: bool },
    /// A second message before approval: stored hidden and held, as the
    /// signal that ends the episode before it may still come (wraps come in
    /// any order). A raise of the floor may show it later.
    Hold,
    Drop,
}

/// At most this many held rows per chat; past it a second message is
/// dropped as before. Only the peer's own requests suffer from a full one.
pub const HELD_PER_CHAT: i64 = 16;

impl DmService {
    /// What stands between me and `peer` in one chat (a `&PubKey` is the
    /// home chat), as the store has it. The relation of a private chat is
    /// its own: a block at home does not shut it, nor its block the home
    /// chat.
    pub async fn load_relation(&self, peer: impl Into<DmPeer>) -> Result<Relationship> {
        let peer = peer.into();
        Ok(match dm_relations::get(&self.store, peer.key.as_hex(), peer.server()).await? {
            Some(r) => Relationship {
                my_contact: MyContact::parse(&r.my_contact).unwrap_or(MyContact::None),
                blocked: r.blocked,
                peer_signal: PeerSignal::parse(&r.peer_signal).unwrap_or(PeerSignal::None),
                was_ever_mutual: r.was_ever_mutual,
                last_signal_at: r.last_signal_at,
                last_my_signal_at: r.last_my_signal_at,
                request_floor: r.request_floor,
                decided_at: r.decided_at,
            },
            None => Relationship::default(),
        })
    }

    /// What the gate makes of a stored row of the peer, as it made of the
    /// message when it came (`apply_inbound`): a text by its length and its
    /// links, anything else as other.
    pub fn kind_of(content_type: &str, text: Option<&str>, format: TextFormat) -> InboundKind {
        match (content_type, text) {
            (repo::CT_TEXT, Some(t)) => Self::kind_of_text(t, format),
            _ => InboundKind::Other,
        }
    }

    /// A Markdown text holds a link when its marks make one, or its words
    /// spell one once they are read (`https:&#47;&#47;…`): a stranger's
    /// request has nothing to click either way.
    pub fn kind_of_text(text: &str, format: TextFormat) -> InboundKind {
        let has_link = if format.is_markdown() {
            messenger_richtext::md::holds_link(text, crate::body::has_link)
        } else {
            crate::body::has_link(text)
        };
        InboundKind::PlainText { chars: text.chars().count(), has_link }
    }

    pub(crate) async fn save_relation(&self, peer: impl Into<DmPeer>, r: &Relationship) -> Result<()> {
        let peer = &peer.into();
        dm_relations::put(
            &self.store,
            &RelationRow {
                peer_pubkey: peer.key.as_hex().to_string(),
                server_id: dm_relations::server_key(peer.server()).to_string(),
                my_contact: r.my_contact.as_str().into(),
                blocked: r.blocked,
                peer_signal: r.peer_signal.as_str().into(),
                was_ever_mutual: r.was_ever_mutual,
                last_signal_at: r.last_signal_at,
                last_my_signal_at: r.last_my_signal_at,
                request_floor: r.request_floor,
                decided_at: r.decided_at,
                created_at: 0,
                updated_at: 0,
            },
        )
        .await
    }

    /// Whether a request to me came or went between two states of the
    /// relationship: the queue of requests is to be counted again.
    pub(crate) fn request_changed(before: &Relationship, after: &Relationship) -> bool {
        (screen_mode(before) == ScreenMode::RequestReceived) != (screen_mode(after) == ScreenMode::RequestReceived)
    }

    /// Whether the network refuses `peer`: the manifest in use names the
    /// key in `banned_keys` (a confirmed report of harm to a child, decided
    /// by a person), kept by the runtime in `msg_banned_keys`. Such a key is
    /// blocked for as long as the manifest names it, whatever the relation
    /// stored for it says, and that relation is never changed by the ban:
    /// a lifted ban leaves it as the user had it. Every gate asks this one
    /// question; a lookup by the primary key, as cheap as `load_relation`,
    /// and read by every process that opens the database.
    pub async fn is_banned(&self, peer: &PubKey) -> Result<bool> {
        banned::contains(&self.store, peer.as_hex()).await
    }

    /// Screen mode and whether the composer may be used. A banned key reads
    /// as blocked, with nothing to send, before any setting is looked at.
    pub(crate) async fn mode_of(&self, chat_id: &str, peer: &DmPeer) -> Result<(ScreenMode, bool)> {
        if self.is_banned(&peer.key).await? {
            return Ok((ScreenMode::Blocked, false));
        }
        if !self.gate_enabled() {
            return Ok((ScreenMode::FullChat, true));
        }
        let r = self.load_relation(peer).await?;
        // The pinned bot is approved beforehand (`with_bot`): a full chat
        // unless I blocked it.
        if self.is_bot_chat(peer) && !r.blocked {
            return Ok((ScreenMode::FullChat, true));
        }
        let sent = repo::count_visible_outgoing(&self.store, chat_id).await?;
        let can = !matches!(outbound_permission(&r, sent, true), OutboundPermission::Deny(_));
        Ok((screen_mode(&r), can))
    }

    pub async fn relation(&self, peer: impl Into<DmPeer>) -> Result<RelationView> {
        let peer = peer.into();
        let chat_id = peer.chat_id();
        let r = self.load_relation(&peer).await?;
        let (mode, can_send) = self.mode_of(&chat_id, &peer).await?;
        Ok(RelationView {
            peer_pubkey: peer.key.as_hex().to_string(),
            server_id: peer.server.clone(),
            mode: mode.as_str().into(),
            my_contact: r.my_contact.as_str().into(),
            blocked: r.blocked,
            peer_signal: r.peer_signal.as_str().into(),
            was_ever_mutual: r.was_ever_mutual,
            can_send,
            banned: self.is_banned(&peer.key).await?,
        })
    }

    pub async fn is_blocked(&self, peer: impl Into<DmPeer>) -> Result<bool> {
        Ok(self.load_relation(peer).await?.blocked)
    }

    /// Whether `peer` is the pinned bot in its home chat. The bot lives on
    /// the home set: its key in a private chat is anybody's, a stranger.
    pub(crate) fn is_bot_chat(&self, peer: &DmPeer) -> bool {
        !peer.is_private() && self.is_bot(&peer.key)
    }

    /// My block of `peer` changed hands (here, or on another device of
    /// mine): the group chats they wrote in count and show without them,
    /// or with them again (`chats::group_chats_follow_block`); the
    /// screen hears of each chat that may have changed.
    async fn group_chats_follow_block(&self, peer: &PubKey) -> Result<Vec<UiEvent>> {
        Ok(chats::group_chats_follow_block(&self.store, peer.as_hex())
            .await?
            .into_iter()
            .map(|chat_id| UiEvent { name: crate::service::UI_EVENT_DM_UPDATED.into(), payload: serde_json::json!({ "chat_id": chat_id }) })
            .collect())
    }

    pub async fn blocked_peers(&self) -> Result<Vec<String>> {
        dm_relations::blocked_peers(&self.store).await
    }

    /// The address book is kept on each device apart, and presence and the
    /// profile follow it. What another device of mine decided about `peer`
    /// is put into this one's book, as `dm_act` does for a decision taken
    /// here. Returns whether the book changed. The book is of the home
    /// set: what is decided in a private chat leaves it as it is.
    async fn mirror_book(&self, peer: &DmPeer, before: &Relationship, after: &Relationship) -> Result<bool> {
        if peer.is_private() {
            return Ok(false);
        }
        let Some(me) = self.my_key().await?.as_deref().and_then(PubKey::parse) else { return Ok(false) };
        if me == peer.key {
            return Ok(false);
        }
        let approved = after.my_contact == MyContact::Approved && !after.blocked;
        if approved && before.my_contact != MyContact::Approved && !self.contacts.is_contact(&peer.key).await? {
            self.contacts.add(&me, peer.key.as_hex(), None).await?;
            return Ok(true);
        }
        let removed = before.my_contact == MyContact::Approved && after.my_contact == MyContact::None;
        if removed && self.contacts.is_contact(&peer.key).await? {
            self.contacts.remove(&peer.key).await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Once per device: the peers I approved on another device before this
    /// one mirrored such decisions (`mirror_book`) join the book. A peer
    /// the book has a row of, live or removed, is left as it is: a removal
    /// was meant, and the approvals that migration 007 inferred from old
    /// chats know nothing of it. Only an approval I sent after the removal
    /// (on another device) brings a removed one back. Returns who was added.
    pub async fn fill_book_once(&self) -> Result<Vec<PubKey>> {
        if settings::get_bool(&self.store, KEY_BOOK_FILLED, false).await? {
            return Ok(vec![]);
        }
        let Some(me) = self.my_key().await?.as_deref().and_then(PubKey::parse) else { return Ok(vec![]) };
        let mut added = Vec::new();
        for hex in dm_relations::approved_peers(&self.store).await? {
            let Some(pk) = PubKey::parse(&hex) else { continue };
            if pk == me {
                continue;
            }
            if let Some(row) = messenger_store::contacts::get(&self.store, pk.as_hex()).await? {
                let Some(removed_at) = row.deleted_at else { continue };
                if self.load_relation(&pk).await?.last_my_signal_at <= removed_at {
                    continue;
                }
            }
            self.contacts.add(&me, pk.as_hex(), None).await?;
            added.push(pk);
        }
        settings::set_bool(&self.store, KEY_BOOK_FILLED, true).await?;
        Ok(added)
    }

    pub async fn relationship_event(&self, chat_id: &str, peer: &DmPeer) -> Result<UiEvent> {
        let (mode, can_send) = self.mode_of(chat_id, peer).await?;
        Ok(UiEvent {
            name: UI_EVENT_DM_RELATIONSHIP.into(),
            payload: serde_json::json!({
                "chat_id": chat_id, "peer": peer.key.as_hex(), "mode": mode.as_str(), "can_send": can_send,
                "banned": self.is_banned(&peer.key).await?,
            }),
        })
    }

    /// Insert a system line (idempotent) and return the event announcing it.
    /// A line is something to show, so the chat is there for it; one dated
    /// before the chat was deleted here belongs to what was deleted. The
    /// line of a message being stored (`of_message`) is shown with it, as
    /// the message passed the tombstone already.
    async fn system_line(
        &self,
        chat_id: &str,
        peer: &DmPeer,
        line: SystemLine,
        at: i64,
        of_message: bool,
    ) -> Result<Option<UiEvent>> {
        if !of_message && at <= chats::cleared_at(&self.store, chat_id).await? {
            return Ok(None);
        }
        let id = system_line_id(line, at, peer);
        let inserted = repo::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.into(),
                wire_id: None,
                direction: repo::DIR_OUT.into(),
                status: repo::STATUS_SENT.into(),
                content_type: repo::CT_SYSTEM.into(),
                text: Some(line.as_str().into()),
                envelope_json: "{}".into(),
                sender_pubkey: String::new(),
                reply_to_id: None,
                target_id: None,
                created_at: at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: None,
            },
        )
        .await?;
        if !inserted {
            return Ok(None);
        }
        chats::ensure_dm(&self.store, peer.key.as_hex(), peer.server()).await?;
        let Some(view) = self.message(&id).await? else { return Ok(None) };
        Ok(Some(UiEvent {
            name: UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": view, "historical": true }),
        }))
    }

    /// Build one control message (wrap for the peer + self-copy) and record
    /// it as a hidden row, so a copy coming back is recognised as ours. Its
    /// time is the time of the decision it tells my other devices of: later
    /// than the last decision this device knows (one a copy brought from a
    /// device whose clock runs ahead included), or the copy of this one
    /// would lose to it there (`merge_decision`) and the devices would part.
    /// `invite` rides on an accept: the secret of an invitation of the
    /// peer, by which I come in without a request.
    async fn control(&self, keys: &Keys, chat_id: &str, peer: &DmPeer, signal: Signal, invite: Option<&str>) -> Result<(i64, Vec<Outbound>)> {
        let content = match invite.filter(|_| signal == Signal::Accept) {
            Some(secret) => Envelope::control_with_invite(signal.as_str(), secret),
            None => Envelope::control(signal.as_str()),
        }
        .encode();
        let decided = self.load_relation(peer).await?.decided_at;
        let at = self.next_created_at(chat_id).await?.max(decided.saturating_add(1));
        // A signal between the two apps: nobody is woken for it.
        let w = wrap_as(keys, peer, &content, at, None, Wake::Nobody)?;
        repo::insert(
            &self.store,
            &NewMessage {
                id: w.rumor_id.as_hex().to_string(),
                chat_id: chat_id.into(),
                wire_id: Some(w.to_peer.id.as_hex().to_string()),
                direction: repo::DIR_OUT.into(),
                status: repo::STATUS_SENT.into(),
                content_type: repo::CT_CONTROL.into(),
                text: None,
                envelope_json: content,
                sender_pubkey: keys.public_key().to_hex(),
                reply_to_id: None,
                target_id: None,
                created_at: at,
                is_hidden: true,
                outbox_local_id: None,
                media_json: None,
            },
        )
        .await?;
        let mut out = vec![self.route_to_peer(peer, w.to_peer).await?];
        if let Some(event) = w.to_self {
            out.push(peer.to_self(event));
        }
        Ok((at, out))
    }

    /// One of my actions: state, signals, system line, events.
    pub async fn act(&self, keys: &Keys, peer: impl Into<DmPeer>, action: Action) -> Result<ActionResult> {
        self.act_with(keys, &peer.into(), action, false, None).await
    }

    /// A request by an invitation of the peer without a text (a link
    /// pasted, a QR scanned): the accept goes at once, the secret on it,
    /// and the peer's device lets me in by it. With a text, the request
    /// goes with the text (`prepare_request_text`).
    pub async fn request_by_invite(&self, keys: &Keys, peer: impl Into<DmPeer>, invite: &str) -> Result<ActionResult> {
        check_invite_secret(invite)?;
        self.act_with(keys, &peer.into(), Action::Request, true, Some(invite)).await
    }

    /// A request the peer is told of at once, without a text and without a
    /// secret: my accept goes now, as with a first text. What opening the
    /// private chat of a server card sends (the runtime's
    /// `contact_open_private`): a device of the peer that approved me in
    /// that chat answers it (confirm-back) and the chat is open on both
    /// sides; one that did not drops it unheard, and my first text there
    /// is a request, as any.
    pub async fn request_told(&self, keys: &Keys, peer: impl Into<DmPeer>) -> Result<ActionResult> {
        self.act_with(keys, &peer.into(), Action::Request, true, None).await
    }

    /// An episode ended (I removed or unblocked a stranger, or the peer sent
    /// an ending signal): what the peer wrote so far belonged to it, and
    /// their next message is a new request. The floor is the newest of
    /// their stored messages, not my clock: it is compared with their rumor
    /// times. `upto` (a peer signal's time) keeps it from passing the
    /// signal; my clock plus a skew caps it, so a row dated in the future
    /// stays counted. My own no does not end an episode: the peer I
    /// declined gets no further message through.
    async fn end_episode(&self, chat_id: &str, r: &mut Relationship, upto: Option<i64>) -> Result<()> {
        if let Some(last) = repo::last_visible_incoming_at(&self.store, chat_id).await? {
            let candidate = upto.map_or(last, |at| last.min(at));
            r.request_floor = raise_floor(r.request_floor, candidate, self.clock.now().secs());
        }
        Ok(())
    }

    /// `with_message`: the action goes with my first text, or by an
    /// invitation, so the peer is told at once (`apply_my_action`);
    /// `invite`: the secret that rides on the accept.
    pub(crate) async fn act_with(
        &self,
        keys: &Keys,
        peer: &DmPeer,
        action: Action,
        with_message: bool,
        invite: Option<&str>,
    ) -> Result<ActionResult> {
        if peer.key.as_hex() == keys.public_key().to_hex() {
            return Err(MessengerError::Invalid("that is your own key".into()));
        }
        // A banned key is let in by nothing of mine either: no request, no
        // accept (by an invitation of theirs neither), no unblock. My block,
        // my removal and my no stay mine to give: they hold after the ban
        // is lifted.
        if self.is_banned(&peer.key).await? {
            match action {
                Action::Unblock => return Err(MessengerError::Invalid("cannot unblock a banned key".into())),
                Action::Request | Action::Accept => return Err(MessengerError::Invalid("peer_banned".into())),
                Action::Block | Action::Remove | Action::Decline => {}
            }
        }
        // Removing someone does not make a chat: one deleted here stays
        // deleted, and a system line makes it when there is something to
        // show. The other actions open the chat, as adding a contact did.
        let chat_id = peer.chat_id();
        if action != Action::Remove {
            chats::ensure_dm(&self.store, peer.key.as_hex(), peer.server()).await?;
        }
        let current = self.load_relation(peer).await?;
        if action == Action::Accept && current.peer_signal != PeerSignal::Approved {
            return Err(MessengerError::Invalid("there is no request to accept".into()));
        }
        if action == Action::Decline && current.peer_signal != PeerSignal::Approved {
            return Err(MessengerError::Invalid("there is no request to decline".into()));
        }
        let has_history = repo::count_visible(&self.store, &chat_id).await? > 0;
        let outcome = apply_my_action(&current, action, has_history, with_message);
        let mut next = outcome.next;
        let decided = next != current;
        let mut outbounds = Vec::new();
        let now = self.clock.now().secs();
        let mut last_at = now;
        for s in &outcome.signals {
            let (at, mut out) = self.control(keys, &chat_id, peer, *s, invite).await?;
            last_at = at;
            outbounds.append(&mut out);
        }
        // A decision taken here is dated: by the signal that tells my other
        // devices of it (its copy carries the same time there), or by my
        // clock when it is silent. An action that changes nothing (a second
        // block, a second accept) decides nothing anew.
        if !outcome.signals.is_empty() {
            next.last_my_signal_at = last_at;
            next.decided_at = last_at;
        } else if decided {
            next.decided_at = decided_now(current.decided_at, now);
        }
        if action == Action::Remove || outcome.signals.contains(&Signal::ContactRemoved) {
            self.end_episode(&chat_id, &mut next, None).await?;
        }
        self.save_relation(peer, &next).await?;
        if decided {
            self.mark_relations_owed().await?;
        }
        let mut events = Vec::new();
        if next.blocked != current.blocked && !peer.is_private() {
            events.extend(self.group_chats_follow_block(&peer.key).await?);
        }
        if next.blocked {
            self.purge_held(&chat_id).await?;
            // A stranger I block is not a chat any more: what they wrote
            // goes with the tombstone, the key stays in the blocked list.
            // A contact I block keeps the chat, with its banner.
            if current.my_contact != MyContact::Approved && !self.is_bot_chat(peer) {
                self.delete_chat(&chat_id).await?;
            }
        } else if next.my_contact == MyContact::Approved {
            // Shown as they come to the screen; I am the one who just
            // answered, so nothing of it rings.
            for e in self.release_held(&chat_id, peer, false).await? {
                if let Effect::Emit(ev) = e {
                    events.push(ev);
                }
            }
        }
        if let Some(line) = outcome.system_line {
            if let Some(ev) = self.system_line(&chat_id, peer, line, last_at, false).await? {
                events.push(ev);
            }
        }
        if next.request_floor > current.request_floor {
            for e in self.regate(&chat_id, peer, false).await? {
                if let Effect::Emit(ev) = e {
                    events.push(ev);
                }
            }
        }
        if next != current {
            events.push(self.relationship_event(&chat_id, peer).await?);
        }
        if Self::request_changed(&current, &next) {
            events.push(self.request_event(&peer.key).await?);
            events.extend(self.cap_requests(self.clock.now().secs()).await?.into_iter().filter_map(|e| match e {
                Effect::Emit(ev) => Some(ev),
                _ => None,
            }));
        }
        Ok(ActionResult { relation: self.relation(peer).await?, outbounds, events })
    }

    /// Whether the gate judges a message of the peer that is `historical`.
    /// History is stored as it was only on a fresh database, in its first
    /// session (no previous one set the floor), and only while my list is
    /// not known: with the snapshot of my relations read in full before
    /// the history (`crate::snapshot`), the history passes the gate as
    /// live messages do; with a part of it, for the peers it names. Its
    /// rumor time is the sender's: "older than when I was last online"
    /// says nothing of the message, so it is no way around the gate. A
    /// block drops it either way (`inbound_decision`).
    pub(crate) async fn enforced(&self, peer: &DmPeer, historical: bool) -> Result<bool> {
        if !historical || self.unread_floor.load(Ordering::SeqCst) >= 0 {
            return Ok(true);
        }
        // The relations of private chats travel in a list of their own
        // (`private_peers`), which a full header cuts and which has no word
        // on completeness: a fresh device takes the history of
        // a private chat as it was, as every history before snapshots; a
        // block the list brought still stands (`hears_hidden`, the gate).
        if peer.is_private() {
            return Ok(false);
        }
        Ok(match self.relations_known() {
            Known::Complete => true,
            Known::Partial => dm_relations::get(&self.store, peer.key.as_hex(), peer.server()).await?.is_some(),
            Known::No => false,
        })
    }

    /// My decision about `peer` as another device of mine holds it (the
    /// snapshot of my relations): it stands here by its time, or by its
    /// strictness in the same second (`merge_decision`), and what the
    /// decision does follows, as for the copy of a signal (`on_control`):
    /// a removal ends the episode, a block purges what waited and takes a
    /// stranger's chat, an approval shows what waited, an expiry takes the
    /// request's chat, the book follows. A copy is no decision: nothing
    /// is bumped, nothing is owed to the relays for it. `None` when the
    /// decision here stands.
    pub(crate) async fn apply_decision_copy(
        &self,
        peer: &DmPeer,
        my_contact: MyContact,
        blocked: bool,
        decided_at: i64,
    ) -> Result<Option<Vec<Effect>>> {
        let chat_id = peer.chat_id();
        let current = self.load_relation(peer).await?;
        let incoming = Relationship { my_contact, blocked, decided_at, ..current };
        let mut next = merge_decision(&current, &incoming);
        if next == current {
            return Ok(None);
        }
        if current.my_contact == MyContact::Approved && next.my_contact == MyContact::None && !next.blocked {
            self.end_episode(&chat_id, &mut next, None).await?;
        }
        self.save_relation(peer, &next).await?;
        let mut effects = Vec::new();
        if next.blocked {
            self.purge_held(&chat_id).await?;
            if current.my_contact != MyContact::Approved && !self.is_bot_chat(peer) {
                self.delete_chat(&chat_id).await?;
            }
        } else if next.my_contact == MyContact::Approved {
            effects.extend(self.release_held(&chat_id, peer, false).await?);
        } else if next.my_contact == MyContact::Expired && screen_mode(&current) == ScreenMode::RequestReceived {
            self.delete_chat(&chat_id).await?;
        }
        effects.push(Effect::Emit(self.relationship_event(&chat_id, peer).await?));
        if self.mirror_book(peer, &current, &next).await? {
            effects.push(Effect::Emit(contacts_updated(Some(&peer.key))));
        }
        if next.request_floor > current.request_floor {
            effects.extend(self.regate(&chat_id, peer, false).await?);
        }
        if Self::request_changed(&current, &next) {
            effects.push(Effect::Emit(self.request_event(&peer.key).await?));
            effects.extend(self.cap_requests(self.clock.now().secs()).await?);
        }
        Ok(Some(effects))
    }

    /// The will of the peer as another device of mine holds it (the
    /// snapshot of my relations): what they signalled last and when, and
    /// whether we were ever both approved. Otherwise a new device learns
    /// it from the history alone, and the history it fetches is the newest
    /// part: a contact whose accept and messages are older than that reads
    /// as "request sent". Their newer word stands, as a signal's does
    /// (`apply_peer_signal`); an accept older than my removal or my no
    /// taken here was for the relation I ended and is no request again.
    /// Nothing is sent and nothing is owed: a copy is no word of theirs.
    /// `None` when the will here stands.
    pub(crate) async fn apply_will_copy(&self, peer: &DmPeer, signal: PeerSignal, at: i64, mutual: bool) -> Result<Option<Vec<Effect>>> {
        let chat_id = peer.chat_id();
        let current = self.load_relation(peer).await?;
        let at = at.min(self.clock.now().secs().saturating_add(FLOOR_SKEW_SECS));
        let mut next = current;
        next.was_ever_mutual |= mutual;
        if at > current.last_signal_at {
            next.last_signal_at = at;
            let ended = signal == PeerSignal::Approved
                && matches!(current.my_contact, MyContact::None | MyContact::Declined)
                && current.decided_at > at;
            if !ended {
                next.peer_signal = signal;
            }
            if next.peer_signal == PeerSignal::Approved && next.my_contact == MyContact::Approved {
                next.was_ever_mutual = true;
            }
        }
        if next == current {
            return Ok(None);
        }
        self.save_relation(peer, &next).await?;
        let mut effects = vec![Effect::Emit(self.relationship_event(&chat_id, peer).await?)];
        if Self::request_changed(&current, &next) {
            effects.push(Effect::Emit(self.request_event(&peer.key).await?));
            effects.extend(self.cap_requests(self.clock.now().secs()).await?);
        }
        Ok(Some(effects))
    }

    /// The peer's will changed here: my other devices hear the peer too,
    /// but a new one has my list alone, so the list owes it when it names
    /// the peer (a relation with a decision of mine, `crate::snapshot`).
    async fn owe_will(&self, before: &Relationship, after: &Relationship) -> Result<()> {
        let will = |r: &Relationship| (r.peer_signal, r.was_ever_mutual);
        let named = after.blocked || after.my_contact != MyContact::None || after.decided_at > 0;
        if named && will(before) != will(after) {
            self.mark_relations_owed().await?;
        }
        Ok(())
    }

    /// Whether a hidden envelope of the peer (a signal, an edit, a delete)
    /// is heard at all, decided before anything of it is stored: a key that
    /// is not let in leaves no row behind. A stranger's first word being a
    /// signal (nothing of them here, no chat with them) is not kept: what
    /// they mean is implied by their text when it passes the gate. When I
    /// take messages from my contacts only, a signal of anybody else is not
    /// heard either; my block is kept by the row it already has. An edit or
    /// a delete is my contact's alone: anybody else got one text in, and
    /// changing it is not among what the gate lets them say (`enforced` is
    /// off for the history a fresh database restores, as for their texts).
    pub(crate) async fn hears_hidden(&self, chat_id: &str, peer: &DmPeer, content_type: &str, enforced: bool) -> Result<bool> {
        if self.is_banned(&peer.key).await? {
            return Ok(false);
        }
        if !self.gate_enabled() {
            return Ok(true);
        }
        let r = self.load_relation(peer).await?;
        if content_type == repo::CT_CONTROL {
            if dm_relations::get(&self.store, peer.key.as_hex(), peer.server()).await?.is_none() && chats::get(&self.store, chat_id).await?.is_none() {
                return Ok(false);
            }
            return Ok(self.dm_from().await? != DmFrom::Contacts || r.my_contact == MyContact::Approved);
        }
        // The pinned bot is approved by the build; my block alone shuts it.
        if self.is_bot_chat(peer) {
            return Ok(!r.blocked);
        }
        Ok(!r.blocked && (!enforced || r.my_contact == MyContact::Approved))
    }

    /// Whether a text the peer puts in place of one of theirs may stand: my
    /// contact's, whatever it is; anybody else's only what the gate would
    /// have let in as their one text, so the row of a request never holds
    /// what the gate keeps out. The bot's cards are not edited at all
    /// (`editable_type`), so its key needs no word here.
    pub(crate) async fn edit_allowed(&self, peer: &DmPeer, text: &str, format: TextFormat) -> Result<bool> {
        if !self.gate_enabled() || self.load_relation(peer).await?.my_contact == MyContact::Approved {
            return Ok(true);
        }
        Ok(Self::kind_of_text(text, format).is_request_text())
    }

    // ─── Hooks of the message flow ──────────────────────────────────────────

    /// Before storing my regular message. Errors carry the stable reason
    /// code (`dm_waiting_approval`, …) as the message; `peer_banned` for a
    /// key the network bans, whatever the gate's switch says.
    pub(crate) async fn gate_outbound(&self, chat_id: &str, peer: &DmPeer, me: &str, is_text: bool) -> Result<OutboundGate> {
        if peer.key.as_hex() != me && self.is_banned(&peer.key).await? {
            return Err(MessengerError::Invalid("peer_banned".into()));
        }
        if !self.gate_enabled() || peer.key.as_hex() == me {
            return Ok(OutboundGate { request: false });
        }
        let r = self.load_relation(peer).await?;
        // To the pinned bot a message is a message, never a request: no
        // accept follows it and the bot does not join the address book.
        if self.is_bot_chat(peer) && !r.blocked {
            return Ok(OutboundGate { request: false });
        }
        let sent = repo::count_visible_outgoing(&self.store, chat_id).await?;
        match outbound_permission(&r, sent, is_text) {
            OutboundPermission::Allow => Ok(OutboundGate { request: false }),
            OutboundPermission::AllowAsRequest => Ok(OutboundGate { request: true }),
            OutboundPermission::Deny(reason) => Err(MessengerError::Invalid(reason.as_str().into())),
        }
    }

    /// Before storing a regular message of the peer. `kind` is what the
    /// message is: before approval only a short plain text without a link
    /// gets through, and what is not is dropped, never held.
    pub(crate) async fn gate_inbound(
        &self,
        chat_id: &str,
        peer: &DmPeer,
        at: i64,
        historical: bool,
        kind: InboundKind,
    ) -> Result<InboundGate> {
        // A banned key: nothing is stored, held or shown, and no request
        // comes of it, whatever the gate's switch says.
        if self.is_banned(&peer.key).await? {
            return Ok(InboundGate::Drop);
        }
        if !self.gate_enabled() {
            return Ok(InboundGate::Pass { effects: vec![], regate: false });
        }
        // The pinned bot needs no approval and asks for none: no request,
        // no one-message limit, no signal implied, nothing saved about it.
        // My block is the one thing that stops it.
        if self.is_bot_chat(peer) {
            let blocked = self.load_relation(peer).await?.blocked;
            return Ok(if blocked { InboundGate::Drop } else { InboundGate::Pass { effects: vec![], regate: false } });
        }
        let enforced = self.enforced(peer, historical).await?;
        let mut r = self.load_relation(peer).await?;
        let seen = repo::count_visible_incoming_since(&self.store, chat_id, r.request_floor).await?;
        match inbound_decision(&r, enforced, seen, at, kind, self.dm_from().await?) {
            InboundDecision::Drop(DropReason::SecondMessageBeforeApproval)
                if dm_held::count(&self.store, chat_id).await? < HELD_PER_CHAT =>
            {
                return Ok(InboundGate::Hold);
            }
            InboundDecision::Drop(_) => return Ok(InboundGate::Drop),
            InboundDecision::Save | InboundDecision::SaveAsRequest => {}
        }
        // The peer ended an episode before any message of it was stored
        // here: the first one stored now says where it ended.
        let mut regate = false;
        let pending = dm_relations::end_pending(&self.store, peer.key.as_hex(), peer.server()).await?;
        if pending > 0 {
            dm_relations::set_end_pending(&self.store, peer.key.as_hex(), peer.server(), 0).await?;
            let floor = floor_after_pending_end(r.request_floor, pending, at, self.clock.now().secs());
            if floor > r.request_floor {
                r.request_floor = floor;
                self.save_relation(peer, &r).await?;
                regate = true;
            }
        }
        let effects = if enforced { self.implied_by_message(chat_id, peer, r, at).await? } else { vec![] };
        Ok(InboundGate::Pass { effects, regate })
    }

    /// Writing to me is a statement of will when nothing explicit and newer
    /// says otherwise. For a message being shown now (passed, or released).
    /// The will it implies is the peer's newest word, as a signal's is: a
    /// signal that comes later dated before this message does not undo it.
    /// Its time counts no further than my clock plus the skew, as a signal's.
    async fn implied_by_message(&self, chat_id: &str, peer: &DmPeer, r: Relationship, at: i64) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        if at < r.last_signal_at {
            return Ok(effects);
        }
        let Some(implied) = signal_implied_by_message(&r) else { return Ok(effects) };
        let mut next = r;
        next.peer_signal = implied;
        let cap = self.clock.now().secs().saturating_add(FLOOR_SKEW_SECS);
        next.last_signal_at = r.last_signal_at.max(at.min(cap));
        if r.my_contact == MyContact::Approved {
            next.was_ever_mutual = true;
        }
        self.save_relation(peer, &next).await?;
        self.owe_will(&r, &next).await?;
        let line = if r.my_contact == MyContact::Approved { SystemLine::RequestAccepted } else { SystemLine::RequestReceived };
        if let Some(ev) = self.system_line(chat_id, peer, line, at - 1, true).await? {
            effects.push(Effect::Emit(ev));
        }
        effects.push(Effect::Emit(self.relationship_event(chat_id, peer).await?));
        Ok(effects)
    }

    /// The floor went up: when nothing of the peer is shown above it, the
    /// oldest held row above it that the gate would now take as the request
    /// is shown, with all that a request coming now gets. One row at most:
    /// it is the one message, the rest stay held. `live` = the raise came
    /// with the running session (a notification may follow).
    pub(crate) async fn regate(&self, chat_id: &str, peer: &DmPeer, live: bool) -> Result<Vec<Effect>> {
        if !self.gate_enabled() || self.is_banned(&peer.key).await? {
            return Ok(vec![]);
        }
        let r = self.load_relation(peer).await?;
        if repo::count_visible_incoming_since(&self.store, chat_id, r.request_floor).await? > 0 {
            return Ok(vec![]);
        }
        let dm_from = self.dm_from().await?;
        for id in dm_held::after(&self.store, chat_id, r.request_floor).await? {
            let Some(row) = repo::get(&self.store, &id).await? else { continue };
            if row.sender_pubkey != peer.key.as_hex() || !row.is_hidden {
                continue;
            }
            // Judged again as it would be coming now: what was held passed
            // the kind check then, and my settings may have changed since.
            let kind = Self::kind_of(&row.content_type, row.text.as_deref(), TextFormat::from_name(Some(&row.fmt)));
            if inbound_decision(&r, true, 0, row.created_at, kind, dm_from) != InboundDecision::SaveAsRequest {
                continue;
            }
            if !dm_held::release(&self.store, &id).await? {
                continue;
            }
            let mut effects = self.implied_by_message(chat_id, peer, r, row.created_at).await?;
            effects.extend(self.land_row(row, live).await?);
            return Ok(effects);
        }
        Ok(vec![])
    }

    /// I approved the peer: what they wrote while they waited is theirs to
    /// have said, shown now in its order (the kind check passed when it was
    /// held). Nothing of it is a request any more. Of a banned key nothing
    /// is shown, whoever approved it (an older build of another device of
    /// mine may): what waited stays held, as evidence, until the ban lifts.
    async fn release_held(&self, chat_id: &str, peer: &DmPeer, live: bool) -> Result<Vec<Effect>> {
        if self.is_banned(&peer.key).await? {
            return Ok(vec![]);
        }
        let mut effects = Vec::new();
        for id in dm_held::after(&self.store, chat_id, 0).await? {
            let Some(row) = repo::get(&self.store, &id).await? else { continue };
            if row.sender_pubkey != peer.key.as_hex() || !row.is_hidden || !dm_held::release(&self.store, &id).await? {
                continue;
            }
            effects.extend(self.land_row(row, live).await?);
        }
        Ok(effects)
    }

    /// I blocked the peer: what they wrote too much before is not shown
    /// later. Deleting the chat purges them as well.
    async fn purge_held(&self, chat_id: &str) -> Result<()> {
        dm_held::purge(&self.store, chat_id).await?;
        Ok(())
    }

    /// The peer ended an episode at `at`. The floor goes up to their newest
    /// shown message, not past `at`; when none of theirs is shown above the
    /// floor (the messages of that episode have not come yet), the end
    /// waits for the first one (`floor_after_pending_end`).
    async fn peer_ended_episode(&self, chat_id: &str, peer: &DmPeer, r: &mut Relationship, at: i64) -> Result<()> {
        if repo::count_visible_incoming_since(&self.store, chat_id, r.request_floor).await? > 0 {
            return self.end_episode(chat_id, r, Some(at)).await;
        }
        // `set_end_pending` makes the row when there is none; the caller's
        // save keeps the column.
        if at > dm_relations::end_pending(&self.store, peer.key.as_hex(), peer.server()).await? {
            dm_relations::set_end_pending(&self.store, peer.key.as_hex(), peer.server(), at).await?;
        }
        Ok(())
    }

    /// A control message arrived (already stored as a hidden row).
    /// `invited`: the peer's accept came with an invitation of mine that
    /// admits them (`redeem_invite`, decided before the row).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn on_control(
        &self,
        chat_id: &str,
        peer: &DmPeer,
        from_me: bool,
        action: Option<&str>,
        at: i64,
        historical: bool,
        invited: bool,
    ) -> Result<Vec<Effect>> {
        if !self.gate_enabled() {
            return Ok(vec![]);
        }
        let Some(signal) = action.and_then(Signal::parse) else { return Ok(vec![]) };
        if invited && !from_me && signal == Signal::Accept {
            return self.admit_by_invite(chat_id, peer, at, historical).await;
        }
        let current = self.load_relation(peer).await?;
        if from_me {
            // Another device of mine acted.
            let Some(OwnSignal { mut next, stood }) = apply_own_signal(&current, signal, at) else { return Ok(vec![]) };
            // The copy stood over what was decided here: then it ends the
            // episode as the action did on that device.
            if stood && signal.ends_episode_for_me() {
                self.end_episode(chat_id, &mut next, None).await?;
            }
            self.save_relation(peer, &next).await?;
            // The device that decided may publish no snapshot (an older
            // build): what stood here goes into this device's.
            if stood {
                self.mark_relations_owed().await?;
            }
            let mut effects = vec![Effect::Emit(self.relationship_event(chat_id, peer).await?)];
            if next.blocked != current.blocked && !peer.is_private() {
                effects.extend(self.group_chats_follow_block(&peer.key).await?.into_iter().map(Effect::Emit));
            }
            if next.blocked {
                self.purge_held(chat_id).await?;
                // Blocked elsewhere: a stranger's chat goes here too.
                if current.my_contact != MyContact::Approved && !self.is_bot_chat(peer) {
                    self.delete_chat(chat_id).await?;
                }
            } else if next.my_contact == MyContact::Approved {
                // Accepted elsewhere: what waited here is shown, without a
                // sound — that device already answered for me.
                effects.extend(self.release_held(chat_id, peer, false).await?);
            }
            if self.mirror_book(peer, &current, &next).await? {
                effects.push(Effect::Emit(contacts_updated(Some(&peer.key))));
            }
            if next.request_floor > current.request_floor {
                effects.extend(self.regate(chat_id, peer, !historical).await?);
            }
            // A request that comes back by a signal (my unblock elsewhere)
            // counts toward the size of the queue as one that lands does.
            if Self::request_changed(&current, &next) {
                effects.push(Effect::Emit(self.request_event(&peer.key).await?));
                effects.extend(self.cap_requests(self.clock.now().secs()).await?);
            }
            return Ok(effects);
        }
        // Whether the stranger is heard at all was decided before their
        // signal was stored (`hears_hidden`).
        let outcome = apply_peer_signal(&current, signal, at, !historical, self.clock.now().secs());
        let mut next = outcome.next;
        // Even when it changes nothing else (a withdrawal over their own
        // accept), what they write after it is a new request. Not past
        // their newest stored message: their `at` alone moves nothing, so a
        // no-op or far-future signal opens at most one more message. Also
        // when it comes after a newer signal (one sync brought the new
        // request and its accept first): it still says where that episode
        // ended, and it moves the floor no further than a fresh one would.
        if signal.ends_episode() {
            self.peer_ended_episode(chat_id, peer, &mut next, at).await?;
        }
        if next != current {
            self.save_relation(peer, &next).await?;
            self.owe_will(&current, &next).await?;
        }
        let raised = next.request_floor > current.request_floor;
        if !outcome.applied {
            return if raised { self.regate(chat_id, peer, !historical).await } else { Ok(vec![]) };
        }
        let mut effects = Vec::new();
        if let Some(line) = outcome.system_line {
            if let Some(ev) = self.system_line(chat_id, peer, line, at, false).await? {
                effects.push(Effect::Emit(ev));
            }
        }
        if outcome.confirm_back {
            let keys = self.signer.read().unwrap().clone();
            if let Some(keys) = keys {
                let (at, outs) = self.control(&keys, chat_id, peer, Signal::Accept, None).await?;
                let mut r = next;
                r.last_my_signal_at = at;
                self.save_relation(peer, &r).await?;
                effects.extend(outs.into_iter().map(Effect::Send));
            }
        }
        if raised {
            effects.extend(self.regate(chat_id, peer, !historical).await?);
        }
        effects.push(Effect::Emit(self.relationship_event(chat_id, peer).await?));
        // A fresh accept after my no is a request again: the queue is kept
        // to its size as when one lands.
        if Self::request_changed(&current, &next) {
            effects.push(Effect::Emit(self.request_event(&peer.key).await?));
            effects.extend(self.cap_requests(self.clock.now().secs()).await?);
        }
        Ok(effects)
    }

    /// The peer came in by an invitation of mine: their accept is their
    /// word, and mine follows at once, as if I had pressed "Accept": the
    /// chat opens in full, the accept goes back (for a historical copy
    /// too: my other device, reading this later as history, must answer
    /// as well, or the peer waits on it), the line `invited` is written,
    /// the book follows, the snapshot is owed. Decided here, dated by the
    /// signal. When my other device admitted them already (its copy or
    /// the snapshot came first), nothing is said twice: their use was
    /// recorded, the rest stands. Without the keys of a session (locked)
    /// the decision is taken and the answer waits for their accept to
    /// come again, as a confirm-back does.
    async fn admit_by_invite(&self, chat_id: &str, peer: &DmPeer, at: i64, historical: bool) -> Result<Vec<Effect>> {
        let current = self.load_relation(peer).await?;
        let now = self.clock.now().secs();
        chats::ensure_dm(&self.store, peer.key.as_hex(), peer.server()).await?;
        let outcome = apply_peer_signal(&current, Signal::Accept, at, !historical, now);
        let mut next = outcome.next;
        // Their accept is the newest word of theirs whatever its date: an
        // invitation is mine to hand out, and it names them.
        next.peer_signal = PeerSignal::Approved;
        next.last_signal_at = next.last_signal_at.max(at.min(now.saturating_add(FLOOR_SKEW_SECS)));
        let mut effects = Vec::new();
        if current.my_contact == MyContact::Approved {
            // My contact already: added quietly here, or let in by my other
            // device, whose accept (its copy came) is dated after theirs and
            // answered them; a quiet add sent nothing, so they still wait.
            next.was_ever_mutual = true;
            let answered = current.last_my_signal_at >= at;
            let keys = self.signer.read().unwrap().clone();
            if let Some(keys) = keys.filter(|_| !answered) {
                let (sig_at, outs) = self.control(&keys, chat_id, peer, Signal::Accept, None).await?;
                effects.extend(outs.into_iter().map(Effect::Send));
                next.last_my_signal_at = sig_at;
            }
            if next != current {
                self.save_relation(peer, &next).await?;
                self.owe_will(&current, &next).await?;
            }
            if !current.was_ever_mutual {
                if let Some(ev) = self.system_line(chat_id, peer, SystemLine::Invited, at, false).await? {
                    effects.push(Effect::Emit(ev));
                }
            }
            if next != current {
                effects.push(Effect::Emit(self.relationship_event(chat_id, peer).await?));
            }
            return Ok(effects);
        }
        next.my_contact = MyContact::Approved;
        next.was_ever_mutual = true;
        let keys = self.signer.read().unwrap().clone();
        let decided = match keys {
            Some(keys) => {
                let (sig_at, outs) = self.control(&keys, chat_id, peer, Signal::Accept, None).await?;
                effects.extend(outs.into_iter().map(Effect::Send));
                next.last_my_signal_at = sig_at;
                sig_at
            }
            None => decided_now(current.decided_at, now),
        };
        next.decided_at = decided;
        self.save_relation(peer, &next).await?;
        self.mark_relations_owed().await?;
        if let Some(ev) = self.system_line(chat_id, peer, SystemLine::Invited, at, false).await? {
            effects.push(Effect::Emit(ev));
        }
        effects.extend(self.release_held(chat_id, peer, false).await?);
        if self.mirror_book(peer, &current, &next).await? {
            effects.push(Effect::Emit(contacts_updated(Some(&peer.key))));
        }
        effects.push(Effect::Emit(self.relationship_event(chat_id, peer).await?));
        if Self::request_changed(&current, &next) {
            effects.push(Effect::Emit(self.request_event(&peer.key).await?));
        }
        Ok(effects)
    }
}

/// The id of a system line: of its kind, time and chat. A private chat's
/// line names its server too, or the home chat and the private one with
/// the same person would share one row for a line of the same second.
fn system_line_id(line: SystemLine, at: i64, peer: &DmPeer) -> String {
    match peer.server() {
        None => format!("sys:{}:{}:{}", line.as_str(), at, peer.key.short()),
        Some(server) => format!("sys:{}:{}:{}@{server}", line.as_str(), at, peer.key.short()),
    }
}

/// A secret as the sender may put on an accept: within the bound of the
/// wire, nothing else (its shape is the receiver's to judge).
pub(crate) fn check_invite_secret(secret: &str) -> Result<()> {
    let n = secret.chars().count();
    if n == 0 || n > messenger_core::envelope::MAX_INVITE_CHARS || secret.chars().any(char::is_control) {
        return Err(MessengerError::Invalid("invite_secret".into()));
    }
    Ok(())
}

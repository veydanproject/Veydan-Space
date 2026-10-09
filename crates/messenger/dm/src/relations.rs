// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Side effects around the relationship matrix: loading and saving the
//! state, building control messages, system lines, and the hooks the
//! message flow calls. Decisions themselves are in `relationship`.

use crate::relationship::{
    apply_my_action, apply_own_signal, apply_peer_signal, floor_after_pending_end, inbound_decision, outbound_permission,
    raise_floor, screen_mode, signal_implied_by_message, Action, DropReason, InboundDecision, MyContact, OutboundPermission,
    PeerSignal, Relationship, ScreenMode, Signal, SystemLine,
};
use crate::service::{DmService, UI_EVENT_DM_MESSAGE};
use crate::wrap::{wrap_as, Wake};
use messenger_core::traits::UiEvent;
use messenger_contacts::UI_EVENT_CONTACTS_UPDATED;
use messenger_core::{Effect, Envelope, MessengerError, Outbound, PubKey, Result};
use messenger_store::dm_held;
use messenger_store::dm_relations::{self, RelationRow};
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
    pub mode: String,
    pub my_contact: String,
    pub blocked: bool,
    pub peer_signal: String,
    pub was_ever_mutual: bool,
    pub can_send: bool,
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
    /// What stands between me and `peer`, as the store has it.
    pub async fn load_relation(&self, peer: &PubKey) -> Result<Relationship> {
        Ok(match dm_relations::get(&self.store, peer.as_hex()).await? {
            Some(r) => Relationship {
                my_contact: MyContact::parse(&r.my_contact).unwrap_or(MyContact::None),
                blocked: r.blocked,
                peer_signal: PeerSignal::parse(&r.peer_signal).unwrap_or(PeerSignal::None),
                was_ever_mutual: r.was_ever_mutual,
                last_signal_at: r.last_signal_at,
                last_my_signal_at: r.last_my_signal_at,
                request_floor: r.request_floor,
            },
            None => Relationship::default(),
        })
    }

    pub(crate) async fn save_relation(&self, peer: &PubKey, r: &Relationship) -> Result<()> {
        dm_relations::put(
            &self.store,
            &RelationRow {
                peer_pubkey: peer.as_hex().to_string(),
                my_contact: r.my_contact.as_str().into(),
                blocked: r.blocked,
                peer_signal: r.peer_signal.as_str().into(),
                was_ever_mutual: r.was_ever_mutual,
                last_signal_at: r.last_signal_at,
                last_my_signal_at: r.last_my_signal_at,
                request_floor: r.request_floor,
                created_at: 0,
                updated_at: 0,
            },
        )
        .await
    }

    /// Screen mode and whether the composer may be used.
    pub(crate) async fn mode_of(&self, chat_id: &str, peer: &PubKey) -> Result<(ScreenMode, bool)> {
        if !self.gate_enabled() {
            return Ok((ScreenMode::FullChat, true));
        }
        let r = self.load_relation(peer).await?;
        let sent = repo::count_visible_outgoing(&self.store, chat_id).await?;
        let can = !matches!(outbound_permission(&r, sent, true), OutboundPermission::Deny(_));
        Ok((screen_mode(&r), can))
    }

    pub async fn relation(&self, peer: &PubKey) -> Result<RelationView> {
        let chat_id = chats::dm_chat_id(peer.as_hex());
        let r = self.load_relation(peer).await?;
        let (mode, can_send) = self.mode_of(&chat_id, peer).await?;
        Ok(RelationView {
            peer_pubkey: peer.as_hex().to_string(),
            mode: mode.as_str().into(),
            my_contact: r.my_contact.as_str().into(),
            blocked: r.blocked,
            peer_signal: r.peer_signal.as_str().into(),
            was_ever_mutual: r.was_ever_mutual,
            can_send,
        })
    }

    pub async fn is_blocked(&self, peer: &PubKey) -> Result<bool> {
        Ok(self.load_relation(peer).await?.blocked)
    }

    pub async fn blocked_peers(&self) -> Result<Vec<String>> {
        dm_relations::blocked_peers(&self.store).await
    }

    /// The address book is kept on each device apart, and presence and the
    /// profile follow it. What another device of mine decided about `peer`
    /// is put into this one's book, as `dm_act` does for a decision taken
    /// here. Returns whether the book changed.
    async fn mirror_book(&self, peer: &PubKey, before: &Relationship, after: &Relationship) -> Result<bool> {
        let Some(me) = self.my_key().await?.as_deref().and_then(PubKey::parse) else { return Ok(false) };
        if &me == peer {
            return Ok(false);
        }
        let approved = after.my_contact == MyContact::Approved && !after.blocked;
        if approved && before.my_contact != MyContact::Approved && !self.contacts.is_contact(peer).await? {
            self.contacts.add(&me, peer.as_hex(), None).await?;
            return Ok(true);
        }
        let removed = before.my_contact == MyContact::Approved && after.my_contact == MyContact::None;
        if removed && self.contacts.is_contact(peer).await? {
            self.contacts.remove(peer).await?;
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

    async fn relationship_event(&self, chat_id: &str, peer: &PubKey) -> Result<UiEvent> {
        let (mode, can_send) = self.mode_of(chat_id, peer).await?;
        Ok(UiEvent {
            name: UI_EVENT_DM_RELATIONSHIP.into(),
            payload: serde_json::json!({
                "chat_id": chat_id, "peer": peer.as_hex(), "mode": mode.as_str(), "can_send": can_send,
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
        peer: &PubKey,
        line: SystemLine,
        at: i64,
        of_message: bool,
    ) -> Result<Option<UiEvent>> {
        if !of_message && at <= chats::cleared_at(&self.store, chat_id).await? {
            return Ok(None);
        }
        let id = format!("sys:{}:{}:{}", line.as_str(), at, peer.short());
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
        chats::ensure_dm(&self.store, peer.as_hex()).await?;
        let Some(view) = self.message(&id).await? else { return Ok(None) };
        Ok(Some(UiEvent {
            name: UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": view, "historical": true }),
        }))
    }

    /// Build one control message (wrap for the peer + self-copy) and record
    /// it as a hidden row, so a copy coming back is recognised as ours.
    async fn control(&self, keys: &Keys, chat_id: &str, peer: &PubKey, signal: Signal) -> Result<(i64, Vec<Outbound>)> {
        let content = Envelope::control(signal.as_str()).encode();
        let at = self.next_created_at(chat_id).await?;
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
        let mut out = vec![Outbound::PublishToInbox {
            recipient: peer.clone(),
            event: w.to_peer,
            hint_relays: self.hints(peer).await?,
        }];
        if let Some(event) = w.to_self {
            out.push(Outbound::PublishOwn { event });
        }
        Ok((at, out))
    }

    /// One of my actions: state, signals, system line, events.
    pub async fn act(&self, keys: &Keys, peer: &PubKey, action: Action) -> Result<ActionResult> {
        self.act_with(keys, peer, action, false).await
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

    pub(crate) async fn act_with(&self, keys: &Keys, peer: &PubKey, action: Action, with_message: bool) -> Result<ActionResult> {
        if peer.as_hex() == keys.public_key().to_hex() {
            return Err(MessengerError::Invalid("that is your own key".into()));
        }
        // Removing someone does not make a chat: one deleted here stays
        // deleted, and a system line makes it when there is something to
        // show. The other actions open the chat, as adding a contact did.
        let chat_id = chats::dm_chat_id(peer.as_hex());
        if action != Action::Remove {
            chats::ensure_dm(&self.store, peer.as_hex()).await?;
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
        let mut outbounds = Vec::new();
        let mut last_at = self.clock.now().secs();
        for s in &outcome.signals {
            let (at, mut out) = self.control(keys, &chat_id, peer, *s).await?;
            last_at = at;
            outbounds.append(&mut out);
        }
        if !outcome.signals.is_empty() {
            next.last_my_signal_at = last_at;
        }
        if action == Action::Remove || outcome.signals.contains(&Signal::ContactRemoved) {
            self.end_episode(&chat_id, &mut next, None).await?;
        }
        self.save_relation(peer, &next).await?;
        if next.blocked || next.my_contact == MyContact::Approved {
            self.purge_held(&chat_id).await?;
        }

        let mut events = Vec::new();
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
        Ok(ActionResult { relation: self.relation(peer).await?, outbounds, events })
    }

    // ─── Hooks of the message flow ──────────────────────────────────────────

    /// Before storing my regular message. Errors carry the stable reason
    /// code (`dm_waiting_approval`, …) as the message.
    pub(crate) async fn gate_outbound(&self, chat_id: &str, peer: &PubKey, me: &str, is_text: bool) -> Result<OutboundGate> {
        if !self.gate_enabled() || peer.as_hex() == me {
            return Ok(OutboundGate { request: false });
        }
        let r = self.load_relation(peer).await?;
        let sent = repo::count_visible_outgoing(&self.store, chat_id).await?;
        match outbound_permission(&r, sent, is_text) {
            OutboundPermission::Allow => Ok(OutboundGate { request: false }),
            OutboundPermission::AllowAsRequest => Ok(OutboundGate { request: true }),
            OutboundPermission::Deny(reason) => Err(MessengerError::Invalid(reason.as_str().into())),
        }
    }

    /// Before storing a regular message of the peer.
    pub(crate) async fn gate_inbound(&self, chat_id: &str, peer: &PubKey, at: i64, historical: bool) -> Result<InboundGate> {
        if !self.gate_enabled() {
            return Ok(InboundGate::Pass { effects: vec![], regate: false });
        }
        let floor = self.unread_floor.load(Ordering::SeqCst);
        let enforced = !historical || (floor >= 0 && at >= floor);
        let mut r = self.load_relation(peer).await?;
        let seen = repo::count_visible_incoming_since(&self.store, chat_id, r.request_floor).await?;
        match inbound_decision(&r, enforced, seen, at) {
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
        let pending = dm_relations::end_pending(&self.store, peer.as_hex()).await?;
        if pending > 0 {
            dm_relations::set_end_pending(&self.store, peer.as_hex(), 0).await?;
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
    async fn implied_by_message(&self, chat_id: &str, peer: &PubKey, r: Relationship, at: i64) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        if at < r.last_signal_at {
            return Ok(effects);
        }
        let Some(implied) = signal_implied_by_message(&r) else { return Ok(effects) };
        let mut next = r;
        next.peer_signal = implied;
        if r.my_contact == MyContact::Approved {
            next.was_ever_mutual = true;
        }
        self.save_relation(peer, &next).await?;
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
    pub(crate) async fn regate(&self, chat_id: &str, peer: &PubKey, live: bool) -> Result<Vec<Effect>> {
        if !self.gate_enabled() {
            return Ok(vec![]);
        }
        let r = self.load_relation(peer).await?;
        if repo::count_visible_incoming_since(&self.store, chat_id, r.request_floor).await? > 0 {
            return Ok(vec![]);
        }
        for id in dm_held::after(&self.store, chat_id, r.request_floor).await? {
            let Some(row) = repo::get(&self.store, &id).await? else { continue };
            if row.sender_pubkey != peer.as_hex() || !row.is_hidden {
                continue;
            }
            if inbound_decision(&r, true, 0, row.created_at) != InboundDecision::SaveAsRequest {
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

    /// I approved or blocked the peer: what they wrote too much before is
    /// not shown later. Deleting the chat purges them as well.
    async fn purge_held(&self, chat_id: &str) -> Result<()> {
        dm_held::purge(&self.store, chat_id).await?;
        Ok(())
    }

    /// The peer ended an episode at `at`. The floor goes up to their newest
    /// shown message, not past `at`; when none of theirs is shown above the
    /// floor (the messages of that episode have not come yet), the end
    /// waits for the first one (`floor_after_pending_end`).
    async fn peer_ended_episode(&self, chat_id: &str, peer: &PubKey, r: &mut Relationship, at: i64) -> Result<()> {
        if repo::count_visible_incoming_since(&self.store, chat_id, r.request_floor).await? > 0 {
            return self.end_episode(chat_id, r, Some(at)).await;
        }
        // `set_end_pending` makes the row when there is none; the caller's
        // save keeps the column.
        if at > dm_relations::end_pending(&self.store, peer.as_hex()).await? {
            dm_relations::set_end_pending(&self.store, peer.as_hex(), at).await?;
        }
        Ok(())
    }

    /// A control message arrived (already stored as a hidden row).
    pub(crate) async fn on_control(
        &self,
        chat_id: &str,
        peer: &PubKey,
        from_me: bool,
        action: Option<&str>,
        at: i64,
        historical: bool,
    ) -> Result<Vec<Effect>> {
        if !self.gate_enabled() {
            return Ok(vec![]);
        }
        let Some(signal) = action.and_then(Signal::parse) else { return Ok(vec![]) };
        let current = self.load_relation(peer).await?;
        if from_me {
            // Another device of mine acted.
            let Some(mut next) = apply_own_signal(&current, signal, at) else { return Ok(vec![]) };
            if signal.ends_episode_for_me() {
                self.end_episode(chat_id, &mut next, None).await?;
            }
            self.save_relation(peer, &next).await?;
            if matches!(signal, Signal::Accept | Signal::Block) {
                self.purge_held(chat_id).await?;
            }
            let mut effects = vec![Effect::Emit(self.relationship_event(chat_id, peer).await?)];
            if self.mirror_book(peer, &current, &next).await? {
                effects.push(Effect::Emit(contacts_updated(Some(peer))));
            }
            if next.request_floor > current.request_floor {
                effects.extend(self.regate(chat_id, peer, !historical).await?);
            }
            return Ok(effects);
        }
        let outcome = apply_peer_signal(&current, signal, at, !historical);
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
                let (at, outs) = self.control(&keys, chat_id, peer, Signal::Accept).await?;
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
        Ok(effects)
    }
}

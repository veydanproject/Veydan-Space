// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Side effects around the relationship matrix: loading and saving the
//! state, building control messages, system lines, and the hooks the
//! message flow calls. Decisions themselves are in `relationship`.

use crate::relationship::{
    apply_my_action, apply_own_signal, apply_peer_signal, inbound_decision, outbound_permission, screen_mode,
    signal_implied_by_message, Action, InboundDecision, MyContact, OutboundPermission, PeerSignal, Relationship,
    ScreenMode, Signal, SystemLine,
};
use crate::service::{DmService, UI_EVENT_DM_MESSAGE};
use crate::wrap::{wrap_as, Wake};
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, Envelope, MessengerError, Outbound, PubKey, Result};
use messenger_store::chats;
use messenger_store::dm_relations::{self, RelationRow};
use messenger_store::messages::{self as repo, NewMessage};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

pub const UI_EVENT_DM_RELATIONSHIP: &str = "dm.relationship";

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
    async fn system_line(&self, chat_id: &str, peer: &PubKey, line: SystemLine, at: i64) -> Result<Option<UiEvent>> {
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
        let at = {
            let now = self.clock.now().secs();
            match repo::last_created_at(&self.store, chat_id).await? {
                Some(last) if last >= now => last + 1,
                _ => now,
            }
        };
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

    pub(crate) async fn act_with(&self, keys: &Keys, peer: &PubKey, action: Action, with_message: bool) -> Result<ActionResult> {
        if peer.as_hex() == keys.public_key().to_hex() {
            return Err(MessengerError::Invalid("that is your own key".into()));
        }
        let chat = chats::ensure_dm(&self.store, peer.as_hex()).await?;
        let current = self.load_relation(peer).await?;
        if action == Action::Accept && current.peer_signal != PeerSignal::Approved {
            return Err(MessengerError::Invalid("there is no request to accept".into()));
        }
        if action == Action::Decline && current.peer_signal != PeerSignal::Approved {
            return Err(MessengerError::Invalid("there is no request to decline".into()));
        }
        let has_history = repo::count_visible(&self.store, &chat.id).await? > 0;
        let outcome = apply_my_action(&current, action, has_history, with_message);
        let mut next = outcome.next;
        let mut outbounds = Vec::new();
        let mut last_at = self.clock.now().secs();
        for s in &outcome.signals {
            let (at, mut out) = self.control(keys, &chat.id, peer, *s).await?;
            last_at = at;
            outbounds.append(&mut out);
        }
        if !outcome.signals.is_empty() {
            next.last_my_signal_at = last_at;
        }
        self.save_relation(peer, &next).await?;

        let mut events = Vec::new();
        if let Some(line) = outcome.system_line {
            if let Some(ev) = self.system_line(&chat.id, peer, line, last_at).await? {
                events.push(ev);
            }
        }
        if next != current {
            events.push(self.relationship_event(&chat.id, peer).await?);
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

    /// Before storing a regular message of the peer. `Ok(None)` = drop.
    pub(crate) async fn gate_inbound(
        &self,
        chat_id: &str,
        peer: &PubKey,
        at: i64,
        historical: bool,
    ) -> Result<Option<Vec<Effect>>> {
        if !self.gate_enabled() {
            return Ok(Some(vec![]));
        }
        let floor = self.unread_floor.load(Ordering::SeqCst);
        let enforced = !historical || (floor >= 0 && at >= floor);
        let r = self.load_relation(peer).await?;
        let seen = repo::count_visible_incoming(&self.store, chat_id).await?;
        let decision = inbound_decision(&r, enforced, seen);
        if matches!(decision, InboundDecision::Drop(_)) {
            return Ok(None);
        }
        let mut effects = Vec::new();
        // Writing to me is a statement of will when nothing explicit and
        // newer says otherwise.
        if enforced && at >= r.last_signal_at {
            if let Some(implied) = signal_implied_by_message(&r) {
                let mut next = r;
                next.peer_signal = implied;
                if r.my_contact == MyContact::Approved {
                    next.was_ever_mutual = true;
                }
                self.save_relation(peer, &next).await?;
                let line = if r.my_contact == MyContact::Approved {
                    SystemLine::RequestAccepted
                } else {
                    SystemLine::RequestReceived
                };
                if let Some(ev) = self.system_line(chat_id, peer, line, at - 1).await? {
                    effects.push(Effect::Emit(ev));
                }
                effects.push(Effect::Emit(self.relationship_event(chat_id, peer).await?));
            }
        }
        Ok(Some(effects))
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
            let Some(next) = apply_own_signal(&current, signal, at) else { return Ok(vec![]) };
            self.save_relation(peer, &next).await?;
            return Ok(vec![Effect::Emit(self.relationship_event(chat_id, peer).await?)]);
        }
        let outcome = apply_peer_signal(&current, signal, at, !historical);
        if outcome.next != current {
            self.save_relation(peer, &outcome.next).await?;
        }
        if !outcome.applied {
            return Ok(vec![]);
        }
        let mut effects = Vec::new();
        if let Some(line) = outcome.system_line {
            if let Some(ev) = self.system_line(chat_id, peer, line, at).await? {
                effects.push(Effect::Emit(ev));
            }
        }
        if outcome.confirm_back {
            let keys = self.signer.read().unwrap().clone();
            if let Some(keys) = keys {
                let (at, outs) = self.control(&keys, chat_id, peer, Signal::Accept).await?;
                let mut r = outcome.next;
                r.last_my_signal_at = at;
                self.save_relation(peer, &r).await?;
                effects.extend(outs.into_iter().map(Effect::Send));
            }
        }
        effects.push(Effect::Emit(self.relationship_event(chat_id, peer).await?));
        Ok(effects)
    }
}

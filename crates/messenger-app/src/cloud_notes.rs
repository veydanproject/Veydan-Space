// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The service notes of the module `cloud` between the identity and its
//! contacts (internal/messenger-wire.md §3, "Восстановление доступа"), and
//! the address book as the module sees it:
//!
//! - [`send`]: a note of `PEER_TYPES` to an approved contact, wrapped and
//!   queued by the runtime (`send_peer_note`);
//! - [`CloudInbox`]: the notes the DM service hands over as the event
//!   `cloud.note` (a `recovery.*` note from an approved contact, or from my
//!   other devices), given to every listener of `PeerNotes::incoming`; a
//!   note that comes before any listener waits for the first, as a card of
//!   the bot does;
//! - [`ApprovedContacts`]: the seam `Contacts`, the contacts we both chose
//!   to talk with (`full_chat`), by the name they are known by here.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use messenger_core::envelope::Envelope;
use messenger_core::PubKey;
use messenger_runtime::cloudnotes::UI_EVENT_BOT_CARD;
use messenger_runtime::cloudnotes::UI_EVENT_CLOUD_NOTE;
use messenger_runtime::MessengerRuntime;
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;
use veydan_shell::seams::{code, ApprovedContact, Card, CloudError, Contacts, Note, SeamFuture};

use crate::cloud_link::from_runtime;
use crate::commands::MessengerState;

/// How many notes wait for a slow listener, and for the first one.
const QUEUE: usize = 64;

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn stopped(message: &str) -> CloudError {
    CloudError::new(code::MESSENGER_STOPPED, message)
}

/// The runtime of the messenger of `app`, while it runs.
pub(crate) fn runtime_of(app: &AppHandle) -> Result<Arc<MessengerRuntime>, CloudError> {
    let state = app
        .try_state::<MessengerState>()
        .ok_or_else(|| stopped("the messenger is not in the product"))?;
    state.runtime().map_err(|e| stopped(&e.to_string()))
}

/// A note of `PEER_TYPES` (checked by the caller) to the approved contact
/// `peer`, 64 hex: sealed by the identity and queued for its inbox relays.
/// A contact we did not both choose to talk with is `bad_request`.
pub async fn send(app: &AppHandle, note: &Note, peer: &str) -> Result<(), CloudError> {
    let rt = runtime_of(app)?;
    let peer = PubKey::parse(peer)
        .filter(|_| peer.bytes().all(|b| !b.is_ascii_uppercase()))
        .ok_or_else(|| CloudError::new(code::BAD_REQUEST, "a key is 64 hex characters in lower case"))?;
    let mut envelope = Envelope::new(&note.t);
    envelope.fields = note.fields.clone();
    envelope.fields.remove("v");
    envelope.fields.remove("t");
    let expiration = note
        .expires_at
        .map(|at| i64::try_from(at).map_err(|_| CloudError::new(code::BAD_REQUEST, "the expiration is out of range")))
        .transpose()?;
    rt.send_peer_note(&peer, &envelope, expiration).await.map_err(|e| match e {
        messenger_core::MessengerError::Invalid(m) if m == "not_approved" => {
            CloudError::new(code::BAD_REQUEST, "the peer is not a contact we both chose to talk with")
        }
        other => from_runtime(other),
    })
}

#[derive(Default)]
struct InboxState {
    listeners: Vec<mpsc::Sender<Note>>,
    /// Notes no listener took yet: given to the first that comes.
    pending: VecDeque<Note>,
}

/// The notes of the module `cloud` that came to the identity, to every
/// listener; kept by the app for the whole run, so a runtime that starts
/// again feeds the same listeners.
#[derive(Default)]
pub struct CloudInbox {
    state: Mutex<InboxState>,
}

impl CloudInbox {
    /// The inbox of `app`, made at the first use.
    pub fn of(app: &AppHandle) -> Arc<CloudInbox> {
        if let Some(inbox) = app.try_state::<Arc<CloudInbox>>() {
            return inbox.inner().clone();
        }
        let inbox = Arc::new(CloudInbox::default());
        app.manage(inbox.clone());
        // Another thread may have managed one first: the managed one wins.
        app.try_state::<Arc<CloudInbox>>()
            .map(|kept| kept.inner().clone())
            .unwrap_or(inbox)
    }

    /// A note from the DM service: to every listener; a listener that is
    /// gone is forgotten; without one it waits, at most [`QUEUE`] of them.
    pub fn offer(&self, note: Note) {
        let mut state = guard(&self.state);
        let mut taken = false;
        state.listeners.retain(|listener| match listener.try_send(note.clone()) {
            Ok(()) => {
                taken = true;
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
        if !taken {
            state.pending.push_back(note);
            if state.pending.len() > QUEUE {
                state.pending.pop_front();
            }
        }
    }

    /// A new listener, given what waited.
    pub fn listen(&self) -> mpsc::Receiver<Note> {
        let (tx, rx) = mpsc::channel(QUEUE);
        let mut state = guard(&self.state);
        while let Some(note) = state.pending.pop_front() {
            if let Err(e) = tx.try_send(note) {
                state.pending.push_front(e.into_inner());
                break;
            }
        }
        state.listeners.push(tx);
        rx
    }
}

/// The note of an event `cloud.note`, when the payload is one.
pub fn note_of_event(payload: &Value) -> Option<Note> {
    let t = payload.get("t")?.as_str()?;
    let from = payload.get("from")?.as_str()?;
    let fields: Map<String, Value> = match payload.get("fields") {
        Some(Value::Object(fields)) => fields.clone(),
        _ => Map::new(),
    };
    if !t.starts_with("recovery.") || PubKey::parse(from).is_none() {
        return None;
    }
    let mut note = Note::new(t, fields);
    note.from = Some(from.to_ascii_lowercase());
    Some(note)
}

/// The card of an event `cloud.card` (`messenger_dm::botcard`: the DM
/// service tells of a card of the pinned bot once, when it puts it into the
/// bot's chat), with who sealed it; the pin is judged once more by
/// [`crate::cloud_seams::PinnedCards`].
pub fn card_of_event(payload: &Value) -> Option<(String, Card)> {
    let id = payload.get("id")?.as_str()?;
    let t = payload.get("t")?.as_str()?;
    let from = payload.get("from")?.as_str()?;
    let at = payload.get("at")?.as_u64()?;
    let fields: Map<String, Value> = match payload.get("fields") {
        Some(Value::Object(fields)) => fields.clone(),
        _ => Map::new(),
    };
    if id.len() != 64 || PubKey::parse(from).is_none() || t.is_empty() {
        return None;
    }
    Some((from.to_ascii_lowercase(), Card { id: id.to_string(), t: t.to_string(), fields, at }))
}

/// Feeds the inbox of `app` from the events of `rt` for as long as the
/// runtime runs, and the pinned cards of the bot: a task of `serve`.
pub fn spawn_forwarder(app: AppHandle, rt: Arc<MessengerRuntime>) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let inbox = CloudInbox::of(&app);
        let mut rx = rt.ui_events();
        loop {
            match rx.recv().await {
                Ok(event) if event.name == UI_EVENT_CLOUD_NOTE => {
                    if let Some(note) = note_of_event(&event.payload) {
                        inbox.offer(note);
                    }
                }
                Ok(event) if event.name == UI_EVENT_BOT_CARD => {
                    if let Some((from, card)) = card_of_event(&event.payload) {
                        if let Some(cards) = app.try_state::<Arc<crate::cloud_seams::PinnedCards>>() {
                            cards.offer(&from, card);
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    })
}

/// The seam `Contacts` over the address book and the relationships of the
/// messenger.
pub struct ApprovedContacts {
    pub app: AppHandle,
}

impl Contacts for ApprovedContacts {
    fn approved(&self) -> SeamFuture<'_, Vec<ApprovedContact>> {
        Box::pin(async move {
            let rt = runtime_of(&self.app)?;
            let mut out = Vec::new();
            for contact in rt.contacts().list().await.map_err(from_runtime)? {
                let Some(pk) = PubKey::parse(&contact.pubkey) else { continue };
                if rt.dm().recovery_notes_allowed(&pk).await.map_err(from_runtime)? {
                    out.push(ApprovedContact {
                        pubkey: pk.as_hex().to_string(),
                        name: contact.label(),
                    });
                }
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_note_is_read_from_the_event_and_a_stranger_type_is_not() {
        let payload = json!({ "t": "recovery.share", "fields": { "share": "AQID" }, "from": "AB".repeat(32) });
        let note = note_of_event(&payload).expect("note");
        assert_eq!(note.t, "recovery.share");
        assert_eq!(note.fields["share"], "AQID");
        assert_eq!(note.from.as_deref(), Some("ab".repeat(32).as_str()));
        assert!(note_of_event(&json!({ "t": "receipt.read", "fields": {}, "from": "ab".repeat(32) })).is_none());
        assert!(note_of_event(&json!({ "t": "recovery.ping", "fields": {}, "from": "nobody" })).is_none());
    }

    #[test]
    fn a_card_is_read_from_the_event_the_dm_service_tells_it_with() {
        // The shape of `messenger_dm::botcard::card_event`.
        let (id, bot) = ("1f".repeat(32), "AB".repeat(32));
        let payload = json!({
            "id": id, "t": "recovery.requested", "from": bot, "at": 1_791_012_345u64,
            "fields": { "request_id": "9c", "expires_at": 1_791_014_145u64 },
        });
        let (from, card) = card_of_event(&payload).expect("card");
        assert_eq!(from, "ab".repeat(32), "the signer as the pin is written");
        assert_eq!((card.id.as_str(), card.t.as_str(), card.at), (id.as_str(), "recovery.requested", 1_791_012_345));
        assert_eq!(card.fields["request_id"], "9c");
        // An unknown type of the bot is a card too, with no fields.
        let bare = json!({ "id": id, "t": "plan.renewed", "from": bot, "at": 5 });
        assert!(card_of_event(&bare).is_some_and(|(_, c)| c.fields.is_empty()));
        for broken in [
            json!({ "id": "short", "t": "recovery.requested", "from": bot, "at": 5 }),
            json!({ "id": id, "t": "", "from": bot, "at": 5 }),
            json!({ "id": id, "t": "recovery.requested", "from": "nobody", "at": 5 }),
            json!({ "id": id, "t": "recovery.requested", "from": bot, "at": -5 }),
        ] {
            assert!(card_of_event(&broken).is_none(), "{broken}");
        }
    }

    #[tokio::test]
    async fn the_inbox_keeps_what_came_before_a_listener_and_feeds_every_listener() {
        let inbox = CloudInbox::default();
        inbox.offer(Note::new("recovery.share.ack", Map::new()));
        let mut first = inbox.listen();
        assert_eq!(first.recv().await.map(|n| n.t).as_deref(), Some("recovery.share.ack"));
        let mut second = inbox.listen();
        inbox.offer(Note::new("recovery.share.pong", Map::new()));
        assert_eq!(first.recv().await.map(|n| n.t).as_deref(), Some("recovery.share.pong"));
        assert_eq!(second.recv().await.map(|n| n.t).as_deref(), Some("recovery.share.pong"));
        drop(first);
        inbox.offer(Note::new("recovery.share.revoke", Map::new()));
        assert_eq!(second.recv().await.map(|n| n.t).as_deref(), Some("recovery.share.revoke"));
        assert_eq!(guard(&inbox.state).listeners.len(), 1, "the dropped listener is forgotten");
    }
}

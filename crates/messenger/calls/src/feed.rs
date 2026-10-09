// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The record of a call (`msg_calls`) and its line in the chat: a system
//! row of `msg_messages` with the id `sys:call:<call_id>`, as the groups
//! write theirs for their operations. The row is made when the call
//! begins (so an outgoing or a ringing call is already in the chat) and
//! its details are written again when the call ends; the chat shows the
//! row, the host words it from the details:
//!
//! ```json
//! { "call_id": "…", "direction": "in"|"out", "media": "audio"|"video",
//!   "outcome": null|"missed"|"declined"|"busy"|"ended"|"failed",
//!   "duration_secs": null|n, "via": null|"direct"|"relay", "started_at": n }
//! ```
//!
//! A missed call of the peer counts as unread, so that it is seen.

use crate::call::{CallView, Direction, Outcome, UI_EVENT_CALL_ENDED};
use crate::engine::{Media, PairKind};
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, Result};
use messenger_dm::{DmService, UI_EVENT_DM_MESSAGE, UI_EVENT_DM_UPDATED};
use messenger_store::calls::{self as repo, CallRow, NewCall};
use messenger_store::messages::{self as msgs, NewMessage};
use messenger_store::{chats, Store};

/// The chat list shows this for a call: a word would be in one language.
const PREVIEW: &str = "📞";
/// The text of the system row; the facts are in its details.
pub const SYSTEM_TEXT: &str = "call";

pub fn system_id(call_id: &str) -> String {
    format!("sys:call:{call_id}")
}

/// The details of the system row, from the record.
pub fn details(row: &CallRow) -> serde_json::Value {
    serde_json::json!({
        "call_id": row.call_id,
        "direction": row.direction,
        "media": row.media,
        "outcome": row.outcome,
        "duration_secs": row.duration_secs(),
        "via": row.outcome.as_ref().map(|_| if row.via_relay { "relay" } else { "direct" }),
        "started_at": row.started_at,
    })
}

#[derive(Clone)]
pub(crate) struct Feed {
    store: Store,
    dm: DmService,
}

impl Feed {
    pub fn new(store: Store, dm: DmService) -> Self {
        Self { store, dm }
    }

    pub async fn get(&self, call_id: &str) -> Result<Option<CallRow>> {
        repo::get(&self.store, call_id).await
    }

    /// A call begins, or I learn of one: the record and the line, once.
    /// Nothing when the call is on record already (a copy).
    pub async fn begin(&self, call_id: &str, peer: &str, direction: Direction, media: Media, started_at: i64) -> Result<Vec<Effect>> {
        // The chat is made only for a line that is really new: a copy of a
        // call on record, or one from before the chat was deleted here,
        // does not bring the chat back.
        let chat_id = chats::dm_chat_id(peer);
        let new = NewCall {
            call_id: call_id.to_string(),
            chat_id: chat_id.clone(),
            peer: peer.to_string(),
            direction: direction.as_str().to_string(),
            media: media.as_str().to_string(),
            started_at,
        };
        if !repo::insert(&self.store, &new).await? {
            return Ok(vec![]);
        }
        // Before the deletion (a call I start at once may share its second).
        if started_at < chats::cleared_at(&self.store, &chat_id).await? {
            return Ok(vec![]);
        }
        let row = repo::get(&self.store, call_id).await?.ok_or_else(|| messenger_core::MessengerError::Storage("call vanished".into()))?;
        let id = system_id(call_id);
        let inserted = msgs::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: None,
                direction: direction.as_str().to_string(),
                status: msgs::STATUS_SENT.into(),
                content_type: msgs::CT_SYSTEM.into(),
                text: Some(SYSTEM_TEXT.into()),
                envelope_json: "{}".into(),
                sender_pubkey: String::new(),
                reply_to_id: None,
                target_id: None,
                created_at: started_at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: Some(details(&row).to_string()),
            },
        )
        .await?;
        if !inserted {
            return Ok(vec![]);
        }
        chats::ensure_dm(&self.store, peer).await?;
        chats::touch(&self.store, &chat_id, started_at, Some(PREVIEW), false).await?;
        let Some(view) = self.dm.message(&id).await? else { return Ok(vec![]) };
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": view, "historical": true }),
        })])
    }

    pub async fn answered(&self, call_id: &str, at: i64) -> Result<()> {
        repo::set_answered(&self.store, call_id, at).await
    }

    pub async fn via(&self, call_id: &str, kind: PairKind) -> Result<()> {
        repo::set_via_relay(&self.store, call_id, kind == PairKind::Relay).await
    }

    /// The call is over: the record is closed, the line told again, the
    /// screen told with `call.ended`. The first end holds; a later word of
    /// the same end (a copy) changes nothing and tells nothing.
    pub async fn finish(&self, view: &CallView, outcome: Outcome, ended_at: i64) -> Result<Vec<Effect>> {
        let call_id = &view.call_id;
        if !repo::finish(&self.store, call_id, outcome.as_str(), ended_at).await? {
            return Ok(vec![]);
        }
        let Some(row) = repo::get(&self.store, call_id).await? else { return Ok(vec![]) };
        let id = system_id(call_id);
        msgs::set_media_json(&self.store, &id, &details(&row).to_string()).await?;
        let missed_me = outcome == Outcome::Missed && row.direction == repo::DIR_IN;
        chats::touch(&self.store, &row.chat_id, row.started_at, Some(PREVIEW), missed_me).await?;
        let mut shown = view.clone();
        shown.phase = crate::call::Phase::Ended;
        shown.answered_at = row.answered_at;
        Ok(vec![
            Effect::Emit(UiEvent {
                name: UI_EVENT_DM_UPDATED.into(),
                payload: serde_json::json!({ "chat_id": row.chat_id, "message_id": id }),
            }),
            Effect::Emit(UiEvent {
                name: UI_EVENT_CALL_ENDED.into(),
                payload: serde_json::json!({ "call": shown, "outcome": outcome, "duration_secs": row.duration_secs() }),
            }),
        ])
    }

    /// Another word of a closed record: a device that never rang wrote
    /// "missed" before the device that took the call said how it ended.
    pub async fn correct(&self, call_id: &str, outcome: Outcome) -> Result<Vec<Effect>> {
        let Some(row) = repo::get(&self.store, call_id).await? else { return Ok(vec![]) };
        let ended_at = row.ended_at.unwrap_or(row.started_at);
        repo::set_outcome(&self.store, call_id, outcome.as_str(), ended_at).await?;
        let Some(row) = repo::get(&self.store, call_id).await? else { return Ok(vec![]) };
        let id = system_id(call_id);
        msgs::set_media_json(&self.store, &id, &details(&row).to_string()).await?;
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": row.chat_id, "message_id": id }),
        })])
    }

    /// A call that never was (the losing half of a glare): no record, no
    /// line. The screen learns of it from the call that won.
    pub async fn forget(&self, call_id: &str) -> Result<Vec<Effect>> {
        let Some(row) = repo::get(&self.store, call_id).await? else { return Ok(vec![]) };
        repo::delete(&self.store, call_id).await?;
        let id = system_id(call_id);
        msgs::delete(&self.store, &id).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": row.chat_id, "message_id": id }),
        })])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_contacts::{ContactService, ProfileService};
    use messenger_core::traits::SystemClock;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_call_from_before_the_chat_was_deleted_does_not_bring_it_back() {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts, profiles, Arc::new(SystemClock));
        let feed = Feed::new(store.clone(), dm.clone());
        let peer = "b".repeat(64);
        let chat_id = chats::dm_chat_id(&peer);

        assert_eq!(feed.begin("c1", &peer, Direction::In, Media::Audio, 100).await.unwrap().len(), 1);
        dm.delete_chat(&chat_id).await.unwrap();
        // A copy of the call on record, then one this device never saw.
        assert!(feed.begin("c1", &peer, Direction::In, Media::Audio, 100).await.unwrap().is_empty());
        assert!(feed.begin("c2", &peer, Direction::Out, Media::Audio, 200).await.unwrap().is_empty());
        assert!(chats::get(&store, &chat_id).await.unwrap().is_none());

        // A call after the deletion is a line, and the chat is there for it.
        let later = chats::cleared_at(&store, &chat_id).await.unwrap() + 10;
        assert_eq!(feed.begin("c3", &peer, Direction::In, Media::Video, later).await.unwrap().len(), 1);
        assert!(chats::get(&store, &chat_id).await.unwrap().is_some());
    }
}

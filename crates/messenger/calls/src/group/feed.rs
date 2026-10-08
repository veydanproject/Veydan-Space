// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The record of a group call (`msg_calls`, kind `group`, migration 021)
//! and its line in the group's chat: a system row `sys:call:<call_id>`,
//! as a call between two has. Made when the start is learned of, its
//! details written again as people come and the call ends; the host
//! words it from the details:
//!
//! ```json
//! { "kind": "group", "call_id": "…", "direction": "in"|"out", "media": "audio"|"video",
//!   "started_by": "<hex>", "participants": n, "outcome": null|"ended"|"failed",
//!   "duration_secs": null|n, "started_at": n }
//! ```
//!
//! `direction` is `out` when I started it; `duration_secs` is from the
//! start of the room to the end of the call, for everybody alike.

use crate::call::Outcome;
use crate::engine::Media;
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, Result};
use messenger_dm::{DmService, UI_EVENT_DM_MESSAGE, UI_EVENT_DM_UPDATED};
use messenger_store::calls::{self as repo, CallRow, GroupCallInfo, NewCall};
use messenger_store::messages::{self as msgs, NewMessage};
use messenger_store::{chats, settings, Store};
use std::collections::BTreeSet;

const PREVIEW: &str = "📞";
pub const SYSTEM_TEXT: &str = "call";
/// The seats this device took in the room of the call, under
/// `<prefix><call_id>` in the settings: a space-separated list. Kept from
/// the first join to the end of the call on record, so that after a
/// restart my own `call.join` of before is told from a word of another
/// device of mine.
const MY_SEATS_PREFIX: &str = "group_call.my_seats.";

fn my_seats_key(call_id: &str) -> String {
    format!("{MY_SEATS_PREFIX}{call_id}")
}

pub fn system_id(call_id: &str) -> String {
    crate::feed::system_id(call_id)
}

/// The details of the system row, from the record.
pub fn details(row: &CallRow, info: &GroupCallInfo) -> serde_json::Value {
    let duration = match (row.outcome.as_ref(), row.ended_at) {
        (Some(_), Some(e)) if e >= row.started_at => Some(e - row.started_at),
        _ => None,
    };
    serde_json::json!({
        "kind": repo::KIND_GROUP,
        "call_id": row.call_id,
        "direction": row.direction,
        "media": row.media,
        "started_by": info.started_by,
        "participants": info.participants,
        "outcome": row.outcome,
        "duration_secs": duration,
        "started_at": row.started_at,
    })
}

#[derive(Clone)]
pub(crate) struct GroupFeed {
    store: Store,
    dm: DmService,
}

impl GroupFeed {
    pub fn new(store: Store, dm: DmService) -> Self {
        Self { store, dm }
    }

    async fn rewrite(&self, call_id: &str) -> Result<Option<CallRow>> {
        let (Some(row), Some(info)) = (repo::get(&self.store, call_id).await?, repo::group_info(&self.store, call_id).await?) else {
            return Ok(None);
        };
        msgs::set_media_json(&self.store, &system_id(call_id), &details(&row, &info).to_string()).await?;
        Ok(Some(row))
    }

    fn updated(chat_id: &str, call_id: &str) -> Effect {
        Effect::Emit(UiEvent {
            name: UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message_id": system_id(call_id) }),
        })
    }

    /// A call of the group begins, or I learn of one: the record and the
    /// line, once. Nothing when it is on record already.
    pub async fn begin(&self, call_id: &str, group_id: &str, started_by: &str, mine: bool, media: Media, started_at: i64) -> Result<Vec<Effect>> {
        let chat_id = messenger_store::groups::group_chat_id(group_id);
        let new = NewCall {
            call_id: call_id.to_string(),
            chat_id: chat_id.clone(),
            peer: group_id.to_string(),
            direction: if mine { repo::DIR_OUT } else { repo::DIR_IN }.to_string(),
            media: media.as_str().to_string(),
            started_at,
        };
        if !repo::insert_group(&self.store, &new, started_by).await? {
            return Ok(vec![]);
        }
        let (Some(row), Some(info)) = (repo::get(&self.store, call_id).await?, repo::group_info(&self.store, call_id).await?) else {
            return Ok(vec![]);
        };
        let id = system_id(call_id);
        let inserted = msgs::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: None,
                direction: new.direction.clone(),
                status: msgs::STATUS_SENT.into(),
                content_type: msgs::CT_SYSTEM.into(),
                text: Some(SYSTEM_TEXT.into()),
                envelope_json: "{}".into(),
                sender_pubkey: started_by.to_string(),
                reply_to_id: None,
                target_id: None,
                created_at: started_at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: Some(details(&row, &info).to_string()),
            },
        )
        .await?;
        if !inserted {
            return Ok(vec![]);
        }
        chats::touch(&self.store, &chat_id, started_at, Some(PREVIEW), false).await?;
        let Some(view) = self.dm.message(&id).await? else { return Ok(vec![]) };
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": view, "historical": true }),
        })])
    }

    /// I got into the room at `at` (the first time counts).
    pub async fn joined(&self, call_id: &str, at: i64) -> Result<()> {
        repo::set_answered(&self.store, call_id, at).await
    }

    /// This device took `seat` in the room of the call: remembered until
    /// the call is over on record.
    pub async fn took_seat(&self, call_id: &str, seat: u32) -> Result<()> {
        let mut seats = self.my_seats(call_id).await?;
        seats.insert(seat);
        let list: Vec<String> = seats.iter().map(|s| s.to_string()).collect();
        settings::set(&self.store, &my_seats_key(call_id), &list.join(" ")).await
    }

    /// The seats this device took in the room of the call, so far.
    pub async fn my_seats(&self, call_id: &str) -> Result<BTreeSet<u32>> {
        Ok(settings::get(&self.store, &my_seats_key(call_id))
            .await?
            .map(|s| s.split_whitespace().filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default())
    }

    /// So many people were seen in the room, me included.
    pub async fn participants(&self, call_id: &str, n: usize) -> Result<Vec<Effect>> {
        repo::set_participants(&self.store, call_id, n as i64).await?;
        Ok(match self.rewrite(call_id).await? {
            Some(row) => vec![Self::updated(&row.chat_id, call_id)],
            None => vec![],
        })
    }

    /// The call is over for everybody. The first end holds.
    pub async fn finish(&self, call_id: &str, outcome: Outcome, ended_at: i64) -> Result<Vec<Effect>> {
        settings::delete(&self.store, &my_seats_key(call_id)).await?;
        if !repo::finish(&self.store, call_id, outcome.as_str(), ended_at).await? {
            return Ok(vec![]);
        }
        let Some(row) = self.rewrite(call_id).await? else { return Ok(vec![]) };
        chats::touch(&self.store, &row.chat_id, row.started_at, Some(PREVIEW), false).await?;
        Ok(vec![Self::updated(&row.chat_id, call_id)])
    }

    pub async fn row(&self, call_id: &str) -> Result<Option<CallRow>> {
        repo::get(&self.store, call_id).await
    }

    /// A call that never was for the group (the losing half of a glare:
    /// two members started one within moments, the group keeps the other):
    /// no record, no line.
    pub async fn forget(&self, call_id: &str) -> Result<Vec<Effect>> {
        settings::delete(&self.store, &my_seats_key(call_id)).await?;
        let Some(row) = repo::get(&self.store, call_id).await? else { return Ok(vec![]) };
        repo::delete(&self.store, call_id).await?;
        let id = system_id(call_id);
        msgs::delete(&self.store, &id).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        Ok(vec![Self::updated(&row.chat_id, call_id)])
    }

    /// The group calls on record without an end although their rooms have
    /// expired by now (`call.end` never came, or came while this device
    /// was off): closed as ended when the room would have.
    pub async fn close_stale(&self, now: i64, lifetime_secs: i64) -> Result<Vec<Effect>> {
        let mut out = vec![];
        for row in repo::stale_group_calls(&self.store, now - lifetime_secs).await? {
            out.extend(self.finish(&row.call_id, Outcome::Ended, row.started_at + lifetime_secs).await?);
        }
        Ok(out)
    }
}

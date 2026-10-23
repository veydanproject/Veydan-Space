// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The sign of life of a keeper (internal/communities-wire.md §18,
//! 2026-10-23): a parameterized replaceable event, kind 30391, `d` = the
//! channel, signed by the key of the keeper of the current series, at most
//! every `INTERVAL_SECS` while the keeper runs. «Хранитель был в сети» is
//! the newest of it and of the epochs: a quiet channel makes no epoch for
//! hours, and a live keeper looked away (живой запуск 2026-10-23: «был в
//! сети: 16:20» while it was online), and a joiner who waits for a key had
//! no epoch at all to tell by.

use crate::state::ChannelState;
use crate::wire::{KIND_HEARTBEAT, SKEW_SECS};
use messenger_core::{MessengerError, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};

pub const HEARTBEAT_VERSION: u32 = 1;
/// A keeper says it runs no more often than this; «away» is a sign older
/// than this (the tickets of the moderators, wire §17).
pub const INTERVAL_SECS: i64 = 300;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel heartbeat: {what}"))
}

/// The content of the event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub v: u32,
    pub community_id: String,
    /// When it was said: the event's `created_at` too.
    pub at: i64,
    /// The series of the keeper that says it (wire §3, `keeper.serial`).
    pub serial: u32,
}

/// The event of a keeper's sign of life.
pub fn sign_heartbeat(keeper: &Keys, hb: &Heartbeat) -> Result<Event> {
    EventBuilder::new(Kind::from(KIND_HEARTBEAT), serde_json::to_string(hb)?)
        .tag(Tag::parse(["d", hb.community_id.as_str()]).map_err(|e| invalid(&e.to_string()))?)
        .custom_created_at(Timestamp::from_secs(hb.at.max(0) as u64))
        .finalize(keeper)
        .map_err(|e| MessengerError::Crypto(e.to_string()))
}

/// Reads a sign of life: signed by the key of the keeper of the current
/// series by the journal and naming that series, of this channel (`d` and
/// the content), not dated past `now` and its skew, newer than the one
/// held (`known_at`).
pub fn verify_heartbeat(signed: &serde_json::Value, state: &ChannelState, known_at: Option<i64>, now: i64) -> Result<Heartbeat> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("event"))?;
    event.verify().map_err(|_| invalid("signature"))?;
    if event.kind.as_u16() != KIND_HEARTBEAT {
        return Err(invalid("kind"));
    }
    let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?;
    if event.pubkey.to_hex() != keeper.pubkey.as_hex() {
        return Err(invalid("not the current keeper"));
    }
    let d = event.tags.iter().find(|t| t.kind() == "d").and_then(|t| t.as_slice().get(1).cloned());
    if d.as_deref() != Some(state.community_id.as_str()) {
        return Err(invalid("d"));
    }
    let hb: Heartbeat = serde_json::from_str(&event.content).map_err(|_| invalid("content"))?;
    if hb.community_id != state.community_id {
        return Err(invalid("community_id"));
    }
    if hb.serial != keeper.serial {
        return Err(invalid("not the current series"));
    }
    if hb.at > now + SKEW_SECS {
        return Err(invalid("dated ahead"));
    }
    if known_at.is_some_and(|k| hb.at <= k) {
        return Err(invalid("not newer"));
    }
    Ok(hb)
}

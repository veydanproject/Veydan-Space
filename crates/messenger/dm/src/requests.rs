// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The queue of requests: the chats of strangers who wrote to me and wait
//! for an answer. A request is a chat in the mode `request_received`, no
//! flag says it twice (`dm_relations::REQUEST_SQL`); the queue has a term
//! and a size, and what runs out of either is gone as if I had deleted the
//! chat, with the key shut (`MyContact::Expired`) until I add it myself.
//! The settings of whom I take messages from live here too.

use crate::peer::DmPeer;
use crate::relations::UI_EVENT_DM_RELATIONSHIP;
use crate::relationship::{decided_now, screen_mode, DmFrom, MyContact, ScreenMode};
use crate::service::DmService;
use crate::snapshot::KEY_PRIVACY_DECIDED_AT;
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, MessengerError, PubKey, Result};
use messenger_store::messages as repo;
use messenger_store::{dm_held, dm_relations, settings};
use nostr::nips::nip19::ToBech32;
use nostr::prelude::PublicKey;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A request came, ran out or was decided: `{"peer", "pending"}`, where
/// `pending` is how many wait now. A signal only: the UI reads the queue
/// by `requests` after it, and after a start or a reconnect.
pub const UI_EVENT_DM_REQUEST: &str = "dm.request";

/// Whose messages I take: `anyone` (a stranger lands in the queue) or
/// `contacts` (a stranger is dropped without a trace).
pub const KEY_DM_FROM: &str = "privacy.dm_from";
/// Days a request waits before it runs out; `0` never.
pub const KEY_KEEP_DAYS: &str = "requests.keep_days";
pub const DEFAULT_KEEP_DAYS: u32 = 7;

/// The queue holds this many chats at most; a new one past it pushes the
/// one with the oldest activity out.
pub const MAX_REQUESTS: i64 = 200;

/// One chat of the queue, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RequestView {
    /// `dm:<hex>`, or `dm:<hex>@<server id>` for a request in a private
    /// chat: the address an answer to it takes
    /// (`messenger_moderation_request_act`), so it lands in that chat.
    pub chat_id: String,
    /// The private server of the request's chat; absent for the home chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server_id: Option<String>,
    /// Hex pubkey of the stranger.
    pub peer: String,
    /// What the app calls them: a profile name, else a short npub.
    pub name: String,
    /// The one visible text of the request (plain; empty when none shows).
    pub text: String,
    /// Rumor time of that text (the peer's clock).
    #[ts(type = "number")]
    pub at: i64,
    /// Held rows waiting behind it.
    pub held: u32,
    /// Where the queue places it: what `before` takes to page past it. Not
    /// `at`: the queue is ordered by the activity of the chat, bounded by
    /// my clock (`dm_relations::request_rows`), and the two part when the
    /// text is dated ahead or is gone.
    #[ts(type = "number")]
    pub cursor: i64,
}

/// The settings of the queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DmSettings {
    /// `anyone` | `contacts`
    pub dm_from: String,
    /// `0` = forever.
    pub keep_days: u32,
}

impl DmService {
    /// Whose messages I take.
    pub(crate) async fn dm_from(&self) -> Result<DmFrom> {
        Ok(settings::get(&self.store, KEY_DM_FROM).await?.as_deref().and_then(DmFrom::parse).unwrap_or_default())
    }

    async fn keep_days(&self) -> Result<u32> {
        Ok(settings::get(&self.store, KEY_KEEP_DAYS).await?.and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_KEEP_DAYS))
    }

    pub async fn dm_settings(&self) -> Result<DmSettings> {
        Ok(DmSettings { dm_from: self.dm_from().await?.as_str().into(), keep_days: self.keep_days().await? })
    }

    /// Set here, now: dated, so my other devices take them over older
    /// ones of theirs (`crate::snapshot`).
    pub async fn dm_settings_set(&self, s: DmSettings) -> Result<()> {
        let Some(from) = DmFrom::parse(&s.dm_from) else {
            return Err(MessengerError::Invalid("dm_from".into()));
        };
        settings::set(&self.store, KEY_DM_FROM, from.as_str()).await?;
        settings::set(&self.store, KEY_KEEP_DAYS, &s.keep_days.to_string()).await?;
        let prev = settings::get(&self.store, KEY_PRIVACY_DECIDED_AT).await?.and_then(|v| v.parse().ok()).unwrap_or(0);
        settings::set(&self.store, KEY_PRIVACY_DECIDED_AT, &decided_now(prev, self.clock.now().secs()).to_string()).await?;
        self.mark_relations_owed().await
    }

    pub async fn requests_count(&self) -> Result<u32> {
        Ok(dm_relations::count_requests(&self.store).await?.max(0) as u32)
    }

    /// The event that tells the UI to count the queue again.
    pub async fn request_event(&self, peer: &PubKey) -> Result<UiEvent> {
        Ok(UiEvent {
            name: UI_EVENT_DM_REQUEST.into(),
            payload: serde_json::json!({ "peer": peer.as_hex(), "pending": self.requests_count().await? }),
        })
    }

    /// The queue, newest first; `before` is the `cursor` of the last one
    /// shown.
    pub async fn requests(&self, before: Option<i64>, limit: u32) -> Result<Vec<RequestView>> {
        let rows = dm_relations::request_rows(&self.store, before, i64::from(limit.clamp(1, 500))).await?;
        let mut out = Vec::with_capacity(rows.len());
        for (chat_id, peer_hex, cursor) in rows {
            let Some(chat) = DmPeer::of_chat(&chat_id) else { continue };
            let peer = chat.key.clone();
            let floor = self.load_relation(&chat).await?.request_floor;
            let (text, at) = match repo::first_visible_incoming_since(&self.store, &chat_id, floor).await? {
                Some(m) if m.deleted_at.is_none() => (m.text.unwrap_or_default(), m.created_at),
                Some(m) => (String::new(), m.created_at),
                None => (String::new(), 0),
            };
            out.push(RequestView {
                chat_id: chat_id.clone(),
                server_id: chat.server().map(String::from),
                peer: peer_hex,
                name: self.display_name(&peer).await?,
                text,
                at,
                held: dm_held::count(&self.store, &chat_id).await?.max(0) as u32,
                cursor,
            });
        }
        Ok(out)
    }

    /// What the app calls `peer`: nickname, profile name, short npub.
    pub async fn display_name(&self, peer: &PubKey) -> Result<String> {
        let name = match self.contacts.get(peer).await? {
            Some(c) => c.label(),
            None => self.profiles.get(peer).await?.map(|p| p.label()).unwrap_or_default(),
        };
        if !name.trim().is_empty() {
            return Ok(name);
        }
        let n = PublicKey::from_hex(peer.as_hex()).ok().and_then(|p| p.to_bech32().ok()).unwrap_or_else(|| peer.as_hex().to_string());
        Ok(format!("{}…{}", &n[..12.min(n.len())], &n[n.len().saturating_sub(4)..]))
    }

    /// The queue is kept: a request whose activity is older than the term
    /// runs out, and past `MAX_REQUESTS` the ones with the oldest activity
    /// are pushed out. At the start of a session and once an hour. The
    /// activity is the time of the request's newest message, bounded by
    /// when this device wrote its chat (`dm_relations::request_rows`): the
    /// peer's clock alone decides neither who runs out nor who is the oldest.
    pub async fn sweep_requests(&self, now: i64) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        let keep_days = self.keep_days().await?;
        if keep_days > 0 {
            let cutoff = now.saturating_sub(i64::from(keep_days).saturating_mul(86_400));
            let total = dm_relations::count_requests(&self.store).await?;
            for (chat_id, peer, last_at) in dm_relations::oldest_requests(&self.store, total).await? {
                if last_at <= cutoff {
                    effects.extend(self.expire_request(&chat_id, &peer, now).await?);
                }
            }
        }
        effects.extend(self.cap_requests(now).await?);
        Ok(effects)
    }

    /// Past the size of the queue, the oldest go: called when a request
    /// lands, so the queue never grows beyond it.
    pub(crate) async fn cap_requests(&self, now: i64) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        let over = dm_relations::count_requests(&self.store).await? - MAX_REQUESTS;
        if over > 0 {
            for (chat_id, peer, _) in dm_relations::oldest_requests(&self.store, over).await? {
                effects.extend(self.expire_request(&chat_id, &peer, now).await?);
            }
        }
        Ok(effects)
    }

    /// The request of `peer` is over: the chat goes as if I had deleted it
    /// (its tombstone keeps the texts out, what was held is purged), and
    /// the key is shut until I add it myself. The peer is told nothing: an
    /// expiry is not an answer. Decided here, now.
    async fn expire_request(&self, chat_id: &str, peer_hex: &str, now: i64) -> Result<Vec<Effect>> {
        // The request of the chat, the private one by its own relation.
        let Some(peer) = DmPeer::of_chat(chat_id).filter(|p| p.key.as_hex() == peer_hex) else { return Ok(vec![]) };
        let current = self.load_relation(&peer).await?;
        if screen_mode(&current) != ScreenMode::RequestReceived {
            return Ok(vec![]);
        }
        self.delete_chat(chat_id).await?;
        let mut next = current;
        next.my_contact = MyContact::Expired;
        next.decided_at = decided_now(current.decided_at, now);
        self.save_relation(&peer, &next).await?;
        self.mark_relations_owed().await?;
        let (mode, can_send) = self.mode_of(chat_id, &peer).await?;
        Ok(vec![
            Effect::Emit(UiEvent {
                name: UI_EVENT_DM_RELATIONSHIP.into(),
                payload: serde_json::json!({
                    "chat_id": chat_id, "peer": peer.key.as_hex(), "mode": mode.as_str(), "can_send": can_send,
                }),
            }),
            Effect::Emit(self.request_event(&peer.key).await?),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relationship::{MyContact as M, PeerSignal as P, Relationship};
    use messenger_store::dm_relations::RelationRow;
    use messenger_store::{chats, Store};

    /// The one predicate of "request" in SQL says what `screen_mode` says,
    /// over every combination of the stored facts.
    #[tokio::test]
    async fn request_sql_agrees_with_screen_mode_everywhere() {
        let s = Store::open_in_memory().await.unwrap();
        let mut want = Vec::new();
        let mut i = 0;
        for m in [M::None, M::Approved, M::Declined, M::Expired] {
            for blocked in [false, true] {
                for p in [P::None, P::Approved, P::Blocked, P::Left, P::Declined, P::Revoked] {
                    for mutual in [false, true] {
                        i += 1;
                        let peer = format!("{i:064x}");
                        dm_relations::put(
                            &s,
                            &RelationRow {
                                peer_pubkey: peer.clone(),
                                server_id: String::new(),
                                my_contact: m.as_str().into(),
                                blocked,
                                peer_signal: p.as_str().into(),
                                was_ever_mutual: mutual,
                                last_signal_at: 0,
                                last_my_signal_at: 0,
                                request_floor: 0,
                                decided_at: 0,
                                created_at: 0,
                                updated_at: 0,
                            },
                        )
                        .await
                        .unwrap();
                        chats::ensure_dm(&s, &peer, None).await.unwrap();
                        let r = Relationship { my_contact: m, blocked, peer_signal: p, was_ever_mutual: mutual, ..Default::default() };
                        if screen_mode(&r) == ScreenMode::RequestReceived {
                            want.push(peer);
                        }
                    }
                }
            }
        }
        let mut got: Vec<String> = dm_relations::request_rows(&s, None, 1000).await.unwrap().into_iter().map(|(_, p, _)| p).collect();
        got.sort();
        want.sort();
        assert_eq!(got, want);
        assert_eq!(dm_relations::count_requests(&s).await.unwrap(), want.len() as i64);
        assert_eq!(want.len(), 4, "none or declined by me, approved by them, not blocked, mutual before or not");
    }
}

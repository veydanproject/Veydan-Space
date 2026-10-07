// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `Inbound::Meta` consumer: profiles into the cache, our own follow list
//! into `msg_follows`. Relay lists are left to the DM router (stage 5).

use crate::book::ContactService;
use crate::profile::ProfileService;
use async_trait::async_trait;
use messenger_core::traits::UiEvent;
use messenger_core::{Context, Effect, Handler, MetaInbound, Result};

pub const UI_EVENT_PROFILE_UPDATED: &str = "profile.updated";
pub const UI_EVENT_FOLLOWS_UPDATED: &str = "follows.updated";

pub struct MetaHandler {
    profiles: ProfileService,
    contacts: ContactService,
}

impl MetaHandler {
    pub fn new(profiles: ProfileService, contacts: ContactService) -> Self {
        Self { profiles, contacts }
    }
}

#[async_trait]
impl Handler<MetaInbound> for MetaHandler {
    async fn handle(&self, msg: MetaInbound, ctx: &Context) -> Result<Vec<Effect>> {
        match msg {
            MetaInbound::Profile { author, created_at, content } => {
                let changed = self.profiles.apply_event(&author, created_at, &content).await?;
                if author == ctx.my_pubkey {
                    self.profiles.note_own_heard();
                }
                if changed {
                    return Ok(vec![Effect::Emit(UiEvent {
                        name: UI_EVENT_PROFILE_UPDATED.into(),
                        payload: serde_json::json!({ "pubkey": author.as_hex() }),
                    })]);
                }
                Ok(vec![])
            }
            MetaInbound::Follows { author, follows, .. } => {
                // Only our own kind 3 is authoritative for msg_follows; other
                // people's follow lists are social-graph data for later.
                if author != ctx.my_pubkey {
                    return Ok(vec![]);
                }
                self.contacts.apply_follow_list(&follows).await?;
                Ok(vec![Effect::Emit(UiEvent {
                    name: UI_EVENT_FOLLOWS_UPDATED.into(),
                    payload: serde_json::json!({ "count": follows.len() }),
                })])
            }
            MetaInbound::RelayList { .. } | MetaInbound::DmRelays { .. } | MetaInbound::Presence { .. } => Ok(vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::traits::SystemClock;
    use messenger_core::{PubKey, Timestamp};
    use messenger_store::Store;
    use nostr::key::Keys;
    use std::sync::Arc;

    fn pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).unwrap()
    }

    #[tokio::test]
    async fn profile_and_own_follows_are_applied() {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store, profiles.clone());
        let h = MetaHandler::new(profiles.clone(), contacts.clone());
        let me = Keys::generate();
        let other = Keys::generate();
        let ctx = Context { my_pubkey: pk(&me), session_started_at: Timestamp(0), clock: Arc::new(SystemClock) };

        let fx = h
            .handle(MetaInbound::Profile { author: pk(&other), created_at: Timestamp(5), content: r#"{"name":"o"}"#.into() }, &ctx)
            .await
            .unwrap();
        assert_eq!(fx.len(), 1);
        assert_eq!(profiles.get(&pk(&other)).await.unwrap().unwrap().name.as_deref(), Some("o"));
        let fx = h
            .handle(MetaInbound::Profile { author: pk(&other), created_at: Timestamp(4), content: r#"{"name":"stale"}"#.into() }, &ctx)
            .await
            .unwrap();
        assert!(fx.is_empty(), "older profile emits nothing");
        assert_eq!(profiles.own_heard(), 0, "not mine");

        // Mine, whether newer than the cache or not: heard.
        let mine = |at| MetaInbound::Profile { author: pk(&me), created_at: Timestamp(at), content: r#"{"name":"me"}"#.into() };
        assert_eq!(h.handle(mine(5), &ctx).await.unwrap().len(), 1);
        assert_eq!(profiles.own_heard(), 1);
        assert!(h.handle(mine(5), &ctx).await.unwrap().is_empty());
        assert_eq!(profiles.own_heard(), 2, "the same again still says the relay has nothing newer");

        let fx = h
            .handle(MetaInbound::Follows { author: pk(&other), created_at: Timestamp(1), follows: vec![pk(&me)] }, &ctx)
            .await
            .unwrap();
        assert!(fx.is_empty());
        assert!(contacts.follows().await.unwrap().is_empty(), "someone else's follow list is ignored");

        let fx = h
            .handle(MetaInbound::Follows { author: pk(&me), created_at: Timestamp(1), follows: vec![pk(&other)] }, &ctx)
            .await
            .unwrap();
        assert_eq!(fx.len(), 1);
        assert_eq!(contacts.follows().await.unwrap(), vec![pk(&other)]);
    }
}

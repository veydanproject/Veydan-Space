// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! How wide I made the panes of the app, as the host asks for them. A
//! change is kept at once and goes to my other devices (`own.ui`) with the
//! next tick of the session's receipt loop, every two seconds, so a run of
//! changes costs one note; the map goes again every few days, and at the
//! close of a session if it is owed. A map from another device comes as
//! `ui_prefs.updated {w}`.

use crate::MessengerRuntime;
use messenger_core::traits::SystemClock;
use messenger_core::{Clock, Outbound, Result};
use messenger_dm::wrap::wrap_own;
use messenger_dm::DmService;
use messenger_ingress::Outbox;
use nostr::key::Keys;
use std::collections::BTreeMap;

/// Queue `own.ui` for my other devices when it is owed (see
/// `DmService::ui_prefs_due`). `true` when one was queued.
pub(crate) async fn send_ui_prefs_if_due(dm: &DmService, outbox: &Outbox, keys: &Keys, now: i64) -> Result<bool> {
    let Some(note) = dm.ui_prefs_due(now).await? else { return Ok(false) };
    let event = wrap_own(keys, &note.encode(), now)?;
    outbox.enqueue(Outbound::PublishOwn { event }).await?;
    dm.ui_prefs_sent(now).await?;
    Ok(true)
}

impl MessengerRuntime {
    /// The widths of the panes: key → px.
    pub async fn ui_prefs_get(&self) -> Result<BTreeMap<String, u32>> {
        Ok(self.dm.ui_prefs().await?.widths)
    }

    /// Keep `px` as the width of the pane `key` (`[a-z0-9._-]`, 1–40;
    /// 200…2000 px); `None` forgets it. My other devices are told with the
    /// next tick. Returns the whole map. Error: `ui_pref_invalid`.
    pub async fn ui_prefs_set(&self, key: &str, px: Option<u32>) -> Result<BTreeMap<String, u32>> {
        Ok(self.dm.set_ui_pref(key, px).await?.widths)
    }

    /// At the close of a session: the map owed now, sent by the next
    /// session if not by this one.
    pub(crate) async fn send_ui_prefs_closing(&self, keys: &Keys) {
        if let Err(e) = send_ui_prefs_if_due(&self.dm, &self.outbox, keys, SystemClock.now().secs()).await {
            eprintln!("messenger ui prefs: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipts::tests::with_a_message_of;
    use messenger_core::envelope::{KIND_OWN_RUMOR, T_OWN_UI};
    use messenger_core::Envelope;
    use nostr::nips::nip59::UnwrappedGift;
    use nostr::prelude::Event;

    /// The maps of widths in the outbox, opened by me.
    async fn maps(rt: &MessengerRuntime, me: &Keys) -> Vec<Envelope> {
        let mut out = Vec::new();
        for r in messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap() {
            let Ok(Outbound::PublishOwn { event }) = r.outbound() else { continue };
            let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
            let Ok(opened) = UnwrappedGift::from_gift_wrap(me, &ev) else { continue };
            assert_eq!(opened.rumor.kind.as_u16(), KIND_OWN_RUMOR);
            assert!(ev.tags.iter().any(|t| t.as_slice() == ["silent", "1"]), "a note of mine wakes nobody");
            let note = Envelope::parse(&opened.rumor.content).unwrap();
            if note.t == T_OWN_UI {
                out.push(note);
            }
        }
        out
    }

    #[tokio::test]
    async fn changes_between_two_ticks_leave_as_one_note() {
        let peer = Keys::generate();
        let (_dir, rt, now) = with_a_message_of(&peer).await;
        let me = rt.session_keys().await.unwrap();
        assert!(rt.ui_prefs_get().await.unwrap().is_empty());
        assert!(rt.ui_prefs_set("chats.side", Some(100)).await.is_err());

        rt.ui_prefs_set("channels.side", Some(400)).await.unwrap();
        rt.ui_prefs_set("channels.side", Some(410)).await.unwrap();
        let map = rt.ui_prefs_set("channels.side", Some(420)).await.unwrap();
        assert_eq!(map, BTreeMap::from([("channels.side".to_string(), 420)]));
        assert!(maps(&rt, &me).await.is_empty(), "nothing before the tick");

        crate::receipts::flush(rt.dm(), rt.groups(), &rt.outbox, &me).await.unwrap();
        let sent = maps(&rt, &me).await;
        assert_eq!(sent.len(), 1, "one note for three changes");
        assert_eq!(sent[0].fields.get("w"), Some(&serde_json::json!({ "channels.side": 420 })));
        assert!(sent[0].fields.get("at").and_then(|v| v.as_i64()).is_some_and(|at| at >= now));

        crate::receipts::flush(rt.dm(), rt.groups(), &rt.outbox, &me).await.unwrap();
        assert_eq!(maps(&rt, &me).await.len(), 1, "nothing new, nothing sent");
        assert_eq!(rt.ui_prefs_get().await.unwrap(), map, "kept");
    }
}

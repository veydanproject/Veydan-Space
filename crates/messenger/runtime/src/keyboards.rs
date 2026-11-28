// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The buttons of a bot under a message (internal/platform-spec.md §9.7),
//! as the host asks for them: a press of mine leaves through the outbox
//! here. What a bot sends with buttons is `dm_send_text_with` and
//! `dm_edit_with`; what a bot hears of a press is the event
//! `messenger_dm::UI_EVENT_DM_PRESS` or `messenger_groups::UI_EVENT_GROUP_PRESS`.

use crate::MessengerRuntime;
use messenger_core::traits::UiEvent;
use messenger_core::{MessengerError, Result};
use messenger_dm::{MessageView, PRESS_TTL_SECS, UI_EVENT_DM_UPDATED};

impl MessengerRuntime {
    /// Press the button with data `d` under the message `message_id` of a
    /// bot, in a direct chat or a group. Nothing happens in the app but
    /// the press to the author and the mark on the message (`pressed`).
    /// Refusals: `press_unknown` (no such incoming text, deleted, hidden),
    /// `press_no_button` (no button with that data now), `press_not_bot`
    /// (the author's profile does not say `bot`), `press_pending` (pressed
    /// less than 30 s ago and no edit since), and those of the chat
    /// (`dm_blocked`, `waiting_approval`, `group_muted`, …).
    pub async fn dm_press(&self, message_id: &str, d: &str) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let chat_id = if let Some(group_id) = self.group_of_message(message_id).await? {
            let (chat_id, out) = {
                let lock = self.groups().lock_of(&group_id).await;
                let _guard = lock.lock().await;
                self.group_driver.renew_stale_key(&keys, &group_id).await?;
                self.groups().prepare_press(&keys, message_id, d).await?
            };
            self.outbox.enqueue_for(out, PRESS_TTL_SECS).await?;
            chat_id
        } else {
            let p = self.dm.prepare_press(&keys, message_id, d).await?;
            self.outbox.enqueue_for(p.to_peer, PRESS_TTL_SECS).await?;
            p.chat_id
        };
        self.outbox.kick();
        let _ = self.ui.send(UiEvent {
            name: UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
        });
        self.dm
            .message(message_id)
            .await?
            .ok_or_else(|| MessengerError::Storage("message vanished".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::keyboard::{Button, ButtonStyle, Keyboard};
    use messenger_core::traits::RelayState;
    use messenger_core::{Clock, MessengerConfig, PubKey, TextFormat, Timestamp, Transport};
    use messenger_dm::{Action, UI_EVENT_DM_PRESS};
    use messenger_testkit::MemorySecretStore;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;

    /// A runtime with a session on the relay `url`, and its events.
    async fn device(
        url: &str,
    ) -> (
        tempfile::TempDir,
        MessengerRuntime,
        broadcast::Receiver<UiEvent>,
        PubKey,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let rt = MessengerRuntime::start(
            MessengerConfig::new(dir.path().join("messenger")),
            Arc::new(MemorySecretStore::unlocked()),
        )
        .await
        .unwrap();
        let ui = rt.ui_events();
        rt.relays().add_user(url, None).await.unwrap();
        rt.servers_use_own().await.unwrap();
        crate::testing::log_in(&rt).await;
        assert!(rt.refresh_signer().await.unwrap());
        for _ in 0..300 {
            if rt
                .relays()
                .pool()
                .await
                .status()
                .await
                .relays
                .iter()
                .all(|r| r.state == RelayState::Connected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let me = rt.session_pubkey().await.unwrap();
        (dir, rt, ui, me)
    }

    /// The message of `chat` whose text is `text`, once it is there (30 s at most).
    async fn message_when(rt: &MessengerRuntime, chat: &str, text: &str) -> MessageView {
        for _ in 0..300 {
            if let Some(m) = rt
                .dm()
                .messages(chat, None, 100)
                .await
                .unwrap()
                .into_iter()
                .find(|m| m.text.as_deref() == Some(text))
            {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no {text:?} in {chat}");
    }

    async fn mode_when(rt: &MessengerRuntime, peer: &PubKey, want: &str) {
        for _ in 0..300 {
            if rt.dm_relation(peer.as_hex()).await.unwrap().mode == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the chat with {peer:?} is not {want}");
    }

    async fn event_named(ui: &mut broadcast::Receiver<UiEvent>, name: &str) -> UiEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match tokio::time::timeout_at(deadline, ui.recv()).await {
                Ok(Ok(ev)) if ev.name == name => return ev,
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(e)) => panic!("{e}"),
                Err(_) => panic!("no {name} within 30 s"),
            }
        }
    }

    /// The whole way on one relay: the bot sends a text with buttons, the
    /// person sees them once the bot's profile says `bot`, presses one, the
    /// bot hears the press, edits the message without buttons, and the
    /// person's message has no buttons and no press left.
    #[tokio::test]
    async fn a_bot_sends_buttons_a_person_presses_the_bot_hears_and_edits_them_away() {
        let relay = messenger_testkit::local_relay().await;
        let url = relay
            .url()
            .await
            .as_str_without_trailing_slash()
            .to_string();
        let (_b, bot, mut bot_ui, bot_pk) = device(&url).await;
        let (_p, person, _person_ui, person_pk) = device(&url).await;
        let (bot_chat, person_chat) = (
            format!("dm:{}", person_pk.as_hex()),
            format!("dm:{}", bot_pk.as_hex()),
        );

        // The person writes first; the bot takes the chat.
        person
            .dm_send_text(bot_pk.as_hex(), "/start", None)
            .await
            .unwrap();
        message_when(&bot, &bot_chat, "/start").await;
        bot.dm_act(person_pk.as_hex(), Action::Accept)
            .await
            .unwrap();
        mode_when(&person, &bot_pk, "full_chat").await;

        let kb = Keyboard {
            rows: vec![vec![
                Button::data("Allow", "a:78fe6469").styled(ButtonStyle::Primary),
                Button::data("Deny", "d:78fe6469").styled(ButtonStyle::Danger),
            ]],
        };
        let too_big = Keyboard {
            rows: vec![vec![Button::data("x", "1")]; 9],
        };
        assert!(
            matches!(bot.dm_send_text_with(person_pk.as_hex(), "x", TextFormat::Plain, None, Some(&too_big)).await, Err(MessengerError::Invalid(c)) if c == "kb_rows")
        );
        let sent = bot
            .dm_send_text_with(
                person_pk.as_hex(),
                "Open wttr.in?",
                TextFormat::Plain,
                None,
                Some(&kb),
            )
            .await
            .unwrap();
        assert_eq!(sent.kb, Some(kb.clone()));

        // No profile of the bot here yet: kept, not drawn.
        let seen = message_when(&person, &person_chat, "Open wttr.in?").await;
        assert_eq!(
            (seen.id.as_str(), seen.kb.as_ref()),
            (sent.id.as_str(), None)
        );
        assert!(
            matches!(person.dm_press(&sent.id, "a:78fe6469").await, Err(MessengerError::Invalid(c)) if c == "press_not_bot")
        );
        let now = messenger_core::traits::SystemClock.now().secs();
        person
            .profiles()
            .apply_event(&bot_pk, Timestamp(now), r#"{"name":"Bot","bot":true}"#)
            .await
            .unwrap();
        assert_eq!(
            person.dm().message(&sent.id).await.unwrap().unwrap().kb,
            Some(kb.clone())
        );

        let lines = bot.dm().messages(&bot_chat, None, 100).await.unwrap().len();
        let pressed = person.dm_press(&sent.id, "a:78fe6469").await.unwrap();
        assert_eq!(
            pressed.pressed.as_ref().map(|p| p.d.as_str()),
            Some("a:78fe6469")
        );
        assert!(
            matches!(person.dm_press(&sent.id, "d:78fe6469").await, Err(MessengerError::Invalid(c)) if c == "press_pending")
        );

        let heard = event_named(&mut bot_ui, UI_EVENT_DM_PRESS).await;
        assert_eq!(heard.payload["chat_id"], bot_chat.as_str());
        assert_eq!(heard.payload["id"], sent.id.as_str());
        assert_eq!(heard.payload["from"], person_pk.as_hex());
        assert_eq!(heard.payload["d"], "a:78fe6469");
        assert_eq!(heard.payload["historical"], false);
        assert_eq!(
            bot.dm().messages(&bot_chat, None, 100).await.unwrap().len(),
            lines,
            "the press is no message"
        );

        let edited = bot
            .dm_edit_with(&sent.id, "Allowed.", TextFormat::Plain, None)
            .await
            .unwrap();
        assert_eq!(edited.kb, None);
        let after = message_when(&person, &person_chat, "Allowed.").await;
        assert_eq!((after.kb, after.pressed), (None, None));
        assert!(
            matches!(person.dm_press(&sent.id, "a:78fe6469").await, Err(MessengerError::Invalid(c)) if c == "press_no_button")
        );
        bot.shutdown().await;
        person.shutdown().await;
    }
}

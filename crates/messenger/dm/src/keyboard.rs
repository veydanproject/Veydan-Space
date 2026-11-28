// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Buttons under a message of a bot (internal/platform-spec.md §9.7): what
//! a row keeps of them, what the host is shown, a press of mine and a
//! press heard. The wire is `messenger_core::keyboard`.
//!
//! A text keeps its keyboard in its `media_json` (`KbState`, written by
//! `messages::set_keyboard`), with the button I pressed and when: that is
//! local and never travels. An edit of the author replaces the keyboard,
//! or takes it away when it has none, and forgets the press.
//!
//! Who may show buttons: a message of mine (a bot sends them), or of a key
//! whose kind 0 says `bot: true`. A keyboard from a key whose profile says
//! it is a person is dropped where it comes, before anything keeps it. One
//! from a key whose profile is not here yet is kept, and shown only once
//! the profile says `bot` (`keyboard_view` reads the cache each time a
//! message is shown, so nothing has to be rewritten when it comes: the
//! host reads the chat again on `profile.updated`).
//!
//! A press is a note to the author (`KIND_PEER_NOTE_RUMOR` in a direct
//! chat, a quiet sealed note in a group): it is no message, never stored,
//! never shown, counted nowhere and never pushed. Heard only by the author
//! of the message, of a message of the chat it came in, for a button the
//! message has now; told once per (message, data, presser) within
//! `PRESS_DEDUP_SECS`.

use crate::peer::DmPeer;
use crate::relationship::{outbound_permission, DenyReason, OutboundPermission};
use crate::service::DmService;
use crate::view::MessageView;
use crate::wrap::wrap_note;
use messenger_core::keyboard::{press_of, Keyboard, PRESS_DEDUP_SECS, PRESS_REPEAT_SECS};
use messenger_core::traits::UiEvent;
use messenger_core::{Envelope, MessengerError, Outbound, PubKey, Result};
use messenger_store::messages::{self as repo, MessageRow};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};

/// A button under a message of mine was pressed in a direct chat:
/// `{"chat_id", "id": "<my message>", "from": "<hex of the presser>",
/// "d": "<data of the button>", "created_at": <secs>, "historical": bool}`
/// (`historical`: it came with history, not with the running session).
/// For the bot that wrote the message; the screen of a person never gets
/// one, since only the author of a message hears its presses.
pub const UI_EVENT_DM_PRESS: &str = "dm.press";

/// The presses heard lately: (message, data, presser) → when.
pub(crate) type PressesSeen = std::collections::HashMap<(String, String, String), i64>;

/// How long the outbox tries a press: a press is for now, and the app
/// lets the button be pressed again after `PRESS_REPEAT_SECS`.
pub const PRESS_TTL_SECS: i64 = 300;

/// The button I pressed and when (unix seconds), kept on this device
/// only. While `at` is younger than `PRESS_REPEAT_SECS` and no edit came,
/// the message is not pressed again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pressed {
    pub d: String,
    pub at: i64,
}

/// What the `media_json` of a text holds of its buttons.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KbState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb: Option<Keyboard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<Pressed>,
}

impl KbState {
    /// The state a row keeps; none when it keeps nothing readable.
    pub fn stored(json: Option<&str>) -> Self {
        json.and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default()
    }

    /// What a text with `kb` keeps: nothing without a keyboard.
    pub fn json_of(kb: Option<Keyboard>) -> Option<String> {
        kb.map(|kb| {
            serde_json::to_string(&KbState {
                kb: Some(kb),
                pressed: None,
            })
            .expect("a keyboard serializes")
        })
    }

    fn json(&self) -> String {
        serde_json::to_string(self).expect("a keyboard serializes")
    }
}

/// A press of mine in a direct chat, recorded here and ready to go.
#[derive(Clone, Debug)]
pub struct PreparedPress {
    pub chat_id: String,
    /// The message with `pressed` set.
    pub message: MessageView,
    /// The note for the author, and nobody else: no copy for my devices.
    pub to_peer: Outbound,
}

fn unknown() -> MessengerError {
    MessengerError::Invalid("press_unknown".into())
}

/// The event of a press heard: `UI_EVENT_DM_PRESS` or the group's.
pub fn press_event(
    name: &str,
    chat_id: &str,
    id: &str,
    from: &PubKey,
    d: &str,
    created_at: i64,
    historical: bool,
) -> UiEvent {
    UiEvent {
        name: name.into(),
        payload: serde_json::json!({
            "chat_id": chat_id,
            "id": id,
            "from": from.as_hex(),
            "d": d,
            "created_at": created_at,
            "historical": historical,
        }),
    }
}

impl DmService {
    /// The keyboard of a `text` or an `edit` that may be kept: mine, or of
    /// a key that is a bot or not known yet. A person's is dropped here.
    pub async fn kb_to_keep(
        &self,
        envelope: &Envelope,
        sender: &PubKey,
        from_me: bool,
    ) -> Result<Option<Keyboard>> {
        let Some(kb) = envelope.kb() else {
            return Ok(None);
        };
        if from_me {
            return Ok(Some(kb));
        }
        match self.profiles.get(sender).await? {
            Some(p) if !p.bot => Ok(None),
            _ => Ok(Some(kb)),
        }
    }

    /// `kb_to_keep` as the `media_json` of a new text row.
    pub async fn kb_json_to_keep(
        &self,
        envelope: &Envelope,
        sender: &PubKey,
        from_me: bool,
    ) -> Result<Option<String>> {
        Ok(KbState::json_of(
            self.kb_to_keep(envelope, sender, from_me).await?,
        ))
    }

    /// An edit of the author was applied to `id`: its keyboard (already
    /// judged by `kb_to_keep`) replaces the one there, none takes it away,
    /// and the press I made is forgotten either way.
    pub async fn edit_keyboard(&self, id: &str, kb: Option<Keyboard>) -> Result<()> {
        repo::set_keyboard(&self.store, id, KbState::json_of(kb).as_deref()).await
    }

    /// Whether the kind 0 of the key `hex` says it is a bot.
    async fn is_profile_bot(&self, hex: &str) -> Result<bool> {
        let Some(pk) = PubKey::parse(hex) else {
            return Ok(false);
        };
        Ok(self.profiles.get(&pk).await?.is_some_and(|p| p.bot))
    }

    /// What `MessageView.kb` and `.pressed` show of a row: the keyboard of
    /// a text not deleted, of mine or of a key whose profile says `bot`;
    /// my press on an incoming one.
    pub(crate) async fn keyboard_view(
        &self,
        r: &MessageRow,
    ) -> Result<(Option<Keyboard>, Option<Pressed>)> {
        if r.content_type != repo::CT_TEXT || r.deleted_at.is_some() || r.media_json.is_none() {
            return Ok((None, None));
        }
        let state = KbState::stored(r.media_json.as_deref());
        let Some(kb) = state.kb else {
            return Ok((None, None));
        };
        if r.direction == repo::DIR_OUT {
            return Ok((Some(kb), None));
        }
        if !self.is_profile_bot(&r.sender_pubkey).await? {
            return Ok((None, None));
        }
        Ok((Some(kb), state.pressed))
    }

    /// Whether I may press the button `d` under `row` now: an incoming
    /// text, shown and not deleted (`press_unknown`), with a button of
    /// that data (`press_no_button`), of a key whose profile says `bot`
    /// (`press_not_bot`), not pressed within `PRESS_REPEAT_SECS` without an
    /// edit since (`press_pending`). The state of its buttons, for
    /// `record_press`. Shared by direct chats and groups.
    pub async fn press_check(&self, row: &MessageRow, d: &str, now: i64) -> Result<KbState> {
        if row.is_hidden
            || row.content_type != repo::CT_TEXT
            || row.direction != repo::DIR_IN
            || row.deleted_at.is_some()
        {
            return Err(unknown());
        }
        let state = KbState::stored(row.media_json.as_deref());
        if !state.kb.as_ref().is_some_and(|kb| kb.has_data(d)) {
            return Err(MessengerError::Invalid("press_no_button".into()));
        }
        if !self.is_profile_bot(&row.sender_pubkey).await? {
            return Err(MessengerError::Invalid("press_not_bot".into()));
        }
        if state
            .pressed
            .as_ref()
            .is_some_and(|p| (now - p.at).abs() < PRESS_REPEAT_SECS)
        {
            return Err(MessengerError::Invalid("press_pending".into()));
        }
        Ok(state)
    }

    /// The press of `d` at `now` is recorded on `id`.
    pub async fn record_press(
        &self,
        id: &str,
        mut state: KbState,
        d: &str,
        now: i64,
    ) -> Result<()> {
        state.pressed = Some(Pressed {
            d: d.to_string(),
            at: now,
        });
        repo::set_keyboard(&self.store, id, Some(&state.json())).await
    }

    /// Press the button `d` under a message of a bot in a direct chat:
    /// checked (`press_check`, and the chat must be one I send notes in,
    /// with the codes of a reaction refused), recorded, and the note for
    /// the author built. Nothing else happens in the app.
    pub async fn prepare_press(
        &self,
        keys: &Keys,
        message_id: &str,
        d: &str,
    ) -> Result<PreparedPress> {
        let row = repo::get(&self.store, message_id)
            .await?
            .ok_or_else(unknown)?;
        let peer = DmPeer::of_chat(&row.chat_id).ok_or_else(unknown)?;
        // The author is the peer of the chat: a press goes to nobody else.
        if peer.key.as_hex() != row.sender_pubkey {
            return Err(unknown());
        }
        let now = self.clock.now().secs();
        let state = self.press_check(&row, d, now).await?;
        if !self.notes_allowed(&peer).await? {
            let r = self.load_relation(&peer).await?;
            let code = match outbound_permission(&r, 1, true) {
                OutboundPermission::Deny(reason) => reason.as_str(),
                _ => DenyReason::WaitingApproval.as_str(),
            };
            return Err(MessengerError::Invalid(code.into()));
        }
        let w = wrap_note(
            keys,
            &peer,
            &Envelope::press(&row.id, d).encode(),
            now,
            false,
            None,
        )?;
        let to_peer = self.route_to_peer(&peer, w.to_peer).await?;
        self.record_press(&row.id, state, d, now).await?;
        let message = self
            .message(&row.id)
            .await?
            .ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        Ok(PreparedPress {
            chat_id: row.chat_id,
            message,
            to_peer,
        })
    }

    /// A `press` from `from` in `chat_id`, heard by `me`: the message and
    /// the data when it is to be told, `None` when it is dropped. Dropped:
    /// my own, one that is malformed, one naming a message that is not a
    /// text of mine in this very chat (shown, not deleted) or a button the
    /// message does not have now (an edit took it away), and a repeat of
    /// the same (message, data, presser) within `PRESS_DEDUP_SECS` of the
    /// first. Shared by direct chats and groups.
    pub async fn press_heard(
        &self,
        chat_id: &str,
        me: &PubKey,
        from: &PubKey,
        envelope: &Envelope,
        now: i64,
    ) -> Result<Option<(String, String)>> {
        if from == me {
            return Ok(None);
        }
        let Some((of, d)) = press_of(envelope) else {
            return Ok(None);
        };
        let Some(row) = repo::get(&self.store, &of).await? else {
            return Ok(None);
        };
        let mine = row.chat_id == chat_id
            && row.sender_pubkey == me.as_hex()
            && row.direction == repo::DIR_OUT
            && row.content_type == repo::CT_TEXT
            && !row.is_hidden
            && row.deleted_at.is_none();
        if !mine
            || !KbState::stored(row.media_json.as_deref())
                .kb
                .is_some_and(|kb| kb.has_data(&d))
        {
            return Ok(None);
        }
        let mut seen = self.presses.lock().unwrap();
        seen.retain(|_, at| (now - *at).abs() < PRESS_DEDUP_SECS);
        let key = (of.clone(), d.clone(), from.as_hex().to_string());
        if seen.contains_key(&key) {
            return Ok(None);
        }
        seen.insert(key, now);
        Ok(Some((of, d)))
    }
}

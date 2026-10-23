// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! How wide I made the panes of the app on a computer, the same on every
//! computer of mine (`own.ui`, internal/messenger-wire.md §3, "Свои
//! устройства"). The whole map travels with the time it was last changed;
//! the later map wins, so every device, a new one too, ends with the same.
//! It is kept in `msg_settings`, so a start without the network has it.
//!
//! A change here is owed to my other devices (`ui_prefs.dirty`) and goes
//! with the next tick of the session (every two seconds), however many
//! changes came meanwhile; the map goes again every few days.

use crate::service::DmService;
use messenger_core::envelope::T_OWN_UI;
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, Envelope, MessengerError, Result};
use messenger_store::settings;
use serde_json::Value;
use std::collections::BTreeMap;

/// The map changed by a note from another device of mine: `{w: {key: px}}`.
pub const UI_EVENT_UI_PREFS_UPDATED: &str = "ui_prefs.updated";

/// The map as kept: `{"w":{…},"at":<secs>}`.
pub const KEY_UI_PREFS: &str = "ui_prefs.map";
/// `"1"` while a change here was not sent to my other devices.
pub const KEY_UI_PREFS_DIRTY: &str = "ui_prefs.dirty";
/// When this device last sent the map (unix seconds).
pub const KEY_UI_PREFS_SENT_AT: &str = "ui_prefs.sent_at";
/// The map goes again after this long, so a device added since learns it.
pub const UI_PREFS_EVERY_SECS: i64 = 3 * 86_400;

/// The narrowest and the widest a pane may be kept.
pub const UI_PX_MIN: u32 = 200;
pub const UI_PX_MAX: u32 = 2000;
/// The most keys a map holds; a note with more takes the first.
pub const UI_PREFS_MAX_KEYS: usize = 64;
/// The longest key.
pub const UI_KEY_MAX_LEN: usize = 40;

/// Error of a key or a width out of bounds.
pub const ERR_UI_PREF_INVALID: &str = "ui_pref_invalid";

/// The widths of the panes and when they last changed (0: never).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiPrefs {
    pub widths: BTreeMap<String, u32>,
    pub at: i64,
}

impl UiPrefs {
    fn note(&self) -> Envelope {
        let widths: Vec<(String, u32)> = self.widths.iter().map(|(k, v)| (k.clone(), *v)).collect();
        Envelope::own_ui(&widths, self.at)
    }

    fn encode(&self) -> String {
        serde_json::json!({ "w": self.widths, "at": self.at }).to_string()
    }

    /// The map of a note or of the store: what is no key or no width in
    /// bounds is skipped. `None` without a time.
    fn parse(fields: &serde_json::Map<String, Value>) -> Option<Self> {
        let at = fields.get("at").and_then(Value::as_i64).filter(|at| *at > 0)?;
        let widths = fields
            .get("w")
            .and_then(Value::as_object)
            .map(|w| {
                w.iter()
                    .filter(|(k, _)| valid_key(k))
                    .filter_map(|(k, v)| Some((k.clone(), v.as_u64().and_then(|px| u32::try_from(px).ok()).filter(|px| valid_px(*px))?)))
                    .take(UI_PREFS_MAX_KEYS)
                    .collect()
            })
            .unwrap_or_default();
        Some(Self { widths, at })
    }
}

/// A key of a pane: `[a-z0-9._-]`, 1 to 40 of them.
pub fn valid_key(key: &str) -> bool {
    (1..=UI_KEY_MAX_LEN).contains(&key.len())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// A width a pane may be kept at.
pub fn valid_px(px: u32) -> bool {
    (UI_PX_MIN..=UI_PX_MAX).contains(&px)
}

impl DmService {
    /// The widths of the panes as this device keeps them.
    pub async fn ui_prefs(&self) -> Result<UiPrefs> {
        let Some(raw) = settings::get(&self.store, KEY_UI_PREFS).await? else { return Ok(UiPrefs::default()) };
        Ok(serde_json::from_str::<serde_json::Map<String, Value>>(&raw).ok().and_then(|m| UiPrefs::parse(&m)).unwrap_or_default())
    }

    /// Keep `px` as the width of the pane `key`; `None` forgets it (the
    /// pane takes its own default). My other devices are owed the map when
    /// it changed. Error: `ui_pref_invalid`.
    pub async fn set_ui_pref(&self, key: &str, px: Option<u32>) -> Result<UiPrefs> {
        if !valid_key(key) || px.is_some_and(|px| !valid_px(px)) {
            return Err(MessengerError::Invalid(ERR_UI_PREF_INVALID.into()));
        }
        let mut kept = self.ui_prefs().await?;
        if kept.widths.get(key).copied() == px {
            return Ok(kept);
        }
        match px {
            Some(px) => {
                if !kept.widths.contains_key(key) && kept.widths.len() >= UI_PREFS_MAX_KEYS {
                    return Err(MessengerError::Invalid(ERR_UI_PREF_INVALID.into()));
                }
                kept.widths.insert(key.to_string(), px);
            }
            None => {
                kept.widths.remove(key);
            }
        }
        // Later than whatever this device knew, even with a clock behind.
        kept.at = self.clock.now().secs().max(kept.at + 1);
        settings::set(&self.store, KEY_UI_PREFS, &kept.encode()).await?;
        settings::set(&self.store, KEY_UI_PREFS_DIRTY, "1").await?;
        Ok(kept)
    }

    /// The note of the map when my other devices are owed it: it changed
    /// here, or none went for `UI_PREFS_EVERY_SECS` and it was ever set.
    /// Call `ui_prefs_sent` once it is queued.
    pub async fn ui_prefs_due(&self, now: i64) -> Result<Option<Envelope>> {
        let kept = self.ui_prefs().await?;
        if kept.at == 0 {
            return Ok(None);
        }
        let dirty = settings::get(&self.store, KEY_UI_PREFS_DIRTY).await?.as_deref() == Some("1");
        let last = settings::get(&self.store, KEY_UI_PREFS_SENT_AT).await?.and_then(|s| s.parse::<i64>().ok());
        if !dirty && last.is_some_and(|at| at > now - UI_PREFS_EVERY_SECS) {
            return Ok(None);
        }
        Ok(Some(kept.note()))
    }

    /// The map was queued at `now`.
    pub async fn ui_prefs_sent(&self, now: i64) -> Result<()> {
        settings::delete(&self.store, KEY_UI_PREFS_DIRTY).await?;
        settings::set(&self.store, KEY_UI_PREFS_SENT_AT, &now.to_string()).await
    }

    /// The map from another device of mine: the later one is kept whole.
    /// At the same time the larger map in its JSON wins, so two devices
    /// that changed it in one second still end alike.
    pub(crate) async fn ui_prefs_elsewhere(&self, envelope: &Envelope) -> Result<Vec<Effect>> {
        debug_assert_eq!(envelope.t, T_OWN_UI);
        let Some(theirs) = UiPrefs::parse(&envelope.fields) else { return Ok(vec![]) };
        let mine = self.ui_prefs().await?;
        let wins = theirs.at > mine.at || (theirs.at == mine.at && theirs.encode() > mine.encode());
        if !wins {
            return Ok(vec![]);
        }
        settings::set(&self.store, KEY_UI_PREFS, &theirs.encode()).await?;
        // A change here not sent yet lost to the later one, and my devices
        // were just told the map: nothing is owed until the next round.
        settings::delete(&self.store, KEY_UI_PREFS_DIRTY).await?;
        settings::set(&self.store, KEY_UI_PREFS_SENT_AT, &self.clock.now().secs().to_string()).await?;
        if theirs.widths == mine.widths {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent { name: UI_EVENT_UI_PREFS_UPDATED.into(), payload: serde_json::json!({ "w": theirs.widths }) })])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_widths_in_bounds() {
        for ok in ["channels.side", "chats.side", "a", "x-1_2.z", &"k".repeat(40)] {
            assert!(valid_key(ok), "{ok}");
        }
        for bad in ["", "Channels", "a b", "ключ", "a/b", &"k".repeat(41)] {
            assert!(!valid_key(bad), "{bad}");
        }
        assert!(valid_px(200) && valid_px(2000) && valid_px(340));
        assert!(!valid_px(199) && !valid_px(2001) && !valid_px(0));
    }

    #[test]
    fn a_map_skips_what_is_out_of_bounds() {
        let note = Envelope::new(T_OWN_UI).with(
            "w",
            serde_json::json!({ "chats.side": 360, "Bad": 300, "small": 10, "big": 9000, "neg": -5, "text": "400", "ok.x": 2000 }),
        );
        assert_eq!(UiPrefs::parse(&note.fields), None, "no time");
        let note = note.with("at", 7);
        let p = UiPrefs::parse(&note.fields).unwrap();
        assert_eq!(p.at, 7);
        assert_eq!(p.widths.into_iter().collect::<Vec<_>>(), vec![("chats.side".into(), 360), ("ok.x".into(), 2000)]);
        assert_eq!(UiPrefs::parse(&Envelope::own_ui(&[], 0).fields), None, "a time of 0 is none");
    }
}

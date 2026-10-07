// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notes between my devices. A chat read on one is read on the others; a
//! message removed for me on one is removed on the others. A note is a
//! rumor of `KIND_OWN_RUMOR` wrapped to my own key (internal/messenger-wire.md
//! §3, "Свои устройства"); building the wrap and sending it is the
//! runtime's job, this module says what the note is and applies one.
//!
//! Notes and messages come in any order: what a note says is kept by the
//! chat or the message it names (`msg_own_read`, `msg_own_hidden`) and
//! holds for what comes after it.
//!
//! The emoji I use most travel the same way: each device counts its own
//! uses and now and then sends the whole map (`own.emoji`); a map that
//! comes raises each count and time to the larger, so every device, a new
//! one too, ends with the same map.
//!
//! The epoch of my presence key travels the same way (`own.presence`),
//! with when it began and the presence switch: the device that rotates
//! tells the contacts the new key, the others take the epoch, beat from
//! the new key, tell it with the same `since` and tell nobody what they
//! told before; a switch turned off on one device is off on all.
//!
//! What of my profile stays off kind 0 travels the same way: my phone and
//! whether my card carries it (`own.profile`, sent on a change and again
//! every week), and the phone each contact sent me in its own card
//! (`own.card`). The later `at` wins, per contact for the second.

use crate::service::{DmService, UI_EVENT_DM_UPDATED};
use messenger_contacts::normalize_phone;
use messenger_core::emoji::is_reaction;
use messenger_core::envelope::{T_OWN_CARD, T_OWN_EMOJI, T_OWN_HIDE, T_OWN_PRESENCE, T_OWN_PROFILE, T_OWN_READ};
use messenger_core::traits::UiEvent;
use messenger_core::{DmInbound, Effect, Envelope, MessengerError, PubKey, Result};
use messenger_store::contact_private::{self, ContactPrivate};
use messenger_store::messages as repo;
use messenger_store::own_private::{self, OwnPrivate};
use messenger_store::{chats, emoji_usage, presence, settings};
use serde_json::Value;

/// A chat was read on another device of mine: its counter went down.
pub const UI_EVENT_CHAT_READ: &str = "chat.read";
/// A peer (or a member of a group) told how far they have read the chat:
/// its messages show another mark.
pub const UI_EVENT_CHAT_RECEIPT: &str = "chat.receipt";
/// The emoji I use most changed by a map from another device of mine.
pub const UI_EVENT_EMOJI_UPDATED: &str = "emoji.updated";

/// `"1"` while this device counted a use my other devices have not been
/// sent yet.
pub const KEY_EMOJI_DIRTY: &str = "emoji.dirty";
/// When this device last sent its map of emoji (unix seconds).
pub const KEY_EMOJI_SNAPSHOT_AT: &str = "emoji.snapshot_at";
/// A map goes no more often than this, however many uses come.
pub const EMOJI_SNAPSHOT_EVERY_SECS: i64 = 600;
/// The most a map may weigh as JSON: the least used fall off.
pub const EMOJI_SNAPSHOT_MAX_BYTES: usize = 2048;
/// The most emoji `emoji_top` hands out.
pub const EMOJI_TOP_MAX: usize = 64;

pub use messenger_core::presence::KEY_PRESENCE;
/// The epoch of my presence key (`messenger_presence::key::derive`); none
/// is 0. It only grows.
pub const KEY_PRESENCE_EPOCH: &str = "presence.epoch";
/// When the current epoch began (unix seconds; none is 0), the same on
/// every device of mine: the `since` of the key told to contacts, so a key
/// of an older epoch told late by a device that missed the rotation loses
/// to the newer one.
pub const KEY_PRESENCE_SINCE: &str = "presence.since";
/// The newest epoch my other devices know of: by an `own.presence` this
/// device sent, or one it heard. A rotation without a session leaves the
/// note owed until the next one.
pub const KEY_PRESENCE_DEVICES_TOLD: &str = "presence.devices_told";
/// My presence key moved to another epoch, or the switch moved on another
/// device: `{}`.
pub const UI_EVENT_PRESENCE_EPOCH_CHANGED: &str = "presence.epoch_changed";

/// My phone or the default of my card changed by a note from another
/// device of mine: `{}`.
pub const UI_EVENT_OWN_PRIVATE_UPDATED: &str = "own_private.updated";
/// The phone a contact sent me changed by a note from another device of
/// mine: `{pubkey}`.
pub const UI_EVENT_CONTACT_PRIVATE_UPDATED: &str = "contact_private.updated";
/// When this device last sent `own.profile` (unix seconds).
pub const KEY_OWN_PROFILE_SENT_AT: &str = "own_profile.sent_at";
/// `own.profile` goes again after this long, so a device added since
/// learns my phone.
pub const OWN_PROFILE_EVERY_SECS: i64 = 7 * 86_400;

/// Where my presence key stands (`DmService::presence_state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresenceState {
    pub epoch: u32,
    pub since: i64,
    pub sharing: bool,
}

impl PresenceState {
    /// The note that tells my other devices.
    pub fn note(&self) -> Envelope {
        Envelope::own_presence(self.epoch, self.since, self.sharing)
    }
}

impl DmService {
    /// The chat was read on this device. The note for my other devices when
    /// there is something they do not know yet.
    pub async fn mark_read(&self, chat_id: &str) -> Result<Option<Envelope>> {
        Ok(chats::mark_read(&self.store, chat_id).await?.map(|at| Envelope::own_read(chat_id, at)))
    }

    /// Hide a message on this device (works for incoming ones too). The
    /// note that hides it on my other devices.
    pub async fn delete_local(&self, message_id: &str) -> Result<Envelope> {
        let row = repo::get(&self.store, message_id)
            .await?
            .ok_or_else(|| messenger_core::MessengerError::Invalid("unknown message".into()))?;
        let at = self.clock.now().secs();
        repo::remember_hidden(&self.store, message_id, at).await?;
        repo::mark_deleted(&self.store, message_id, at).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        Ok(Envelope::own_hide(&row.chat_id, message_id))
    }

    /// A note of mine: already checked to be sealed by me (ingress).
    pub(crate) async fn apply_own(&self, msg: &DmInbound, envelope: &Envelope) -> Result<Vec<Effect>> {
        // The one note about no chat.
        if envelope.t == T_OWN_EMOJI {
            return self.emoji_elsewhere(envelope).await;
        }
        if envelope.t == T_OWN_PRESENCE {
            return self.presence_elsewhere(envelope).await;
        }
        if envelope.t == T_OWN_PROFILE {
            return self.own_private_elsewhere(envelope).await;
        }
        if envelope.t == T_OWN_CARD {
            return self.contact_private_elsewhere(envelope).await;
        }
        let Some(chat_id) = envelope.str_field("chat") else { return Ok(vec![]) };
        match envelope.t.as_str() {
            T_OWN_READ => {
                let Some(at) = envelope.fields.get("at").and_then(|v| v.as_i64()) else { return Ok(vec![]) };
                self.read_elsewhere(chat_id, at).await
            }
            T_OWN_HIDE => {
                let Some(target) = envelope.str_field("target") else { return Ok(vec![]) };
                let at = msg.created_at.secs();
                repo::remember_hidden(&self.store, target, at).await?;
                let Some(row) = repo::get(&self.store, target).await? else { return Ok(vec![]) };
                if row.chat_id != chat_id || row.is_hidden || row.deleted_at.is_some() {
                    return Ok(vec![]);
                }
                repo::mark_deleted(&self.store, target, at).await?;
                chats::recompute_last(&self.store, chat_id).await?;
                Ok(vec![Effect::Emit(UiEvent {
                    name: UI_EVENT_DM_UPDATED.into(),
                    payload: serde_json::json!({ "chat_id": chat_id, "message_id": target }),
                })])
            }
            _ => Ok(vec![]),
        }
    }

    /// The chat was read on another device of mine up to `at`: by a note,
    /// or by a message I wrote there (who answers has read what came before).
    pub async fn read_elsewhere(&self, chat_id: &str, at: i64) -> Result<Vec<Effect>> {
        if !chats::read_up_to(&self.store, chat_id, at).await? {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent { name: UI_EVENT_CHAT_READ.into(), payload: serde_json::json!({ "chat_id": chat_id }) })])
    }

    /// The map of emoji from another device of mine: what is not an emoji
    /// or not a pair of numbers is skipped, and no more than a map holds is
    /// taken.
    async fn emoji_elsewhere(&self, envelope: &Envelope) -> Result<Vec<Effect>> {
        let Some(map) = envelope.fields.get("usage").and_then(|v| v.as_object()) else { return Ok(vec![]) };
        let usage: Vec<(String, i64, i64)> = map
            .iter()
            .filter(|(emoji, _)| is_reaction(emoji))
            .filter_map(|(emoji, v)| match v.as_array().map(Vec::as_slice) {
                Some([count, at]) => Some((emoji.clone(), count.as_i64()?, at.as_i64()?)),
                _ => None,
            })
            .take(emoji_usage::SNAPSHOT_MAX as usize)
            .collect();
        if !emoji_usage::merge(&self.store, &usage).await? {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent { name: UI_EVENT_EMOJI_UPDATED.into(), payload: serde_json::json!({}) })])
    }

    /// The epoch of my presence key; 0 before any rotation.
    pub async fn presence_epoch(&self) -> Result<u32> {
        Ok(self.presence_state().await?.epoch)
    }

    /// When the current epoch began; 0 for epoch 0.
    pub async fn presence_since(&self) -> Result<i64> {
        Ok(self.presence_state().await?.since)
    }

    /// The epoch of my presence key, when it began, and whether I share
    /// presence at all: the same on every device of mine once the notes
    /// went round.
    pub async fn presence_state(&self) -> Result<PresenceState> {
        Ok(PresenceState {
            epoch: settings::get(&self.store, KEY_PRESENCE_EPOCH).await?.and_then(|s| s.parse().ok()).unwrap_or(0),
            since: settings::get(&self.store, KEY_PRESENCE_SINCE).await?.and_then(|s| s.parse().ok()).unwrap_or(0),
            sharing: settings::get_bool(&self.store, KEY_PRESENCE, true).await?,
        })
    }

    async fn set_presence_state(&self, s: &PresenceState) -> Result<()> {
        settings::set_bool(&self.store, KEY_PRESENCE, s.sharing).await?;
        settings::set(&self.store, KEY_PRESENCE_SINCE, &s.since.to_string()).await?;
        settings::set(&self.store, KEY_PRESENCE_EPOCH, &s.epoch.to_string()).await
    }

    /// This device moves my presence key to the next epoch, sharing or not
    /// from now on. The epoch begins a second from now and never at or
    /// before the one it follows, so whatever was told of the old key,
    /// a withdrawal too, is older. My other devices are owed the news
    /// (`presence_devices_owed`).
    pub async fn rotate_presence(&self, sharing: bool) -> Result<PresenceState> {
        let s = self.presence_state().await?;
        let next = PresenceState {
            epoch: s.epoch.saturating_add(1),
            since: (self.clock.now().secs() + 1).max(s.since + 1),
            sharing,
        };
        self.set_presence_state(&next).await?;
        Ok(next)
    }

    /// The state my other devices have not been told (`own.presence`): this
    /// device moved to an epoch none of them told it of. Told once.
    pub async fn presence_devices_owed(&self) -> Result<Option<PresenceState>> {
        let s = self.presence_state().await?;
        Ok((s.epoch > self.presence_devices_told_epoch().await?).then_some(s))
    }

    /// My other devices know of `epoch`.
    pub async fn presence_devices_told(&self, epoch: u32) -> Result<()> {
        if epoch > self.presence_devices_told_epoch().await? {
            settings::set(&self.store, KEY_PRESENCE_DEVICES_TOLD, &epoch.to_string()).await?;
        }
        Ok(())
    }

    async fn presence_devices_told_epoch(&self) -> Result<u32> {
        Ok(settings::get(&self.store, KEY_PRESENCE_DEVICES_TOLD).await?.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Another device of mine rotated, or turned presence on or off: this
    /// one takes the same epoch, `since` and switch. A newer epoch wins; at
    /// the same epoch (two devices that rotated at once) the later `since`
    /// and "not sharing" win, so both end alike. Whom this device told
    /// counts as told the new key: the device that rotated tells it, so a
    /// contact removed there is not told here before this device learns of
    /// the removal; with sharing off the runtime withdraws it from them.
    async fn presence_elsewhere(&self, envelope: &Envelope) -> Result<Vec<Effect>> {
        let epoch = envelope.fields.get("epoch").and_then(|v| v.as_u64()).and_then(|e| u32::try_from(e).ok());
        let since = envelope.fields.get("since").and_then(|v| v.as_i64()).filter(|s| *s >= 0);
        let (Some(epoch), Some(since)) = (epoch, since) else { return Ok(vec![]) };
        let sharing = envelope.fields.get("sharing").and_then(|v| v.as_bool()).unwrap_or(true);
        let mine = self.presence_state().await?;
        let next = match epoch.cmp(&mine.epoch) {
            std::cmp::Ordering::Greater => PresenceState { epoch, since, sharing },
            std::cmp::Ordering::Equal => {
                PresenceState { epoch, since: since.max(mine.since), sharing: sharing && mine.sharing }
            }
            std::cmp::Ordering::Less => return Ok(vec![]),
        };
        if next == mine {
            return Ok(vec![]);
        }
        self.set_presence_state(&next).await?;
        self.presence_devices_told(epoch).await?;
        if epoch > mine.epoch {
            presence::carry_told(&self.store, epoch).await?;
        }
        Ok(vec![Effect::Emit(UiEvent { name: UI_EVENT_PRESENCE_EPOCH_CHANGED.into(), payload: serde_json::json!({}) })])
    }

    /// I used `emoji` here at `at`, for a reaction or in a message: it
    /// counts, and my other devices are owed the map.
    pub async fn emoji_used(&self, emoji: &str, at: i64) -> Result<()> {
        if !is_reaction(emoji) {
            return Err(MessengerError::Invalid(crate::reactions::Refusal::Invalid.code().into()));
        }
        emoji_usage::bump(&self.store, emoji, at).await?;
        settings::set(&self.store, KEY_EMOJI_DIRTY, "1").await
    }

    /// The `n` emoji I use most, the most used first.
    pub async fn emoji_top(&self, n: usize) -> Result<Vec<String>> {
        if n == 0 {
            return Ok(vec![]);
        }
        emoji_usage::top(&self.store, n.min(EMOJI_TOP_MAX) as i64).await
    }

    /// The map for my other devices when uses were counted since the last
    /// one and that was at least `EMOJI_SNAPSHOT_EVERY_SECS` ago. Taken:
    /// the next one waits for new uses. If it cannot be sent, call
    /// `emoji_snapshot_again`.
    pub async fn emoji_snapshot_if_due(&self, now: i64) -> Result<Option<Envelope>> {
        let last = settings::get(&self.store, KEY_EMOJI_SNAPSHOT_AT).await?.and_then(|s| s.parse::<i64>().ok());
        if last.is_some_and(|at| at > now - EMOJI_SNAPSHOT_EVERY_SECS) {
            return Ok(None);
        }
        self.emoji_snapshot_now(now).await
    }

    /// The map for my other devices if uses were counted since the last
    /// one, however recent that was: the session is closing.
    pub async fn emoji_snapshot_now(&self, now: i64) -> Result<Option<Envelope>> {
        if settings::get(&self.store, KEY_EMOJI_DIRTY).await?.as_deref() != Some("1") {
            return Ok(None);
        }
        settings::delete(&self.store, KEY_EMOJI_DIRTY).await?;
        settings::set(&self.store, KEY_EMOJI_SNAPSHOT_AT, &now.to_string()).await?;
        let mut usage = emoji_usage::all(&self.store).await?;
        let mut note = Envelope::own_emoji(&usage);
        // The least used fall off until it fits.
        while note.encode().len() > EMOJI_SNAPSHOT_MAX_BYTES && !usage.is_empty() {
            usage.pop();
            note = Envelope::own_emoji(&usage);
        }
        Ok(Some(note))
    }

    /// The map taken by `emoji_snapshot_*` did not leave: it is owed again.
    pub async fn emoji_snapshot_again(&self) -> Result<()> {
        settings::set(&self.store, KEY_EMOJI_DIRTY, "1").await
    }

    // ─── My phone, and the phones my contacts sent me ───────────────────────

    /// My phone and whether my card carries it by default.
    pub async fn own_private(&self) -> Result<OwnPrivate> {
        own_private::get(&self.store).await
    }

    /// Change my phone (any way of writing a number; empty or `None`
    /// removes it) and the default of my card. Returns what is kept and the
    /// note for my other devices. Error: `phone_invalid`.
    pub async fn set_own_private(&self, phone: Option<&str>, share_phone: bool) -> Result<(OwnPrivate, Envelope)> {
        let phone = clean_phone(phone)?;
        let kept = own_private::set(&self.store, phone.as_deref(), share_phone, self.clock.now().secs()).await?;
        Ok((kept.clone(), own_profile_note(&kept)))
    }

    /// The note of my phone when my other devices are owed it again: it was
    /// ever set, and none went for `OWN_PROFILE_EVERY_SECS` (a device added
    /// since learns it so). Call `own_profile_sent` once it is queued.
    pub async fn own_profile_due(&self, now: i64) -> Result<Option<Envelope>> {
        let kept = own_private::get(&self.store).await?;
        if kept.updated_at == 0 {
            return Ok(None);
        }
        let last = settings::get(&self.store, KEY_OWN_PROFILE_SENT_AT).await?.and_then(|s| s.parse::<i64>().ok());
        if last.is_some_and(|at| at > now - OWN_PROFILE_EVERY_SECS) {
            return Ok(None);
        }
        Ok(Some(own_profile_note(&kept)))
    }

    /// The note of my phone was queued at `now`.
    pub async fn own_profile_sent(&self, now: i64) -> Result<()> {
        settings::set(&self.store, KEY_OWN_PROFILE_SENT_AT, &now.to_string()).await
    }

    /// The note of a change could not be queued: it is owed at once, so
    /// the next `own_profile_due` gives it whenever the last one went.
    pub async fn own_profile_owed(&self) -> Result<()> {
        settings::delete(&self.store, KEY_OWN_PROFILE_SENT_AT).await
    }

    /// The phone a contact sent me in its own card, if any.
    pub async fn contact_private(&self, pubkey: &str) -> Result<Option<ContactPrivate>> {
        contact_private::get(&self.store, pubkey).await
    }

    /// Keep `phone` as the contact's (`None` forgets it), as its card made
    /// at `at` says (the card's own time, never later than now): a card
    /// older than the one the kept phone came from changes nothing, here
    /// or, by the same time in the note, on my other devices. The note for
    /// my other devices, or `None` when nothing changed.
    pub async fn keep_contact_phone(&self, pubkey: &str, phone: Option<&str>, at: i64) -> Result<Option<Envelope>> {
        let pk = PubKey::parse(pubkey).ok_or_else(|| MessengerError::Invalid("card_invalid".into()))?;
        let phone = clean_phone(phone)?;
        let at = at.clamp(1, self.clock.now().secs().max(1));
        let old = contact_private::get(&self.store, pk.as_hex()).await?;
        if old.as_ref().map(|o| &o.phone) == Some(&phone) {
            return Ok(None);
        }
        let value = ContactPrivate { pubkey: pk.as_hex().to_string(), phone, updated_at: at };
        if !contact_private::put_if_newer(&self.store, &value).await? {
            return Ok(None);
        }
        Ok(Some(Envelope::own_card(&value.pubkey, value.phone.as_deref(), value.updated_at)))
    }

    /// My phone as another device of mine has it: the later one is kept.
    async fn own_private_elsewhere(&self, envelope: &Envelope) -> Result<Vec<Effect>> {
        let (Some(phone), Some(at)) = (note_phone(envelope), note_at(envelope)) else { return Ok(vec![]) };
        let share_phone = envelope.fields.get("share_phone").and_then(|v| v.as_bool()).unwrap_or(false);
        let value = OwnPrivate { phone, share_phone, updated_at: at };
        if !own_private::put_if_newer(&self.store, &value).await? {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent { name: UI_EVENT_OWN_PRIVATE_UPDATED.into(), payload: serde_json::json!({}) })])
    }

    /// The phone of a contact as another device of mine has it: per
    /// contact, the later one is kept.
    async fn contact_private_elsewhere(&self, envelope: &Envelope) -> Result<Vec<Effect>> {
        let pubkey = envelope.str_field("pubkey").and_then(PubKey::parse);
        let (Some(pubkey), Some(phone), Some(at)) = (pubkey, note_phone(envelope), note_at(envelope)) else { return Ok(vec![]) };
        let value = ContactPrivate { pubkey: pubkey.as_hex().to_string(), phone, updated_at: at };
        if !contact_private::put_if_newer(&self.store, &value).await? {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_CONTACT_PRIVATE_UPDATED.into(),
            payload: serde_json::json!({ "pubkey": value.pubkey }),
        })])
    }

    /// A message just stored that was removed for me before it came: it is
    /// removed now. `true` when it was.
    pub async fn hide_if_hidden(&self, message_id: &str) -> Result<bool> {
        match repo::hidden_at(&self.store, message_id).await? {
            Some(at) => {
                repo::mark_deleted(&self.store, message_id, at).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// A phone as the user wrote it, in the one form; none for nothing.
fn clean_phone(phone: Option<&str>) -> Result<Option<String>> {
    match phone.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Ok(Some(normalize_phone(p).map_err(|code| MessengerError::Invalid(code.into()))?)),
        None => Ok(None),
    }
}

fn own_profile_note(kept: &OwnPrivate) -> Envelope {
    Envelope::own_profile(kept.phone.as_deref(), kept.share_phone, kept.updated_at)
}

/// The `phone` of a note: `Some(None)` when there is none, `None` when it
/// is no number (the note is dropped).
fn note_phone(envelope: &Envelope) -> Option<Option<String>> {
    match envelope.fields.get("phone") {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(p)) => normalize_phone(p).ok().map(Some),
        Some(_) => None,
    }
}

fn note_at(envelope: &Envelope) -> Option<i64> {
    envelope.fields.get("at").and_then(Value::as_i64).filter(|at| *at > 0)
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! From a push to a notice: the event opened, the chat looked up, the
//! rules of the chat applied. Everything the app would decide about the
//! message is decided the same way here, from the same rows, only
//! nothing is written: the app writes when it runs.

use crate::bundle::KeyBundle;
use crate::fetch;
use crate::notice::{Body, CallNotice, ChatKind, Notice, Outcome, Plain, Reason};
use crate::push::{PushData, PushKind};
use crate::settings::{Content, Settings};
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::envelope::{T_CALL_INVITE, T_CONTACT, T_CONTROL, T_DELETE, T_EDIT, T_MEDIA, T_TEXT};
use messenger_core::traits::{Clock, SystemClock};
use messenger_core::{Envelope, EventSource, Inbound, MessengerConfig, MessengerError, PubKey, RawEvent, RelayUrl, Result, Timestamp};
use messenger_dm::pushtags::{author_key, author_mark};
use messenger_dm::relationship::{inbound_decision, InboundDecision};
use messenger_dm::body::body_of;
use messenger_dm::DmService;
use messenger_groups::wire::{self, Opened, T_INVITE, T_JOIN_REQUEST, T_WELCOME};
use messenger_groups::service::MEMBERSHIP_JOINED;
use messenger_ingress::classify::KIND_DM_RUMOR;
use messenger_store::{chats, groups, messages, Store};
use nostr::key::Keys;
use std::path::Path;
use std::sync::Arc;

/// Says what the push is about. `data_dir` is the messenger's data
/// directory, the one with `messenger.db` in it.
/// Without keys (the app has a lock, or the settings want nothing said)
/// only what the database knows of the chat is told: the group's name,
/// whether it is muted.
pub async fn describe(data_dir: &Path, bundle: Option<&KeyBundle>, push: &PushData) -> Result<Outcome> {
    let config = MessengerConfig::new(data_dir);
    let store = Store::open_read_only(&config).await?;
    let out = match bundle {
        Some(bundle) => match bundle.keys() {
            Ok(keys) => Describe::new(store.clone(), keys).run(bundle, push).await,
            Err(e) => Err(e),
        },
        None => plain(&store, push).await.map(Outcome::Plain),
    };
    store.close().await;
    out
}

struct Describe {
    store: Store,
    keys: Keys,
    me: PubKey,
    contacts: ContactService,
    dm: DmService,
}

impl Describe {
    fn new(store: Store, keys: Keys) -> Self {
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts.clone(), profiles, Arc::new(SystemClock));
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a public key is 64 hex chars");
        Self { store, keys, me, contacts, dm }
    }

    async fn run(&self, bundle: &KeyBundle, push: &PushData) -> Result<Outcome> {
        let settings = Settings::load(&self.store).await?;
        let mut plain = plain(&self.store, push).await?;
        if settings.content == Content::None {
            // Nothing of a message is said, so no message is opened. A
            // call is the one thing that is: a ring is not a word of the
            // content, and a phone that says "only that something came"
            // must still ring. The push says whether it carries a call
            // (anyone can say so on the outside, so the wrap is opened
            // and the call rings from a mutual contact only, with no
            // name); whatever goes wrong in that says only that
            // something came.
            if push.call {
                if let Ok(Some(outcome)) = self.call_only(&settings, push).await {
                    return Ok(outcome);
                }
            }
            return Ok(Outcome::Plain(plain));
        }

        let Some(raw) = self.event_of(push).await? else {
            return Ok(Outcome::Plain(plain));
        };
        if raw.kind == messenger_ingress::classify::KIND_GROUP_MESSAGE && self.written_by_me(&raw) {
            return Ok(Outcome::Quiet { reason: Reason::Own });
        }

        match messenger_ingress::classify(&raw, Some(&self.keys)) {
            Inbound::Dm(dm) => self.dm_notice(dm, &settings, push.count).await,
            Inbound::Group(g) => match bundle.group_key(&g.group_id, g.key_id.as_deref().unwrap_or_default()) {
                Some(key) => self.group_notice(&g.group_id, &key, &g.ciphertext, &settings, push.count).await,
                // A key this device does not have yet: the app will, when it runs.
                None => {
                    plain.chat = Some(groups::group_chat_id(&g.group_id));
                    Ok(Outcome::Plain(plain))
                }
            },
            _ => Ok(Outcome::Quiet { reason: Reason::Invalid }),
        }
    }


    /// The push opened for a call and nothing else: the invitation of a
    /// mutual contact rings (`Some`, with no name and no picture under
    /// the settings that brought us here); any other content is left
    /// unread (`None`).
    async fn call_only(&self, settings: &Settings, push: &PushData) -> Result<Option<Outcome>> {
        let Some(raw) = self.event_of(push).await? else { return Ok(None) };
        let Inbound::Dm(dm) = messenger_ingress::classify(&raw, Some(&self.keys)) else { return Ok(None) };
        if dm.rumor_kind != KIND_DM_RUMOR || dm.sender == self.me {
            return Ok(None);
        }
        let Ok(envelope) = Envelope::parse(&dm.content) else { return Ok(None) };
        if envelope.t != T_CALL_INVITE {
            return Ok(None);
        }
        if self.dm.load_relation(&dm.sender).await?.blocked {
            return Ok(Some(Outcome::Quiet { reason: Reason::Blocked }));
        }
        self.call_notice(&dm.sender, &envelope, dm.created_at.0, settings).await.map(Some)
    }

    async fn event_of(&self, push: &PushData) -> Result<Option<RawEvent>> {
        let json: serde_json::Value = match (&push.event, &push.event_id, &push.relay) {
            (Some(text), _, _) => serde_json::from_str(text).map_err(|e| MessengerError::Invalid(format!("event: {e}")))?,
            (None, Some(id), Some(relay)) => match fetch::event(&self.store, relay, id).await? {
                Some(v) => v,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        let raw = raw_event(json, push.relay.as_deref())?;
        if let Some(id) = &push.event_id {
            if raw.id.as_hex() != id {
                return Err(MessengerError::Invalid("the event is not the one the push named".into()));
            }
        }
        Ok(Some(raw))
    }

    /// Group events are signed by throwaway keys; the author's mark on
    /// them is what says "mine" to this device and to nobody else.
    fn written_by_me(&self, raw: &RawEvent) -> bool {
        let mark = raw.json["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_array())
            .filter(|t| t.first().and_then(|s| s.as_str()) == Some("vp"))
            .filter_map(|t| t.get(1).and_then(|s| s.as_str()))
            .next();
        match (mark, author_mark(&author_key(&self.keys), raw.pubkey.as_hex())) {
            (Some(m), Some(mine)) => m == mine,
            _ => false,
        }
    }

    async fn dm_notice(&self, dm: messenger_core::DmInbound, settings: &Settings, count: u32) -> Result<Outcome> {
        // A note between my devices or from a peer (a receipt, a reaction)
        // is for the app, whatever its type says.
        if dm.rumor_kind != KIND_DM_RUMOR {
            return Ok(Outcome::Quiet { reason: Reason::NotAMessage });
        }
        if dm.sender == self.me {
            return Ok(Outcome::Quiet { reason: Reason::Own });
        }
        let peer = dm.sender.clone();
        let chat_id = chats::dm_chat_id(peer.as_hex());
        // Other NIP-17 clients send bare text; read it as a text message.
        let envelope = Envelope::parse(&dm.content).unwrap_or_else(|_| Envelope::text(&dm.content));

        // What the app would do with this message, decided from the same rows.
        // A copy of something already stored was let in when it came.
        let relation = self.dm.load_relation(&peer).await?;
        if relation.blocked {
            return Ok(Outcome::Quiet { reason: Reason::Blocked });
        }
        if envelope.t == T_CALL_INVITE {
            return self.call_notice(&peer, &envelope, dm.created_at.0, settings).await;
        }
        let stored = messages::get(&self.store, dm.rumor_id.as_hex()).await?.is_some();
        let seen = messages::count_visible_incoming(&self.store, &chat_id).await?;
        let kind = match inbound_decision(&relation, !stored, seen) {
            InboundDecision::Save => ChatKind::Dm,
            InboundDecision::SaveAsRequest => ChatKind::Request,
            InboundDecision::Drop(_) => return Ok(Outcome::Quiet { reason: Reason::NotForMe }),
        };

        let (body, chat) = match envelope.t.as_str() {
            T_TEXT | T_MEDIA => (body_of(&envelope), Some(chat_id.clone())),
            // A card the app would drop is nothing to tell of.
            T_CONTACT if card_dropped(&envelope, &peer) => return Ok(Outcome::Quiet { reason: Reason::Invalid }),
            T_CONTACT => (body_of(&envelope), Some(chat_id.clone())),
            T_EDIT | T_DELETE | T_CONTROL => return Ok(Outcome::Quiet { reason: Reason::NotAMessage }),
            T_INVITE => {
                let invite: wire::Invite = wire::dm_body(&envelope)?;
                (Some(Body::Invite { group_name: invite.name }), None)
            }
            T_JOIN_REQUEST => {
                let request: wire::JoinRequest = wire::dm_body(&envelope)?;
                let Some(group) = groups::get(&self.store, &request.group_id).await? else {
                    return Ok(Outcome::Quiet { reason: Reason::NotForMe });
                };
                (Some(Body::JoinRequest { group_name: group.name }), Some(groups::group_chat_id(&request.group_id)))
            }
            T_WELCOME => {
                let welcome: wire::Welcome = wire::dm_body(&envelope)?;
                let Some(group) = groups::get(&self.store, &welcome.group_id).await? else {
                    return Ok(Outcome::Quiet { reason: Reason::NotForMe });
                };
                (Some(Body::Welcome { group_name: group.name }), Some(groups::group_chat_id(&welcome.group_id)))
            }
            other if other.starts_with("group.") => return Ok(Outcome::Quiet { reason: Reason::NotAMessage }),
            // Types from a newer app: shown by the sender's name, the app will say more.
            _ => (body_of(&envelope), Some(chat_id.clone())),
        };

        let (title, picture, muted) = match self.dm.chat(&chat_id).await? {
            Some(view) => (view.title, view.picture, view.is_muted),
            None => {
                let (name, picture) = self.contacts.face_of(&peer).await?;
                (name, picture, false)
            }
        };
        Ok(Outcome::Show(Notice {
            kind,
            chat,
            sender: title.clone(),
            title,
            sender_key: peer.as_hex().to_string(),
            picture: https(picture),
            body: body.filter(|_| settings.content == Content::SenderText),
            muted,
            hide_on_lockscreen: settings.lockscreen_hidden,
            count,
        }))
    }

    /// Somebody calls (`call.invite`). The same gate as the app's: only a
    /// peer we both chose to talk with may ring this phone, as a ring tells
    /// that the device is alive. A new offer inside a call (`restart`) is a
    /// signal for the app. What the phone rings with is who calls, by the
    /// chat's name (nobody under "no content"); the app shows the rest.
    async fn call_notice(&self, peer: &PubKey, envelope: &Envelope, created_at: i64, settings: &Settings) -> Result<Outcome> {
        if envelope.fields.get("restart").and_then(serde_json::Value::as_bool).unwrap_or(false) {
            return Ok(Outcome::Quiet { reason: Reason::NotAMessage });
        }
        let call_id = envelope.str_field("call_id").filter(|id| is_call_id(id));
        let media = envelope.str_field("media").filter(|m| matches!(*m, "audio" | "video"));
        let (Some(call_id), Some(media)) = (call_id, media) else {
            return Ok(Outcome::Quiet { reason: Reason::Invalid });
        };
        if !self.dm.calls_allowed(peer).await? {
            return Ok(Outcome::Quiet { reason: Reason::NotForMe });
        }
        let chat_id = chats::dm_chat_id(peer.as_hex());
        let (name, picture) = if settings.content == Content::None {
            (String::new(), None)
        } else {
            match self.dm.chat(&chat_id).await? {
                Some(view) => (view.title, view.picture),
                None => self.contacts.face_of(peer).await?,
            }
        };
        let (created_at, expires_at) = invite_times(created_at, SystemClock.now().secs());
        Ok(Outcome::Call(CallNotice {
            call_id: call_id.to_string(),
            media: media.to_string(),
            name,
            peer_key: peer.as_hex().to_string(),
            picture: https(picture),
            created_at,
            expires_at,
            hide_on_lockscreen: settings.lockscreen_hidden,
        }))
    }

    async fn group_notice(
        &self,
        group_id: &str,
        key: &messenger_groups::GroupKey,
        ciphertext: &str,
        settings: &Settings,
        count: u32,
    ) -> Result<Outcome> {
        let Some(group) = groups::get(&self.store, group_id).await? else {
            return Ok(Outcome::Quiet { reason: Reason::NotForMe });
        };
        if group.membership != MEMBERSHIP_JOINED {
            return Ok(Outcome::Quiet { reason: Reason::NotForMe });
        }
        // The key is one this phone has, and what came does not open with
        // it or is not signed by its author: not a message of the group,
        // whoever sent it. Nothing is shown for it.
        let message = match wire::open(group_id, key, ciphertext) {
            Ok(Opened::Message(m)) => m,
            Ok(Opened::Op { .. } | Opened::Chain(_)) => return Ok(Outcome::Quiet { reason: Reason::NotAMessage }),
            Err(_) => return Ok(Outcome::Quiet { reason: Reason::Invalid }),
        };
        if message.author == self.me {
            return Ok(Outcome::Quiet { reason: Reason::Own });
        }
        let envelope = Envelope::parse(&message.content).unwrap_or_else(|_| Envelope::text(&message.content));
        let body = match envelope.t.as_str() {
            T_TEXT | T_MEDIA => body_of(&envelope),
            T_CONTACT if card_dropped(&envelope, &message.author) => return Ok(Outcome::Quiet { reason: Reason::Invalid }),
            T_CONTACT => body_of(&envelope),
            T_EDIT | T_DELETE => return Ok(Outcome::Quiet { reason: Reason::NotAMessage }),
            _ => return Ok(Outcome::Quiet { reason: Reason::NotAMessage }),
        };
        let chat = groups::group_chat_id(group_id);
        let (sender, picture) = self.contacts.face_of(&message.author).await?;
        Ok(Outcome::Show(Notice {
            kind: ChatKind::Group,
            chat: Some(chat.clone()),
            title: group.name,
            sender,
            sender_key: message.author.as_hex().to_string(),
            picture: https(picture),
            body: body.filter(|_| settings.content == Content::SenderText),
            muted: chats::is_muted(&self.store, &chat).await?,
            hide_on_lockscreen: settings.lockscreen_hidden,
            count,
        }))
    }
}

/// How long an invitation to a call is good for, from the time inside its
/// rumor: `messenger_calls::signal::INVITE_TTL_SECS`, spelled here again
/// because this crate stays free of the core of calls (a test of this
/// crate, `tests/describe.rs`, keeps the two the same).
pub const CALL_INVITE_TTL_SECS: i64 = 45;

/// When the invitation was made and until when it is good, as the core
/// judges them (`messenger_calls::service`, on an invite): the time inside
/// the rumor, and never later than `now`, so that a clock ahead of mine
/// makes no call of the future and no longer ring.
pub fn invite_times(created_at: i64, now: i64) -> (i64, i64) {
    let at = created_at.min(now);
    (at, at.saturating_add(CALL_INVITE_TTL_SECS))
}

/// A call id as the core of calls makes it: 16 random bytes, hex.
fn is_call_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Whether the app drops this card when it stores the message
/// (`messenger_dm::cards::received_card`), as it does a card without a
/// valid key.
fn card_dropped(envelope: &Envelope, author: &PubKey) -> bool {
    messenger_dm::cards::received_card(envelope, author.as_hex()).is_none()
}

/// A signed event as JSON, checked as far as its shape goes; the
/// signature is checked where the event is classified.
fn raw_event(json: serde_json::Value, relay: Option<&str>) -> Result<RawEvent> {
    let bad = |what: &str| MessengerError::Invalid(format!("event: {what}"));
    let id = json["id"].as_str().and_then(messenger_core::EventId::parse).ok_or_else(|| bad("id"))?;
    let pubkey = json["pubkey"].as_str().and_then(PubKey::parse).ok_or_else(|| bad("pubkey"))?;
    let kind = json["kind"].as_u64().and_then(|k| u16::try_from(k).ok()).ok_or_else(|| bad("kind"))?;
    let created_at = json["created_at"].as_i64().ok_or_else(|| bad("created_at"))?;
    let url = relay.and_then(RelayUrl::parse).unwrap_or_else(|| RelayUrl::parse("wss://push.invalid").expect("a fixed url"));
    Ok(RawEvent { id, kind, pubkey, created_at: Timestamp(created_at), json, source: EventSource::Relay { url } })
}

/// What can be said before the event is opened, or when it cannot be.
async fn plain(store: &Store, push: &PushData) -> Result<Plain> {
    let (kind, chat, title, muted) = match (push.kind, &push.group_id) {
        (PushKind::Group, Some(id)) => {
            let chat = groups::group_chat_id(id);
            let group = groups::get(store, id).await?;
            let muted = chats::is_muted(store, &chat).await?;
            (ChatKind::Group, Some(chat), group.map(|g| g.name), muted)
        }
        _ => (ChatKind::Dm, None, None, false),
    };
    Ok(Plain { kind, chat, title, muted, count: push.count })
}

/// Pictures come from profiles anyone can write: only https addresses are
/// passed on, as the app itself shows no others.
pub(crate) fn https(picture: Option<String>) -> Option<String> {
    picture.filter(|p| p.starts_with("https://") && p.len() <= 2048)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An invitation from a clock ahead of mine is good for 45 s from
    /// now, not from its time: the phone would otherwise ring on from a
    /// push for as long as the limit of a ringing allows, while the app
    /// treats the same invitation as 45 s old at most.
    #[test]
    fn an_invitation_from_the_future_is_good_for_its_life_from_now() {
        assert_eq!(invite_times(1_000, 1_010), (1_000, 1_045));
        assert_eq!(invite_times(1_600, 1_010), (1_010, 1_055));
        assert_eq!(invite_times(i64::MAX, 1_010), (1_010, 1_055));
    }
}

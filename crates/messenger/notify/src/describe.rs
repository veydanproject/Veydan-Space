// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! From a push to a notice: the event opened, the chat looked up, the
//! rules of the chat applied. Everything the app would decide about the
//! message is decided the same way here, from the same rows, only
//! nothing is written to them: the app writes when it runs. What is shown
//! is left for it as it came (`messenger_ingress::push_inbox`), so that the
//! chat the notification opens holds the message before a relay answers.

use crate::bundle::KeyBundle;
use crate::fetch;
use crate::notice::{Body, CallEnd, CallNotice, ChatKind, Notice, Outcome, Plain, Reason};
use crate::push::{PushData, PushKind};
use crate::settings::{Content, Settings};
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::envelope::{
    KIND_PEER_NOTE_RUMOR, T_CALL_ANSWER, T_CALL_BUSY, T_CALL_DECLINE, T_CALL_END, T_CALL_INVITE, T_CONTACT, T_CONTROL, T_DELETE, T_EDIT, T_MEDIA,
    T_TEXT,
};
use messenger_core::traits::{Clock, SystemClock};
use messenger_core::{Envelope, EventSource, Inbound, MessengerConfig, MessengerError, PubKey, RawEvent, RelayUrl, Result, Timestamp};
use messenger_dm::pushtags::{author_key, author_mark};
use messenger_dm::relationship::{inbound_decision, screen_mode, InboundDecision, InboundKind, ScreenMode};
use messenger_dm::body::body_of;
use messenger_dm::DmService;
use messenger_groups::wire::{self, Opened, T_INVITE, T_JOIN_REQUEST, T_WELCOME};
use messenger_groups::service::{invite_standing_in, InviteStanding, MEMBERSHIP_JOINED};
use messenger_ingress::classify::KIND_DM_RUMOR;
use messenger_ingress::push_inbox;
use messenger_store::{banned, calls, chats, groups, messages, Store};
use nostr::key::Keys;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Says what the push is about. `data_dir` is the messenger's data
/// directory, the one with `messenger.db` in it.
/// Without keys (the app has a lock, or the settings want nothing said)
/// only what the database knows of the chat is told: the group's name,
/// whether it is muted. A push the server marked as a call is the
/// exception: without keys it says nothing at all ([`Reason::NoKeys`]),
/// since "something came" is no call, and a call rings now or never.
pub async fn describe(data_dir: &Path, bundle: Option<&KeyBundle>, push: &PushData) -> Result<Outcome> {
    describe_as(&MessengerConfig::new(data_dir), bundle, push).await
}

/// [`describe`] with the app's own configuration: its data directory and
/// the pinned key of the bot of Veydan Cloud (`bot_pubkey`), whose messages
/// are no request and whose cards are told without a word of theirs, as
/// the app takes them (`DmService::with_bot`).
pub async fn describe_as(config: &MessengerConfig, bundle: Option<&KeyBundle>, push: &PushData) -> Result<Outcome> {
    let store = Store::open_read_only(config).await?;
    let out = match bundle {
        Some(bundle) => match bundle.keys() {
            Ok(keys) => Describe::new(store.clone(), keys, config.bot_pubkey.as_deref(), config.push_inbox_dir()).run(bundle, push).await,
            Err(e) => Err(e),
        },
        None if push.call => Ok(Outcome::Quiet { reason: Reason::NoKeys }),
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
    /// Where what is shown is left for the app.
    inbox: PathBuf,
}

impl Describe {
    fn new(store: Store, keys: Keys, bot: Option<&str>, inbox: PathBuf) -> Self {
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts.clone(), profiles, Arc::new(SystemClock)).with_bot(bot);
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a public key is 64 hex chars");
        Self { store, keys, me, contacts, dm, inbox }
    }

    async fn run(&self, bundle: &KeyBundle, push: &PushData) -> Result<Outcome> {
        let settings = Settings::load(&self.store).await?;
        // A push of a group the network closes says nothing, under any
        // settings: the app takes nothing of the group in.
        if let Some(group_id) = &push.group_id {
            if banned::group_contains(&self.store, group_id).await? {
                return Ok(Outcome::Quiet { reason: Reason::Blocked });
            }
        }
        let mut plain = plain(&self.store, push).await?;
        if settings.content == Content::None || bundle.calls_only {
            // Nothing of a message is said, so no message is opened. A
            // call is the one thing that is: a ring is not a word of the
            // content, and a phone that says "only that something came"
            // must still ring (and stop ringing). The push says whether
            // it carries a call (anyone can say so on the outside, so the
            // wrap is opened and the call rings from a mutual contact
            // only, with no name). A push so marked never says "something
            // came": where a ring was due, that is the notification of
            // nothing. What opens and is no call and no end of one is
            // quiet (`NotACall`), what cannot be had is quiet
            // (`Unreachable`), and what cannot be read is an error, as
            // it is with the content shown.
            if push.call {
                let outcome = self.call_only(&settings, push).await?;
                return Ok(outcome.unwrap_or(Outcome::Quiet { reason: Reason::NotACall }));
            }
            return Ok(Outcome::Plain(plain));
        }

        let Some(raw) = self.event_of(push).await? else {
            if push.call {
                return Ok(Outcome::Quiet { reason: Reason::Unreachable });
            }
            return Ok(Outcome::Plain(plain));
        };
        if raw.kind == messenger_ingress::classify::KIND_GROUP_MESSAGE && self.written_by_me(&raw) {
            return Ok(Outcome::Quiet { reason: Reason::Own });
        }

        let outcome = match messenger_ingress::classify(&raw, Some(&self.keys)) {
            Inbound::Dm(dm) => self.dm_notice(dm, &settings, push.count).await?,
            // The event may name another group than the push did.
            Inbound::Group(g) if banned::group_contains(&self.store, &g.group_id).await? => Outcome::Quiet { reason: Reason::Blocked },
            Inbound::Group(g) => match bundle.group_key(&g.group_id, g.key_id.as_deref().unwrap_or_default()) {
                Some(key) => self.group_notice(&g.group_id, &key, &g.ciphertext, &settings, push.count).await?,
                // A key this device does not have yet: the app will, when it runs.
                None => {
                    plain.chat = Some(groups::group_chat_id(&g.group_id));
                    Outcome::Plain(plain)
                }
            },
            _ => Outcome::Quiet { reason: Reason::Invalid },
        };
        // The message the notification shows waits in the chat it opens,
        // before the app reaches a relay. A message that stays quiet is
        // the relay's to bring: nothing was promised of it.
        if matches!(outcome, Outcome::Show(_)) {
            if let Err(e) = push_inbox::keep(&self.inbox, &raw) {
                eprintln!("messenger notify: the event is not left for the app: {e}");
            }
        }
        Ok(outcome)
    }


    /// The push opened for a call and nothing else: the invitation of a
    /// mutual contact rings (`Some`, with no name and no picture under
    /// the settings that brought us here), the end of a call stops the
    /// ringing; any other content is left unread (`None`).
    async fn call_only(&self, settings: &Settings, push: &PushData) -> Result<Option<Outcome>> {
        let Some(raw) = self.event_of(push).await? else {
            return Ok(Some(Outcome::Quiet { reason: Reason::Unreachable }));
        };
        let Inbound::Dm(dm) = messenger_ingress::classify(&raw, Some(&self.keys)) else { return Ok(None) };
        if dm.rumor_kind == KIND_PEER_NOTE_RUMOR {
            return self.call_signal(&dm).await.map(Some);
        }
        if dm.rumor_kind != KIND_DM_RUMOR || dm.sender == self.me {
            return Ok(None);
        }
        let Ok(envelope) = Envelope::parse(&dm.content) else { return Ok(None) };
        if envelope.t != T_CALL_INVITE {
            return Ok(None);
        }
        if self.dm.load_relation(&dm.sender).await?.blocked || self.dm.is_banned(&dm.sender).await? {
            return Ok(Some(Outcome::Quiet { reason: Reason::Blocked }));
        }
        // No name and no picture, as nothing of the content is shown.
        let settings = Settings { content: Content::None, ..*settings };
        self.call_notice(&dm.sender, &envelope, dm.created_at.0, &settings).await.map(Some)
    }

    /// A note of a call (kind 30079) that reached the phone by push: the
    /// ones that end a ringing (see [`CallEnd`]) say so; every other
    /// signal of a call, and every other note, is the app's.
    async fn call_signal(&self, dm: &messenger_core::DmInbound) -> Result<Outcome> {
        let quiet = Ok(Outcome::Quiet { reason: Reason::NotAMessage });
        let Ok(envelope) = Envelope::parse(&dm.content) else { return quiet };
        let Some(call_id) = envelope.str_field("call_id").filter(|id| is_call_id(id)) else { return quiet };
        let ends = if dm.sender == self.me {
            // My other device took, declined or ended the call, or was
            // busy with another: this phone stops ringing for it.
            matches!(envelope.t.as_str(), T_CALL_ANSWER | T_CALL_DECLINE | T_CALL_BUSY | T_CALL_END)
        } else {
            // The caller gave up before I answered. The caller's other
            // words (an answer, a refusal) are for the app that calls.
            envelope.t == T_CALL_END && self.dm.calls_allowed(&dm.sender).await?
        };
        if !ends {
            return quiet;
        }
        Ok(Outcome::CallEnd(CallEnd { call_id: call_id.to_string() }))
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
        // is for the app, whatever its type says; the one that ends a
        // ringing is for the phone.
        if dm.rumor_kind == KIND_PEER_NOTE_RUMOR {
            return self.call_signal(&dm).await;
        }
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
        // A copy of something stored and shown was let in when it came; a row
        // stored hidden (a request held back until the old episode ends, or
        // one removed for me elsewhere) was never shown, so nothing is told.
        let relation = self.dm.load_relation(&peer).await?;
        // A key the network bans (`DmService::is_banned`) reads as blocked:
        // the app drops what it sends, so the phone tells nothing of it.
        if relation.blocked || self.dm.is_banned(&peer).await? {
            return Ok(Outcome::Quiet { reason: Reason::Blocked });
        }
        if envelope.t == T_CALL_INVITE {
            return self.call_notice(&peer, &envelope, dm.created_at.0, settings).await;
        }
        let row = messages::get(&self.store, dm.rumor_id.as_hex()).await?;
        if row.as_ref().is_some_and(|r| r.is_hidden) {
            return Ok(Outcome::Quiet { reason: Reason::NotForMe });
        }
        let stored = row.is_some();
        // The pinned bot of Veydan Cloud is approved beforehand: what it
        // writes is a message of its chat, never a request, and its block
        // was looked at above. Its signals are not heard.
        let from_bot = self.dm.is_bot(&peer);
        // The protocol of groups is no message of the chat: the app hands it
        // to the groups before the gate of requests (`GroupService::on_dm`,
        // the block alone, looked at above), and so does this.
        let group = envelope.t.starts_with("group.");
        // The protocol of a group the network closes is not heard by the
        // app (`GroupService::on_dm`): no invitation, request or welcome
        // of it is told here either.
        if group {
            if let Some(group_id) = envelope.str_field("group_id") {
                if banned::group_contains(&self.store, group_id).await? {
                    return Ok(Outcome::Quiet { reason: Reason::Blocked });
                }
            }
        }
        if !(from_bot || group) {
            // The same gate as the app's, from the same facts: what the
            // message is (a short plain text without a link, or not), whom
            // I take messages from, and what came before it. A stranger's
            // first word is a request: the app puts it in the queue and
            // raises no notification for it, so neither does the phone; a
            // copy of a request already stored is the same request.
            let seen = messages::count_visible_incoming_since(&self.store, &chat_id, relation.request_floor).await?;
            let kind = match envelope.t.as_str() {
                T_TEXT => DmService::kind_of(messages::CT_TEXT, envelope.as_text(), envelope.format()),
                _ => InboundKind::Other,
            };
            let dm_from = self.dm.dm_settings().await?.dm_from;
            let dm_from = messenger_dm::DmFrom::parse(&dm_from).unwrap_or_default();
            match inbound_decision(&relation, !stored, seen, dm.created_at.0, kind, dm_from) {
                InboundDecision::Save if screen_mode(&relation) == ScreenMode::RequestReceived => {
                    return Ok(Outcome::Quiet { reason: Reason::Request })
                }
                // A stored text of somebody who is not my contact (one I
                // declined, one whose chat I deleted): the app told of it
                // when it came, if at all, and tells nothing of a copy.
                InboundDecision::Save if stored && relation.my_contact != messenger_dm::relationship::MyContact::Approved => {
                    return Ok(Outcome::Quiet { reason: Reason::NotForMe })
                }
                InboundDecision::Save => {}
                InboundDecision::SaveAsRequest => return Ok(Outcome::Quiet { reason: Reason::Request }),
                InboundDecision::Drop(_) => return Ok(Outcome::Quiet { reason: Reason::NotForMe }),
            }
        }
        let kind = ChatKind::Dm;

        let (body, chat) = match envelope.t.as_str() {
            T_TEXT | T_MEDIA => (body_of(&envelope), Some(chat_id.clone())),
            // A card the app would drop is nothing to tell of.
            T_CONTACT if card_dropped(&envelope, &peer) => return Ok(Outcome::Quiet { reason: Reason::Invalid }),
            T_CONTACT => (body_of(&envelope), Some(chat_id.clone())),
            T_EDIT | T_DELETE | T_CONTROL => return Ok(Outcome::Quiet { reason: Reason::NotAMessage }),
            T_INVITE => {
                // As the app tells of it (`GroupService::on_invite`, one rule:
                // `invite_standing`): a well-formed invitation from a manager
                // by the log this device holds, or from my contact.
                let invite: wire::Invite = wire::dm_body(&envelope)?;
                if invite.check(SystemClock.now().secs()).is_err() {
                    return Ok(Outcome::Quiet { reason: Reason::Invalid });
                }
                let contact = relation.my_contact == messenger_dm::relationship::MyContact::Approved;
                if invite_standing_in(&self.store, &invite.group_id, &peer, contact).await? != InviteStanding::Vouched {
                    return Ok(Outcome::Quiet { reason: Reason::NotForMe });
                }
                (Some(Body::Invite { group_name: invite.shown_name() }), None)
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
            // A card of the bot: the app words it by its type where it is
            // shown; nothing of its envelope is told here.
            _ if from_bot => (None, Some(chat_id.clone())),
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
    /// signal for the app. An invitation older than its life is a missed
    /// call (the app's to show), and a call the app has on record as
    /// answered or over (it was up, or caught up since) rings nothing
    /// either: a push comes late as often as not, and a ring for a call
    /// that is over is the "empty" notification of a phone. What the phone
    /// rings with is who calls, by the chat's name (nobody under "no
    /// content"); the app shows the rest.
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
        let now = SystemClock.now().secs();
        let (created_at, expires_at) = invite_times(created_at, now);
        if invite_expired(expires_at, now) {
            return Ok(Outcome::Quiet { reason: Reason::Expired });
        }
        if calls::get(&self.store, call_id).await?.is_some_and(|row| call_over(&row)) {
            return Ok(Outcome::Quiet { reason: Reason::Over });
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
        // What a key the network bans writes in a group is hidden by the
        // app and wakes nobody; neither does the phone.
        if self.dm.is_banned(&message.author).await? {
            return Ok(Outcome::Quiet { reason: Reason::Blocked });
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

/// Whether an invitation good until `expires_at` is past at `now`: the
/// core rings while `now - created_at <= INVITE_TTL_SECS`
/// (`messenger_calls::service`), so the last second is still a ring.
pub fn invite_expired(expires_at: i64, now: i64) -> bool {
    expires_at < now
}

/// Whether the app's record of a call says it is answered or over: it was
/// taken (here or on another device), declined, missed, busy, ended or
/// failed. Open, with nothing of the kind, while the app rings for it or
/// never heard of its end.
pub fn call_over(row: &calls::CallRow) -> bool {
    row.answered_at.is_some() || row.ended_at.is_some() || row.outcome.is_some()
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

    /// The last second of an invitation still rings, as in the core; the
    /// one after is a missed call.
    #[test]
    fn an_invitation_expires_after_its_last_second() {
        let (_, expires_at) = invite_times(1_000, 1_000);
        assert!(!invite_expired(expires_at, 1_045));
        assert!(invite_expired(expires_at, 1_046));
    }

    #[test]
    fn a_call_on_record_is_over_once_answered_ended_or_judged() {
        let open = calls::CallRow {
            call_id: "c".into(),
            chat_id: "dm:x".into(),
            peer: "x".into(),
            direction: "in".into(),
            media: "audio".into(),
            started_at: 1,
            answered_at: None,
            ended_at: None,
            outcome: None,
            via_relay: false,
        };
        assert!(!call_over(&open));
        assert!(call_over(&calls::CallRow { answered_at: Some(2), ..open.clone() }));
        assert!(call_over(&calls::CallRow { ended_at: Some(2), ..open.clone() }));
        assert!(call_over(&calls::CallRow { outcome: Some("missed".into()), ..open.clone() }));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The facade of moderation, as the page and the command line call it:
//! the queue of requests of strangers and whom I take messages from, the
//! moderation of groups, the invitations. One place, so that every host
//! gets the same words and the same pages (the types are the contract,
//! and the page is written against them).
//!
//! Nothing is decided here: the rules of a request are the dm crate's
//! (`messenger_dm::requests`, `relationship`), and an answer to a request
//! takes the road of every relationship action (`dm_act`), so the signals,
//! the address book and presence follow as they do from the chat. The
//! marks of a group are the groups crate's (`messenger_groups::marks`,
//! `moderation`): here they are only named, locked and carried out. An
//! invitation is the dm crate's too (`messenger_dm::invites`): here are
//! its presets (a link, a code, a QR: the secret, the term, the uses) and
//! the link it is handed out as. What this module adds is the paging, the
//! keeping of the queue by the session (`sweep_loop`: once the history
//! caught up, then every hour) and the forwarding of what a sweep decided
//! to the page.

use crate::session::UI_EVENT_HISTORY_SYNCED;
use crate::MessengerRuntime;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use messenger_contacts::book::parse_key;
use messenger_core::traits::{SystemClock, UiEvent};
use messenger_core::{Clock, Effect, MessengerError, PubKey, Result};
use messenger_dm::relationship::MyContact;
use messenger_dm::{Action, DmService, Invite, InviteKind, InviteNew, MessageView};
use messenger_groups::{ModNote, OpBody, PostMode, Premod, Schedule as GroupSchedule};
use messenger_ingress::Outbox;
use messenger_links::contact::npub_of;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;
use tokio::sync::broadcast;
use ts_rs::TS;

pub use messenger_dm::{DmSettings, RequestView, UI_EVENT_DM_INVITE_PAUSED, UI_EVENT_DM_REQUEST};
pub use messenger_groups::{UI_EVENT_GROUP_MODERATION, UI_EVENT_GROUP_QUEUE};

/// The queue is kept this often while a session runs.
pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(3600);
/// The first sweep of a session waits for the history to catch up (an
/// answer of my other device may be in it), but not longer than this: a
/// request that ran out is out, relays or no relays.
pub(crate) const SWEEP_WAIT_FOR_HISTORY: Duration = Duration::from_secs(120);

/// The presets of invitations (plan 1.6): a link is for one person for a
/// week, a code for one person for a day, a QR for anybody while it is
/// on the screen (a quarter of an hour, made anew while shown), or for
/// good when asked for neither a term nor a number.
pub const LINK_TTL_SECS: i64 = 7 * 86_400;
pub const CODE_TTL_SECS: i64 = 86_400;
pub const QR_TTL_SECS: i64 = 15 * 60;
/// A code is this many digits.
pub const CODE_DIGITS: usize = 6;

// ─── Requests ───────────────────────────────────────────────────────────────

/// One page of the queue of requests, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RequestsPage {
    pub items: Vec<RequestView>,
    /// What to pass as `before` for the next page; absent on the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub next_before: Option<i64>,
    /// How many wait in all.
    pub total: u32,
}

/// What I do with a request: let the stranger in, say no, or shut the
/// key. Each is a signal to the peer, as from the banner of the chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RequestAction {
    Accept,
    Decline,
    Block,
}

// ─── Groups: the queue, the actions, the log, the settings ──────────────────

/// A message waiting in the queue of a group, or hidden there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct QueueItem {
    pub message_id: String,
    pub group_id: String,
    /// Hex pubkey of who wrote it.
    pub author: String,
    pub text: String,
    #[ts(type = "number")]
    pub at: i64,
    /// Which version of the message the queue holds: an edit makes a new one.
    pub version: String,
    /// How many more of the same author wait behind it.
    pub more_from_author: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct QueuePage {
    pub items: Vec<QueueItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub next_before: Option<i64>,
    pub total: u32,
}

/// What of the queue to show; every field left out means "any".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct QueueFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub since: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub until: Option<i64>,
    /// A part of the text or of the author's name, or their key (npub or hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
    /// Every message its own line, not one line per author: what one author
    /// wrote, with `author`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub each: Option<bool>,
}

/// One act of moderation in a group. On the wire it is tagged by `t`, as
/// every sum of the messenger is (`Body`, `BotCard`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ModAction {
    Hide {
        targets: Vec<String>,
    },
    HideAuthor {
        author: String,
        #[ts(type = "number")]
        from: i64,
        #[ts(type = "number")]
        to: i64,
    },
    Unhide {
        targets: Vec<String>,
    },
    Approve {
        target: String,
        version: String,
    },
    Reject {
        target: String,
    },
    /// Ban the author and hide what they wrote; `target` is the message
    /// the decision was taken on, when there is one.
    Spam {
        author: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        target: Option<String>,
    },
    /// `until` absent: for good.
    Mute {
        who: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        until: Option<i64>,
    },
    Unmute {
        who: String,
    },
    Trust {
        who: String,
        trusted: bool,
    },
}

/// One line of the log of moderation of a group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LogEntry {
    #[ts(type = "number")]
    pub at: i64,
    /// Hex pubkey of the manager who acted.
    pub by: String,
    pub action: String,
    pub target: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LogPage {
    pub items: Vec<LogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub next_before: Option<i64>,
}

/// When members may post: `days` are 1 (Monday) to 7, `from` and `to`
/// are `HH:MM` in `tz`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Schedule {
    pub tz: String,
    pub days: Vec<u8>,
    pub from: String,
    pub to: String,
}

/// The settings of moderation of a group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct GroupModSettings {
    /// `all` | `managers`
    pub post_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub schedule: Option<Schedule>,
    /// `off` | `newcomers` | `all`
    pub premod: String,
}

// ─── Invitations ────────────────────────────────────────────────────────────

/// What an invitation is to be, as the page asks for one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InvitePreset {
    /// `link` | `code` | `qr`
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_uses: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub ttl_secs: Option<i64>,
    /// For one person only, hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub for_pubkey: Option<String>,
    pub label: String,
}

/// An invitation as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InviteView {
    pub id: String,
    pub kind: String,
    pub secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub link: Option<String>,
    pub label: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_uses: Option<u32>,
    pub used: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub for_pubkey: Option<String>,
    pub revoked: bool,
}

/// The state of the invitations on this device: paused when somebody
/// guessed too often (ten wrong secrets within an hour), until then.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InviteStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub paused_until: Option<i64>,
}

/// Somebody I block, as the list shows them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BlockedView {
    /// Hex pubkey.
    pub peer: String,
    pub npub: String,
    /// What the app calls them: a nickname, a profile name, a short npub.
    pub name: String,
    /// The network bans them (the manifest's `banned_keys`): the list shows
    /// it and offers no unblock, which the runtime would refuse anyway.
    #[serde(default)]
    pub banned: bool,
}

/// 16 random bytes as base64url without padding: the secret of a link
/// and of a QR.
fn link_secret() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Six digits, each of the million equally likely: 24 random bits are
/// taken again while they fall past the largest multiple of a million
/// below 2^24, so the remainder is not skewed. A leading zero is a digit
/// like any other.
fn code_secret() -> Result<String> {
    loop {
        let mut bytes = [0u8; 3];
        getrandom::fill(&mut bytes).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let v = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]);
        if v < 16_000_000 {
            return Ok(format!("{:0width$}", v % 1_000_000, width = CODE_DIGITS));
        }
    }
}

// ─── The facade ─────────────────────────────────────────────────────────────

impl MessengerRuntime {
    /// The queue of requests, newest first; `before` is the `next_before`
    /// of the page shown last. `limit` is kept within 1..=500.
    pub async fn requests(&self, before: Option<i64>, limit: u32) -> Result<RequestsPage> {
        let limit = limit.clamp(1, 500);
        let items = self.dm().requests(before, limit).await?;
        // A full page may have more behind it; the store pages by where it
        // places a request (`cursor`), which is not the time of its text.
        let next_before = items.last().filter(|_| items.len() == limit as usize).map(|r| r.cursor);
        let total = self.dm().requests_count().await?;
        Ok(RequestsPage { items, next_before, total })
    }

    /// Answer the request of `peer` (hex or npub). The same road as the
    /// banner of the chat takes (`dm_act`): the signal, the address book,
    /// presence and the events follow. Doing again what is done already
    /// changes nothing and sends nothing: the page may press twice, and
    /// an accept said again would go out again as a signal. A no to a
    /// request that ran out meanwhile is done too: an expiry tells the
    /// peer nothing, and the no would. `<key>@<server>` (`addr`) answers
    /// the request of the private chat on that server, by its relation.
    pub async fn request_act(&self, peer: &str, action: RequestAction) -> Result<()> {
        let chat = self.peer_addr(peer).await?;
        let r = self.dm().load_relation(chat).await?;
        let (action, done) = match action {
            RequestAction::Accept => (Action::Accept, r.my_contact == MyContact::Approved && !r.blocked),
            RequestAction::Decline => (Action::Decline, matches!(r.my_contact, MyContact::Declined | MyContact::Expired) && !r.blocked),
            RequestAction::Block => (Action::Block, r.blocked),
        };
        if done {
            return Ok(());
        }
        self.dm_act(peer, action).await?;
        Ok(())
    }

    pub async fn requests_count(&self) -> Result<u32> {
        self.dm().requests_count().await
    }

    pub async fn dm_settings(&self) -> Result<DmSettings> {
        self.dm().dm_settings().await
    }

    /// Whom I take messages from and how long a request waits. A shorter
    /// term applies at once: the queue is swept here, not at the next hour.
    pub async fn dm_settings_set(&self, s: DmSettings) -> Result<()> {
        self.dm().dm_settings_set(s).await?;
        let effects = self.dm().sweep_requests(SystemClock.now().secs()).await?;
        forward_effects(&self.outbox, &self.ui, effects).await
    }

    /// The messages of a chat as I see them: in a group I moderate, those
    /// that wait for a moderator too. What the page and the command line
    /// list a chat by.
    pub async fn messages(&self, chat_id: &str, before: Option<i64>, limit: i64) -> Result<Vec<MessageView>> {
        let moderator = match (chat_id.strip_prefix("group:"), self.session_pubkey().await) {
            (Some(group), Some(me)) => self.groups().is_moderator(group, &me).await?,
            _ => false,
        };
        self.dm().messages_as(chat_id, before, limit, moderator).await
    }

    /// What waits for a moderator in a group: one line per author (their
    /// oldest), or per message with `each`, newest first; `before` is the `next_before` of the page
    /// shown last. For moderators and above (`group_not_permitted`).
    pub async fn group_queue(&self, group: &str, filter: QueueFilter, before: Option<i64>, limit: u32) -> Result<QueuePage> {
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        let limit = limit.clamp(1, 500);
        let author = match &filter.author {
            Some(a) => Some(parse_key(a)?.as_hex().to_string()),
            None => None,
        };
        // A whole key searched for is its author, in hex as the rows hold it.
        let text = filter.text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(|t| parse_key(t).map(|k| k.as_hex().to_string()).unwrap_or_else(|_| t.to_string()));
        let f = messenger_store::messages::QueueFilter { author, since: filter.since, until: filter.until, text, each: filter.each.unwrap_or(false) };
        let (lines, total) = self.groups().queue(group, &me, &f, before, limit as i64).await?;
        let next_before = lines.last().filter(|_| lines.len() >= limit as usize).map(|l| l.row.created_at);
        let items = lines
            .into_iter()
            .map(|l| QueueItem {
                message_id: l.row.id,
                group_id: group.to_string(),
                author: l.row.sender_pubkey,
                text: l.row.text.unwrap_or_default(),
                at: l.row.created_at,
                version: l.version,
                more_from_author: l.more_from_author,
            })
            .collect();
        Ok(QueuePage { items, next_before, total: total.max(0) as u32 })
    }

    /// One act of moderation in a group: a note of mine (hide, unhide,
    /// approve, reject), an operation of the journal (mute, unmute,
    /// trust), or both (spam: a ban when I may give one, and a hide of
    /// everything the author wrote up to now). Doing what is done already
    /// sends nothing.
    pub async fn group_mod(&self, group: &str, action: ModAction) -> Result<()> {
        let keys = self.session_keys().await?;
        let me = PubKey::parse(&keys.public_key().to_hex()).ok_or(MessengerError::NotLoggedIn)?;
        let lock = self.groups().lock_of(group).await;
        let _guard = lock.lock().await;
        let now = SystemClock.now().secs();
        let note = match action {
            ModAction::Hide { targets } => ModNote::Hide { targets, reason: String::new() },
            ModAction::HideAuthor { author, from, to } => ModNote::HideAuthor { author: parse_key(&author)?, from, to, reason: String::new() },
            ModAction::Unhide { targets } => ModNote::Unhide { targets },
            ModAction::Approve { target, version } => ModNote::Approve { target, version },
            ModAction::Reject { target } => ModNote::Reject { target },
            ModAction::Spam { author, target } => {
                let author = parse_key(&author)?;
                let state = self.groups().state_of(group).await?;
                // The ban is a manager's; a moderator hides and leaves the ban
                // to one. Banned already: the hide alone.
                let may_ban = state.role_of(&me).is_some_and(|mine| messenger_groups::permits(mine, messenger_groups::Action::Ban(state.role_of(&author))));
                if !state.is_banned(&author) && may_ban {
                    let outcome = self.groups().act(&keys, group, OpBody::Ban { who: author.clone() }).await?;
                    self.group_driver.apply(outcome).await?;
                }
                // The span closes at the later of now and the message decided on: a clock ahead of mine wrote it.
                let mut to = now;
                if let Some(t) = target {
                    if let Some(m) = self.dm().message(&t).await? {
                        to = to.max(m.created_at);
                    }
                }
                ModNote::HideAuthor { author, from: 0, to, reason: messenger_groups::REASON_SPAM.into() }
            }
            ModAction::Mute { who, until } => {
                let who = parse_key(&who)?;
                let state = self.groups().state_of(group).await?;
                if state.member(&who).is_some_and(|m| m.muted_at(now) && m.muted_until == until) {
                    return Ok(());
                }
                return self.group_mod_op(&keys, group, OpBody::SetMuted { who, muted: true, until }).await;
            }
            ModAction::Unmute { who } => {
                let who = parse_key(&who)?;
                let state = self.groups().state_of(group).await?;
                if !state.member(&who).is_some_and(|m| m.muted_at(now)) {
                    return Ok(());
                }
                return self.group_mod_op(&keys, group, OpBody::SetMuted { who, muted: false, until: None }).await;
            }
            ModAction::Trust { who, trusted } => {
                let who = parse_key(&who)?;
                let state = self.groups().state_of(group).await?;
                if state.member(&who).is_some_and(|m| m.trusted == trusted) {
                    return Ok(());
                }
                return self.group_mod_op(&keys, group, OpBody::SetTrusted { who, trusted }).await;
            }
        };
        let outcome = self.groups().moderate(&keys, group, note).await?;
        self.group_driver.apply(outcome).await
    }

    /// An operation of the journal that is moderation, carried out under
    /// the lock of the group, which the caller holds; the groups crate
    /// gives the page the word of it.
    async fn group_mod_op(&self, keys: &nostr::key::Keys, group: &str, body: OpBody) -> Result<()> {
        let outcome = self.groups().act(keys, group, body).await?;
        self.group_driver.apply(outcome).await?;
        Ok(())
    }

    /// What was decided in a group, newest first: the marks and the
    /// operations of moderation of the journal, merged by time; `before`
    /// is the `next_before` of the page shown last. A full page takes
    /// along every further line of the same second as its last one (a
    /// ban and the hiding of the spam come in one second), so that the
    /// next page, strictly older, passes none of them by. For moderators
    /// and above.
    pub async fn group_log(&self, group: &str, before: Option<i64>, limit: u32) -> Result<LogPage> {
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        let limit = limit.clamp(1, 500) as usize;
        let mut items: Vec<LogEntry> = Vec::new();
        let mut more = false;
        for l in self.groups().mod_log(group, &me).await?.into_iter().filter(|l| before.is_none_or(|b| l.at < b)) {
            if items.len() >= limit && items.last().is_some_and(|last| last.at != l.at) {
                more = true;
                break;
            }
            items.push(LogEntry { at: l.at, by: l.by, action: l.action, target: l.target, detail: l.detail });
        }
        let next_before = items.last().filter(|_| more).map(|l| l.at);
        Ok(LogPage { items, next_before })
    }

    pub async fn group_mod_settings(&self, group: &str) -> Result<GroupModSettings> {
        let g = self.group_get(group).await?;
        Ok(GroupModSettings {
            post_mode: g.post_mode,
            schedule: g.schedule.map(|s| Schedule { tz: s.tz, days: s.days, from: s.from, to: s.to }),
            premod: g.premod,
        })
    }

    /// The rules of posting of a group, as an operation of its journal
    /// that carries only what changed; nothing goes out when nothing did.
    /// For managers (`group_not_permitted`); a wrong value is refused by
    /// its name (`post_mode`, `premod`, `post_schedule`).
    pub async fn group_mod_settings_set(&self, group: &str, s: GroupModSettings) -> Result<()> {
        let post_mode = match s.post_mode.as_str() {
            "all" => PostMode::All,
            "managers" => PostMode::Managers,
            _ => return Err(MessengerError::Invalid("post_mode".into())),
        };
        let premod = match s.premod.as_str() {
            "off" => Premod::Off,
            "newcomers" => Premod::Newcomers,
            "all" => Premod::All,
            _ => return Err(MessengerError::Invalid("premod".into())),
        };
        let schedule = s.schedule.map(|x| GroupSchedule { tz: x.tz, days: x.days, from: x.from, to: x.to });
        if let Some(sched) = &schedule {
            if sched.is_empty() || sched.check().is_err() {
                return Err(MessengerError::Invalid("post_schedule".into()));
            }
        }
        let state = self.groups().state_of(group).await?;
        let body = OpBody::EditSettings {
            name: None,
            about: None,
            picture: None,
            history_for_new: None,
            call_node: None,
            call_node_key: None,
            post_mode: Some(post_mode).filter(|p| *p != state.post_mode),
            // A schedule without days takes the schedule away.
            post_schedule: match (&schedule, &state.schedule) {
                (Some(new), Some(old)) if new == old => None,
                (Some(new), _) => Some(new.clone()),
                (None, Some(_)) => Some(GroupSchedule { tz: "UTC".into(), days: vec![], from: "00:00".into(), to: "23:59".into() }),
                (None, None) => None,
            },
            premod: Some(premod).filter(|p| *p != state.premod),
        };
        if let OpBody::EditSettings { post_mode: None, post_schedule: None, premod: None, .. } = &body {
            return Ok(());
        }
        self.group_act(group, body).await?;
        Ok(())
    }

    /// Say again, as me, what another moderator decided (`from`, hex or
    /// npub): for the admin who demotes one and keeps their decisions.
    /// Returns how many notes went out.
    pub async fn group_inherit(&self, group: &str, from: &str) -> Result<u32> {
        let keys = self.session_keys().await?;
        let from = parse_key(from)?;
        let lock = self.groups().lock_of(group).await;
        let _guard = lock.lock().await;
        let (n, outcome) = self.groups().inherit_marks(&keys, group, &from).await?;
        self.group_driver.apply(outcome).await?;
        Ok(n)
    }

    /// An invitation of mine, by its preset: `link` (one use, a week, a
    /// 16-byte secret in my link), `code` (one use, a day, six digits to
    /// say with my key), `qr` (any number, a quarter of an hour; with
    /// neither a term nor a number asked for, for good). A term or a
    /// number given overrides the preset; `for_pubkey` keeps it for one
    /// person. Refused: an unknown kind (`invite_kind`), a term or a
    /// number of zero (`invite_ttl`, `invite_max_uses`), my own key as
    /// the person (`invite_for`).
    pub async fn invite_create(&self, p: InvitePreset) -> Result<InviteView> {
        let made = self.invite_make(p).await?;
        self.invite_view(made).await
    }

    /// The invitation of `invite_create`, stored, not yet shown (the view
    /// writes my link, which may itself ask for one: `contact_link_on`).
    pub(crate) async fn invite_make(&self, p: InvitePreset) -> Result<Invite> {
        let kind = InviteKind::parse(&p.kind).ok_or_else(|| MessengerError::Invalid("invite_kind".into()))?;
        if p.ttl_secs.is_some_and(|t| t <= 0) {
            return Err(MessengerError::Invalid("invite_ttl".into()));
        }
        if p.max_uses.is_some_and(|m| m == 0) {
            return Err(MessengerError::Invalid("invite_max_uses".into()));
        }
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        let for_pubkey = match &p.for_pubkey {
            Some(f) => {
                let pk = parse_key(f)?;
                if pk == me {
                    return Err(MessengerError::Invalid("invite_for".into()));
                }
                Some(pk)
            }
            None => None,
        };
        let (secret, max_uses, ttl) = match kind {
            InviteKind::Link => (link_secret()?, p.max_uses.or(Some(1)), p.ttl_secs.or(Some(LINK_TTL_SECS))),
            InviteKind::Code => (code_secret()?, p.max_uses.or(Some(1)), p.ttl_secs.or(Some(CODE_TTL_SECS))),
            InviteKind::Qr => {
                let ttl = match (p.ttl_secs, p.max_uses) {
                    (None, None) => None,
                    (None, Some(_)) => Some(QR_TTL_SECS),
                    (ttl, _) => ttl,
                };
                (link_secret()?, p.max_uses, ttl)
            }
        };
        let now = SystemClock.now().secs();
        self.dm()
            .invite_create(InviteNew { secret, kind, label: p.label, expires_at: ttl.map(|t| now + t), max_uses, for_pubkey })
            .await
    }

    /// Every invitation of mine: the ones that still admit first.
    pub async fn invite_list(&self) -> Result<Vec<InviteView>> {
        let mut out = Vec::new();
        for i in self.dm().invite_list().await? {
            out.push(self.invite_view(i).await?);
        }
        Ok(out)
    }

    /// Revoke it here and, by the snapshot of my relations, on my other
    /// devices. One revoked already stays so; an unknown id is refused
    /// (`invite_unknown`).
    pub async fn invite_revoke(&self, id: &str) -> Result<()> {
        self.dm().invite_revoke(id).await
    }

    pub async fn invite_status(&self) -> Result<InviteStatus> {
        Ok(InviteStatus { paused_until: self.dm().invite_paused_until().await? })
    }

    /// The link of an invitation, as it is handed out: my contact link
    /// with the secret on it, for a link and a QR; `None` for a code.
    pub(crate) async fn invite_link(&self, id: &str) -> Result<Option<String>> {
        let i = self.dm().invite_get(id).await?.ok_or_else(|| MessengerError::Invalid("invite_unknown".into()))?;
        self.link_of_invite(&i).await
    }

    async fn link_of_invite(&self, i: &Invite) -> Result<Option<String>> {
        if InviteKind::parse(&i.row.kind) == Some(InviteKind::Code) {
            return Ok(None);
        }
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        Ok(Some(self.contact_link_with(&me, Some(&i.row.secret)).await?))
    }

    async fn invite_view(&self, i: Invite) -> Result<InviteView> {
        let link = self.link_of_invite(&i).await?;
        Ok(InviteView {
            id: i.row.id.clone(),
            kind: i.row.kind.clone(),
            secret: i.row.secret.clone(),
            link,
            label: i.row.label.clone(),
            created_at: i.row.created_at,
            expires_at: i.row.expires_at,
            max_uses: i.row.max_uses.and_then(|m| u32::try_from(m).ok()),
            used: i.used_by.len() as u32,
            for_pubkey: i.row.for_pubkey.clone(),
            revoked: i.row.revoked_at.is_some(),
        })
    }

    /// Whom I block, with the name the app calls them by and their npub:
    /// the page has no bech32 of its own.
    pub async fn blocked(&self) -> Result<Vec<BlockedView>> {
        let mut out = Vec::new();
        for hex in self.dm().blocked_peers().await? {
            let Some(pk) = PubKey::parse(&hex) else { continue };
            out.push(BlockedView {
                npub: npub_of(&pk).unwrap_or_default(),
                name: self.dm().display_name(&pk).await?,
                banned: self.dm().is_banned(&pk).await?,
                peer: hex,
            });
        }
        Ok(out)
    }
}

// ─── The sweep of the queue ─────────────────────────────────────────────────

/// What a service decided, carried out as the session's sink does it: a
/// wrap to the outbox (one pump for all of them), an event to the page, a
/// notice as the event `notify`.
pub(crate) async fn forward_effects(outbox: &Outbox, ui: &broadcast::Sender<UiEvent>, effects: Vec<Effect>) -> Result<()> {
    let mut sends = false;
    for e in effects {
        match e {
            Effect::Send(out) => {
                outbox.enqueue(out).await?;
                sends = true;
            }
            Effect::Emit(ev) => {
                let _ = ui.send(ev);
            }
            Effect::Notify(notice) => {
                let _ = ui.send(UiEvent { name: "notify".into(), payload: serde_json::to_value(notice).unwrap_or_default() });
            }
        }
    }
    if sends {
        outbox.kick();
    }
    Ok(())
}

/// One keeping of the queue, now.
async fn sweep_now(dm: &DmService, outbox: &Outbox, ui: &broadcast::Sender<UiEvent>) {
    match dm.sweep_requests(SystemClock.now().secs()).await {
        Ok(effects) => {
            if let Err(e) = forward_effects(outbox, ui, effects).await {
                eprintln!("messenger requests: what the sweep decided was not carried out: {e}");
            }
        }
        Err(e) => eprintln!("messenger requests: the queue was not swept: {e}"),
    }
}

/// The keeper of the queue of requests, as the session runs it: once the
/// history of this session caught up (`history.synced`), or after
/// `wait_for_history` without it, and then every `every`. The session
/// aborts it when it ends.
pub(crate) async fn sweep_loop(
    dm: DmService,
    outbox: Outbox,
    ui: broadcast::Sender<UiEvent>,
    wait_for_history: Duration,
    every: Duration,
) {
    // Subscribed before anything else, so a catch-up that ends at once
    // is not missed.
    let events = ui.subscribe();
    keep(events, wait_for_history, every, || sweep_now(&dm, &outbox, &ui)).await;
}

/// The schedule of the keeper, apart from what it runs (`sweep`): the
/// first time once `history.synced` comes on `events`, or at
/// `wait_for_history` without it; then every `every`. Over when the page
/// is gone (`events` closed) before the first time; a receiver that fell
/// behind keeps waiting, the end of the history is not lost to it. Apart,
/// so the timing is tested without a session or a clock of the wall.
async fn keep<F: Future<Output = ()>>(
    mut events: broadcast::Receiver<UiEvent>,
    wait_for_history: Duration,
    every: Duration,
    mut sweep: impl FnMut() -> F,
) {
    let deadline = tokio::time::sleep(wait_for_history);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            ev = events.recv() => match ev {
                Ok(ev) if ev.name == UI_EVENT_HISTORY_SYNCED => break,
                Err(broadcast::error::RecvError::Closed) => return,
                _ => {}
            },
        }
    }
    drop(events);
    loop {
        sweep().await;
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::LinkView;
    use crate::tests::{ctx_of, dm_inbound, pk_of};
    use messenger_core::{MessengerConfig, PubKey};
    use messenger_testkit::MemorySecretStore;
    use nostr::key::Keys;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// A runtime with a session, offline, with the relationship gate on:
    /// a stranger's first word is a request.
    async fn started() -> (tempfile::TempDir, MessengerRuntime, PubKey) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        rt.dm().set_gate(true);
        let me = rt.session_pubkey().await.unwrap();
        (dir, rt, me)
    }

    fn now() -> i64 {
        SystemClock.now().secs()
    }

    /// `text` from a stranger, dated `at`, as ingress hands it in.
    async fn knock(rt: &MessengerRuntime, me: &PubKey, who: &Keys, text: &str, at: i64) {
        let msg = dm_inbound(&pk_of(who), me, messenger_core::Envelope::text(text).encode(), at);
        rt.dm().apply_inbound(msg, &ctx_of(me, at)).await.unwrap();
    }

    /// How many wraps wait in the outbox: a signal sent is one more. The
    /// snapshot of my relations (kind 30078) is not counted: the publisher
    /// of the list queues it by itself, `PUBLISH_AFTER` after a decision,
    /// and in a slow run it lands between two counts of a test.
    async fn queued(rt: &MessengerRuntime) -> usize {
        let rows = messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap();
        rows.iter()
            .filter(|r| {
                !matches!(r.outbound(), Ok(messenger_core::Outbound::PublishOwn { event })
                    if event.json["kind"] == messenger_core::relations::KIND_RELATIONS)
            })
            .count()
    }

    #[tokio::test]
    async fn the_queue_pages_newest_first_and_counts_the_whole() {
        let (_dir, rt, me) = started().await;
        assert_eq!(rt.requests(None, 50).await.unwrap(), RequestsPage { items: vec![], next_before: None, total: 0 });
        let t = now() - 100;
        let strangers: Vec<Keys> = (0..3).map(|_| Keys::generate()).collect();
        for (i, s) in strangers.iter().enumerate() {
            knock(&rt, &me, s, &format!("hi {i}"), t + i as i64).await;
        }
        let first = rt.requests(None, 2).await.unwrap();
        assert_eq!(first.total, 3);
        assert_eq!(first.items.len(), 2);
        assert_eq!(first.items[0].peer, strangers[2].public_key().to_hex(), "newest first");
        assert_eq!((first.items[0].text.as_str(), first.items[0].at, first.items[0].held), ("hi 2", t + 2, 0));
        assert_eq!(first.next_before, Some(t + 1), "the `at` of the last one shown");
        let rest = rt.requests(first.next_before, 2).await.unwrap();
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.items[0].text, "hi 0");
        assert_eq!(rest.next_before, None, "the last page");
        assert_eq!(rt.requests_count().await.unwrap(), 3);
        // A second word before an answer waits behind the first.
        knock(&rt, &me, &strangers[0], "still there?", t + 10).await;
        let page = rt.requests(None, 50).await.unwrap();
        let of_0 = page.items.iter().find(|r| r.peer == strangers[0].public_key().to_hex()).unwrap();
        assert_eq!((of_0.text.as_str(), of_0.held), ("hi 0", 1));
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn an_answer_takes_the_road_of_the_chat_and_a_second_one_changes_nothing() {
        let (_dir, rt, me) = started().await;
        let (anna, bob, carl) = (Keys::generate(), Keys::generate(), Keys::generate());
        let t = now() - 50;
        for (k, s) in [&anna, &bob, &carl].iter().enumerate() {
            knock(&rt, &me, s, "hi", t + k as i64).await;
        }
        assert_eq!(rt.requests_count().await.unwrap(), 3);
        let mut events = rt.ui_events();

        let before = queued(&rt).await;
        rt.request_act(&anna.public_key().to_hex(), RequestAction::Accept).await.unwrap();
        let after_accept = queued(&rt).await;
        assert!(after_accept > before, "the accept went out");
        assert!(rt.contacts().is_contact(&pk_of(&anna)).await.unwrap(), "in the address book, as from the chat");
        assert_eq!(rt.requests_count().await.unwrap(), 2);
        let mut told = None;
        while let Ok(ev) = events.try_recv() {
            if ev.name == UI_EVENT_DM_REQUEST {
                told = Some(ev.payload);
            }
        }
        let told = told.expect("the queue was told to count again");
        assert_eq!((told["peer"].as_str(), told["pending"].as_u64()), (Some(anna.public_key().to_hex().as_str()), Some(2)));
        rt.request_act(&anna.public_key().to_hex(), RequestAction::Accept).await.unwrap();
        assert_eq!(queued(&rt).await, after_accept, "accepted already: no second signal");

        rt.request_act(&bob.public_key().to_hex(), RequestAction::Decline).await.unwrap();
        let after_decline = queued(&rt).await;
        assert!(after_decline > after_accept, "the no went out, as from the chat");
        assert_eq!(rt.requests_count().await.unwrap(), 1);
        rt.request_act(&bob.public_key().to_hex(), RequestAction::Decline).await.unwrap();
        assert_eq!(queued(&rt).await, after_decline, "declined already: no second signal");
        assert_eq!(rt.dm_relation(&bob.public_key().to_hex()).await.unwrap().my_contact, "declined");

        rt.request_act(&carl.public_key().to_hex(), RequestAction::Block).await.unwrap();
        let after_block = queued(&rt).await;
        assert!(after_block > after_decline, "the block went out");
        rt.request_act(&carl.public_key().to_hex(), RequestAction::Block).await.unwrap();
        assert_eq!(queued(&rt).await, after_block, "blocked already: no second signal");
        assert_eq!(rt.requests_count().await.unwrap(), 0);
        assert_eq!(rt.dm_blocked().await.unwrap(), vec![carl.public_key().to_hex()]);
        // Nobody asked: nothing to answer, and the error says so.
        assert!(rt.request_act(&Keys::generate().public_key().to_hex(), RequestAction::Accept).await.is_err());
        rt.shutdown().await;
    }

    /// What the outbox holds besides the snapshot of my relations: wraps
    /// for the home set (an inbox or my relays) and wraps for one server.
    async fn queued_by_scope(rt: &MessengerRuntime) -> (usize, Vec<String>) {
        let rows = messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap();
        let (mut home, mut servers) = (0, Vec::new());
        for r in rows {
            match r.outbound().unwrap() {
                messenger_core::Outbound::PublishOwn { event } if event.json["kind"] == messenger_core::relations::KIND_RELATIONS => {}
                messenger_core::Outbound::PublishScoped { scope: messenger_core::Scope::Server { id }, .. } => servers.push(id),
                _ => home += 1,
            }
        }
        (home, servers)
    }

    /// A request in a private chat is answered in that chat, by its
    /// address (the `chat_id` of the row): the signal goes to its server,
    /// the home relation and the address book stay as they were; a block
    /// is that chat's alone. The rows say which server.
    #[tokio::test]
    async fn a_request_of_a_private_chat_is_answered_there_and_not_at_home() {
        let (_dir, rt, me) = started().await;
        let s1 = rt
            .server_put_private(&crate::private::PrivateServerInput { relay_url: "wss://s1.example".into(), name: Some("S1".into()), ..Default::default() })
            .await
            .unwrap();
        let (anna, bob) = (Keys::generate(), Keys::generate());
        let t = now() - 50;
        for (i, who) in [&anna, &bob].into_iter().enumerate() {
            let mut msg = dm_inbound(&pk_of(who), &me, messenger_core::Envelope::text("hi on s1").encode(), t + i as i64);
            msg.srv = Some(s1.clone());
            rt.dm().apply_inbound(msg, &ctx_of(&me, t)).await.unwrap();
        }
        let page = rt.requests(None, 50).await.unwrap();
        assert_eq!(page.total, 2);
        let anna_on_s1 = format!("{}@{s1}", anna.public_key().to_hex());
        let row = page.items.iter().find(|r| r.peer == anna.public_key().to_hex()).unwrap();
        assert_eq!((row.chat_id.as_str(), row.server_id.as_deref()), (format!("dm:{anna_on_s1}").as_str(), Some(s1.as_str())));

        let (home_before, _) = queued_by_scope(&rt).await;
        rt.request_act(&row.chat_id, RequestAction::Accept).await.unwrap();
        let (home_after, on_servers) = queued_by_scope(&rt).await;
        assert_eq!(home_after, home_before, "nothing of the accept goes home");
        assert!(!on_servers.is_empty() && on_servers.iter().all(|id| id == &s1), "the accept went to S1: {on_servers:?}");
        assert_eq!(rt.dm_relation(&anna_on_s1).await.unwrap().my_contact, "approved");
        assert_eq!(rt.dm_relation(&anna.public_key().to_hex()).await.unwrap().my_contact, "none", "the home relation is untouched");
        assert!(rt.dm().chat(&format!("dm:{}", anna.public_key().to_hex())).await.unwrap().is_none(), "no home chat came of it");
        assert!(!rt.contacts().is_contact(&pk_of(&anna)).await.unwrap(), "nor a contact of the book");
        assert_eq!(rt.requests_count().await.unwrap(), 1);

        // The address `<key>@<server>` does the same; a block is the chat's.
        rt.request_act(&format!("{}@{s1}", bob.public_key().to_hex()), RequestAction::Block).await.unwrap();
        assert!(rt.dm_relation(&format!("{}@{s1}", bob.public_key().to_hex())).await.unwrap().blocked);
        assert!(!rt.dm_relation(&bob.public_key().to_hex()).await.unwrap().blocked, "not blocked at home");
        assert!(rt.dm_blocked().await.unwrap().is_empty(), "the list of blocked people is of the home set");
        assert_eq!(rt.requests_count().await.unwrap(), 0);
        let (home_end, _) = queued_by_scope(&rt).await;
        assert_eq!(home_end, home_before, "nothing of the block goes home");
        // A server this device has not: refused, nothing done.
        assert!(rt.request_act(&format!("{}@{}", anna.public_key().to_hex(), "f".repeat(16)), RequestAction::Decline).await.is_err());
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn the_settings_round_trip_and_a_shorter_term_sweeps_at_once() {
        let (_dir, rt, me) = started().await;
        assert_eq!(rt.dm_settings().await.unwrap(), DmSettings { dm_from: "anyone".into(), keep_days: 7 });
        let old = Keys::generate();
        knock(&rt, &me, &old, "hi", now() - 2 * 86_400).await;
        let fresh = Keys::generate();
        knock(&rt, &me, &fresh, "hi", now() - 60).await;
        assert_eq!(rt.requests_count().await.unwrap(), 2);
        assert!(matches!(
            rt.dm_settings_set(DmSettings { dm_from: "everyone".into(), keep_days: 7 }).await,
            Err(MessengerError::Invalid(ref m)) if m == "dm_from"
        ));

        let mut events = rt.ui_events();
        rt.dm_settings_set(DmSettings { dm_from: "contacts".into(), keep_days: 1 }).await.unwrap();
        assert_eq!(rt.dm_settings().await.unwrap(), DmSettings { dm_from: "contacts".into(), keep_days: 1 });
        assert_eq!(rt.requests_count().await.unwrap(), 1, "two days old: out, now");
        let peers: Vec<String> = rt.requests(None, 50).await.unwrap().items.into_iter().map(|r| r.peer).collect();
        assert_eq!(peers, vec![fresh.public_key().to_hex()]);
        let mut pending = None;
        while let Ok(ev) = events.try_recv() {
            if ev.name == UI_EVENT_DM_REQUEST {
                pending = ev.payload["pending"].as_u64();
            }
        }
        assert_eq!(pending, Some(1), "the page is told");
        assert_eq!(rt.dm_relation(&old.public_key().to_hex()).await.unwrap().my_contact, "expired");
        // A no to the one that ran out (the page had it open): done already,
        // and the peer, whom the expiry told nothing, hears nothing now.
        let before = queued(&rt).await;
        rt.request_act(&old.public_key().to_hex(), RequestAction::Decline).await.unwrap();
        assert_eq!(queued(&rt).await, before, "no signal");
        assert_eq!(rt.dm_relation(&old.public_key().to_hex()).await.unwrap().my_contact, "expired");
        // Under "contacts" the next stranger is dropped without a trace.
        knock(&rt, &me, &Keys::generate(), "hello?", now()).await;
        assert_eq!(rt.requests_count().await.unwrap(), 1);
        // A request of before stays; 0 is "never".
        rt.dm_settings_set(DmSettings { dm_from: "anyone".into(), keep_days: 0 }).await.unwrap();
        assert_eq!(rt.requests_count().await.unwrap(), 1);
        rt.shutdown().await;
    }

    /// What a sweep of the session's keeper decides reaches the page, and
    /// the session runs a keeper of its own.
    #[tokio::test]
    async fn the_session_sweeps_after_the_history_and_tells_the_page() {
        let (_dir, rt, me) = started().await;
        let stale = Keys::generate();
        knock(&rt, &me, &stale, "hi", now() - 10 * 86_400).await;
        assert_eq!(rt.requests_count().await.unwrap(), 1);
        let mut events = rt.ui_events();
        let keeper = tokio::spawn(sweep_loop(
            rt.dm().clone(),
            rt.outbox().clone(),
            rt.ui.clone(),
            Duration::from_secs(30),
            Duration::from_secs(3600),
        ));
        tokio::task::yield_now().await;
        assert_eq!(rt.requests_count().await.unwrap(), 1, "nothing before the history caught up");
        let _ = rt.ui.send(UiEvent { name: UI_EVENT_HISTORY_SYNCED.into(), payload: serde_json::json!({ "since": 0 }) });
        let told = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let ev = events.recv().await.unwrap();
                if ev.name == UI_EVENT_DM_REQUEST {
                    return ev.payload;
                }
            }
        })
        .await
        .expect("the sweep told the page");
        assert_eq!(told["pending"], 0);
        assert_eq!(rt.dm_relation(&stale.public_key().to_hex()).await.unwrap().my_contact, "expired");
        keeper.abort();
        // The session of the runtime runs the same keeper: one of its own.
        assert!(rt.session.lock().await.as_ref().is_some_and(|s| s.sweeps_requests()));
        rt.shutdown().await;
    }

    /// The moderation of a group of one, offline: the rules of posting
    /// round-trip as an operation of the journal and go out once, the
    /// queue and the log answer, and what cannot be done is refused by
    /// its name.
    #[tokio::test]
    async fn the_rules_of_posting_round_trip_and_the_queue_and_log_answer() {
        let (_dir, rt, me) = started().await;
        let g = rt.group_create(messenger_groups::GroupKind::Private, "Team", "", true, None).await.unwrap();
        assert_eq!((g.queue, g.needs_update), (0, false));
        let defaults = rt.group_mod_settings(&g.id).await.unwrap();
        assert_eq!(defaults, GroupModSettings { post_mode: "all".into(), schedule: None, premod: "off".into() });
        assert_eq!(rt.messages(&g.chat_id, None, 50).await.unwrap().len(), 1, "the line of the creation, as the owner (a moderator) sees it");

        let before = queued(&rt).await;
        let wanted = GroupModSettings {
            post_mode: "managers".into(),
            schedule: Some(Schedule { tz: "Europe/Berlin".into(), days: vec![1, 2, 3, 4, 5], from: "09:00".into(), to: "18:00".into() }),
            premod: "newcomers".into(),
        };
        rt.group_mod_settings_set(&g.id, wanted.clone()).await.unwrap();
        assert_eq!(rt.group_mod_settings(&g.id).await.unwrap(), wanted);
        let after = queued(&rt).await;
        assert!(after > before, "one operation went out");
        rt.group_mod_settings_set(&g.id, wanted.clone()).await.unwrap();
        assert_eq!(queued(&rt).await, after, "set already: nothing goes out");
        let view = rt.group_get(&g.id).await.unwrap();
        assert_eq!((view.post_mode.as_str(), view.premod.as_str(), view.schedule.is_some()), ("managers", "newcomers", true));
        // Taken away: a schedule of no days in the operation, none in the view.
        rt.group_mod_settings_set(&g.id, GroupModSettings { schedule: None, ..wanted.clone() }).await.unwrap();
        assert_eq!(rt.group_mod_settings(&g.id).await.unwrap().schedule, None);
        for (bad, code) in [
            (GroupModSettings { post_mode: "owners".into(), ..wanted.clone() }, "post_mode"),
            (GroupModSettings { premod: "some".into(), ..wanted.clone() }, "premod"),
            (
                GroupModSettings {
                    schedule: Some(Schedule { tz: "Europe/Berlin".into(), days: vec![], from: "09:00".into(), to: "18:00".into() }),
                    ..wanted.clone()
                },
                "post_schedule",
            ),
        ] {
            assert!(matches!(rt.group_mod_settings_set(&g.id, bad).await, Err(MessengerError::Invalid(ref m)) if m == code), "{code}");
        }

        // The queue of a group of one is empty; the log holds the settings, newest first.
        let page = rt.group_queue(&g.id, QueueFilter::default(), None, 50).await.unwrap();
        assert_eq!(page, QueuePage { items: vec![], next_before: None, total: 0 });
        let log = rt.group_log(&g.id, None, 50).await.unwrap();
        assert_eq!(log.items.len(), 2, "two operations: the rules, then the schedule taken away");
        assert!(log.items.iter().all(|l| l.action == "settings" && l.by == me.as_hex()));
        assert!(log.items[1].detail.contains("\"post_mode\":\"managers\""));
        assert!(log.items[0].detail.contains("\"schedule\":null"));
        assert_eq!(log.next_before, None);
        let one = rt.group_log(&g.id, None, 1).await.unwrap();
        assert_eq!((one.items.len(), one.next_before), (1, Some(one.items[0].at)));
        assert_eq!(rt.group_log(&g.id, one.next_before, 50).await.unwrap().items.len(), 1, "strictly older");

        // Acts on nobody and on nothing are refused; my own word is above my reach.
        let nobody = Keys::generate().public_key().to_hex();
        assert!(rt.group_mod(&g.id, ModAction::Mute { who: nobody.clone(), until: None }).await.is_err());
        assert!(rt.group_mod(&g.id, ModAction::Hide { targets: vec!["ab".repeat(32)] }).await.is_err());
        let line = rt.messages(&g.chat_id, None, 50).await.unwrap().pop().unwrap();
        assert!(rt.group_mod(&g.id, ModAction::Hide { targets: vec![line.id] }).await.is_err(), "a line of the system is no message");
        assert_eq!(rt.group_inherit(&g.id, &nobody).await.unwrap(), 0, "nothing of theirs to say again");
        assert!(rt.group_mod(&g.id, ModAction::Trust { who: "not a key".into(), trusted: true }).await.is_err());
        rt.shutdown().await;
    }

    // ─── Members of a group, as their devices would speak ────────────────────

    use messenger_core::inbound::{Envelope as WireEnvelope, GroupInbound};
    use messenger_core::outbound::WireEvent;
    use messenger_core::{EventSource, RelayUrl, Timestamp};
    use messenger_groups::{wire, GroupKey, GroupKind, GroupView, KeyId, OpLog, Role};
    use nostr::nips::nip19::ToBech32;

    /// The key the group seals with now, read from the secret store under
    /// the name the groups crate keeps it by (`key_ref`): the test stands
    /// in for a member whose device holds the same key.
    async fn group_key(rt: &MessengerRuntime, group: &str) -> GroupKey {
        let id = rt.group_get(group).await.unwrap().key.expect("a key").id;
        let bytes = rt.secrets.get(&format!("group.{group}.key.{id}")).await.unwrap().expect("the key is kept");
        wire::check_key(&KeyId(id), &bytes).unwrap()
    }

    /// A sealed event of the group handed in as the relay hands it.
    async fn deliver(rt: &MessengerRuntime, me: &PubKey, group: &str, event: &WireEvent, at: i64) {
        let ev: nostr::prelude::Event = serde_json::from_value(event.json.clone()).unwrap();
        let tag = |name: &str| ev.tags.iter().filter(|t| t.kind() == name).filter_map(|t| t.as_slice().get(1)).next().cloned();
        let msg = GroupInbound {
            envelope: WireEnvelope {
                wire_id: event.id.clone(),
                source: EventSource::Relay { url: RelayUrl::parse("wss://relay.example").unwrap() },
                wire_created_at: Timestamp(ev.created_at.as_secs() as i64),
            },
            group_id: group.to_string(),
            sender: PubKey::parse(&ev.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(ev.created_at.as_secs() as i64),
            kind: 9,
            key_id: tag("k"),
            ciphertext: ev.content.clone(),
            reply_to: None,
        };
        let keys = rt.session_keys().await.unwrap();
        let outcome = rt.groups().on_event(&keys, msg, &ctx_of(me, at)).await.unwrap();
        rt.group_driver.apply(outcome).await.unwrap();
    }

    /// `who` says `text` in the group, dated `at`; the id of the message.
    async fn says(rt: &MessengerRuntime, me: &PubKey, group: &str, who: &Keys, text: &str, at: i64) -> String {
        let key = group_key(rt, group).await;
        let signed = wire::sign_message(who, group, &messenger_core::Envelope::text(text).encode(), at, None).unwrap();
        let sealed = wire::seal_message(group, &key, None, &signed, who).unwrap();
        deliver(rt, me, group, &sealed, at).await;
        signed.id.to_hex()
    }

    /// `who` makes an operation of the journal, on the journal as this
    /// device holds it.
    async fn acts(rt: &MessengerRuntime, me: &PubKey, group: &str, who: &Keys, body: OpBody, at: i64) {
        let ops: Vec<_> = messenger_store::groups::ops(rt.store(), group)
            .await
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .filter_map(|v| wire::verify_op(&v, group).ok())
            .collect();
        let op = OpLog::from_ops(ops).unwrap().next(&pk_of(who), at, body);
        let signed = wire::sign_op(who, &op).unwrap();
        let key = group_key(rt, group).await;
        let sealed = wire::seal_op(group, &key, &signed, vec![], who).unwrap();
        deliver(rt, me, group, &sealed, at).await;
    }

    /// A group of the owner's device with two members let in and every
    /// message checked.
    async fn moderated(rt: &MessengerRuntime) -> (GroupView, Keys, Keys) {
        let g = rt.group_create(GroupKind::Private, "Team", "", true, None).await.unwrap();
        let (bob, carol) = (Keys::generate(), Keys::generate());
        // Let in by the owner's own operation of the journal, handed in
        // as the relay would hand it back: the invitation and its answer
        // are another test's, and their devices are this test.
        let keys = rt.session_keys().await.unwrap();
        let me = rt.session_pubkey().await.unwrap();
        for (i, who) in [&bob, &carol].into_iter().enumerate() {
            acts(rt, &me, &g.id, &keys, OpBody::Admit { who: pk_of(who) }, now() - 10 + i as i64).await;
        }
        assert_eq!(rt.group_get(&g.id).await.unwrap().members.len(), 3);
        rt.group_mod_settings_set(&g.id, GroupModSettings { post_mode: "all".into(), schedule: None, premod: "all".into() }).await.unwrap();
        (g, bob, carol)
    }

    /// Every act of the facade on a member goes out once and not again
    /// when it is done already; the queue and the log page through lines
    /// that share a second, and the queue takes an author as an npub.
    #[tokio::test]
    async fn the_acts_of_moderation_go_out_once_and_the_pages_keep_a_second_together() {
        let (_dir, rt, me) = started().await;
        let (g, bob, carol) = moderated(&rt).await;
        let bob_pk = pk_of(&bob);
        let t = now();
        let b1 = says(&rt, &me, &g.id, &bob, "b1", t).await;
        let c1 = says(&rt, &me, &g.id, &carol, "c1", t).await;
        // Dated a little ahead of the clock, as a clock ahead of mine
        // would: the span of a «spam» ends there, whenever it is said.
        let b2 = says(&rt, &me, &g.id, &bob, "b2", t + 100).await;
        let c2 = says(&rt, &me, &g.id, &carol, "c2", t + 100).await;
        assert_eq!(rt.group_get(&g.id).await.unwrap().queue, 4);

        // A page of one takes both lines of the second; the next page is empty.
        let page = rt.group_queue(&g.id, QueueFilter::default(), None, 1).await.unwrap();
        assert_eq!((page.total, page.items.len(), page.next_before), (4, 2, Some(t)), "one line per author (their oldest), both dated alike");
        assert!(rt.group_queue(&g.id, QueueFilter::default(), page.next_before, 1).await.unwrap().items.is_empty(), "nothing passed by");
        let of_bob = QueueFilter { author: Some(bob.public_key().to_bech32().unwrap()), ..Default::default() };
        let page = rt.group_queue(&g.id, of_bob, None, 50).await.unwrap();
        assert_eq!((page.total, page.items.len(), page.items[0].message_id.as_str(), page.items[0].more_from_author), (2, 1, b1.as_str(), 1));

        let mut n = queued(&rt).await;
        let mut once = async |action: ModAction, what: &str| {
            rt.group_mod(&g.id, action.clone()).await.unwrap();
            let after = queued(&rt).await;
            assert!(after > n, "{what}: went out");
            n = after;
            rt.group_mod(&g.id, action).await.unwrap();
            assert_eq!(queued(&rt).await, n, "{what}: done already, nothing goes out again");
        };
        once(ModAction::Approve { target: b1.clone(), version: b1.clone() }, "approve").await;
        once(ModAction::Reject { target: c1.clone() }, "reject").await;
        once(ModAction::Reject { target: c2.clone() }, "reject").await;
        once(ModAction::Hide { targets: vec![b2.clone()] }, "hide").await;
        once(ModAction::Unhide { targets: vec![b2.clone()] }, "unhide").await;
        once(ModAction::Mute { who: bob_pk.as_hex().into(), until: Some(t + 3600) }, "mute until").await;
        once(ModAction::Mute { who: bob_pk.as_hex().into(), until: None }, "mute for good").await;
        once(ModAction::Unmute { who: bob_pk.as_hex().into() }, "unmute").await;
        once(ModAction::Trust { who: bob_pk.as_hex().into(), trusted: true }, "trust").await;
        once(ModAction::Trust { who: bob_pk.as_hex().into(), trusted: false }, "untrust").await;
        once(ModAction::Spam { author: bob_pk.as_hex().into(), target: Some(b2.clone()) }, "spam").await;
        let state = rt.groups().state_of(&g.id).await.unwrap();
        assert!(state.is_banned(&bob_pk));
        assert_eq!(rt.group_get(&g.id).await.unwrap().queue, 0, "bob's rows are hidden, carol's rejected");

        // A mark is dated at the later of now and its message, and my later
        // word about one message after my earlier one: the hiding of `b2`
        // and the rejection of `c2` share the second of the two messages,
        // the unhiding of `b2` is the second after. A page of one keeps
        // every line of a second together, and the page after, strictly
        // older, holds the rest, so that the pages cover the whole once.
        let all = rt.group_log(&g.id, None, 50).await.unwrap();
        let mut actions: Vec<&str> = all.items.iter().map(|l| l.action.as_str()).collect();
        actions.sort_unstable();
        assert_eq!(actions, vec!["approve", "ban", "hide", "hide", "mute", "mute", "reject", "reject", "settings", "trust", "unhide", "unmute", "untrust"]);
        assert!(all.items.iter().any(|l| l.action == "ban" && l.target == bob_pk.as_hex()));
        assert!(all.items.iter().any(|l| l.action == "hide" && l.target == bob_pk.as_hex() && l.detail.contains("\"from\":0") && l.detail.contains("\"reason\":\"spam\"")));
        let first = rt.group_log(&g.id, None, 1).await.unwrap();
        assert_eq!((first.items.len(), first.items[0].at, first.items[0].action.as_str(), first.next_before), (1, t + 101, "unhide", Some(t + 101)));
        let tied = rt.group_log(&g.id, first.next_before, 1).await.unwrap();
        assert_eq!(tied.items.iter().map(|l| (l.at, l.action.as_str())).collect::<Vec<_>>(), vec![(t + 100, "hide"), (t + 100, "reject")], "a page of one holds every line of the second");
        assert_eq!(tied.next_before, Some(t + 100));
        let rest = rt.group_log(&g.id, tied.next_before, 50).await.unwrap();
        assert!(rest.items.iter().all(|l| l.at < t + 100) && rest.next_before.is_none());
        assert_eq!(all.items.len(), 1 + tied.items.len() + rest.items.len(), "nothing passed by, nothing twice");
        rt.shutdown().await;
    }

    /// A moderator who may not ban says «spam» and the hiding alone goes
    /// out; the author stays a member.
    #[tokio::test]
    async fn a_moderator_s_spam_is_the_hiding_alone() {
        let (_dir, rt, me) = started().await;
        let (g, bob, carol) = moderated(&rt).await;
        let carol_pk = pk_of(&carol);
        rt.group_act(&g.id, OpBody::TransferOwnership { to: pk_of(&bob) }).await.unwrap();
        let t = now();
        acts(&rt, &me, &g.id, &bob, OpBody::SetRole { who: me.clone(), role: Role::Moderator }, t).await;
        assert_eq!(rt.group_get(&g.id).await.unwrap().my_role.as_deref(), Some("moderator"));
        let c1 = says(&rt, &me, &g.id, &carol, "buy now", t + 1).await;
        assert_eq!(rt.group_get(&g.id).await.unwrap().queue, 1, "a moderator still sees what waits");
        let n = queued(&rt).await;
        rt.group_mod(&g.id, ModAction::Spam { author: carol_pk.as_hex().into(), target: Some(c1.clone()) }).await.unwrap();
        assert_eq!(queued(&rt).await, n + 1, "the hiding alone: the ban is a manager's");
        let state = rt.groups().state_of(&g.id).await.unwrap();
        assert!(!state.is_banned(&carol_pk) && state.is_member(&carol_pk));
        assert!(rt.messages(&g.chat_id, None, 50).await.unwrap().iter().all(|m| m.id != c1), "hidden here");
        let log = rt.group_log(&g.id, None, 50).await.unwrap();
        assert!(log.items.iter().any(|l| l.action == "hide" && l.target == carol_pk.as_hex() && l.detail.contains("\"reason\":\"spam\"")));
        assert!(!log.items.iter().any(|l| l.action == "ban"));
        rt.shutdown().await;
    }

    /// `keep` counting its sweeps, on the clock of the test. The caller
    /// gives it a turn before moving the clock: its wait starts when it is
    /// first polled.
    fn kept(ui: &broadcast::Sender<UiEvent>, wait: Duration, every: Duration) -> (Arc<AtomicU32>, tokio::task::JoinHandle<()>) {
        let count = Arc::new(AtomicU32::new(0));
        let events = ui.subscribe();
        let n = count.clone();
        let task = tokio::spawn(async move {
            keep(events, wait, every, || {
                let n = n.clone();
                async move {
                    n.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
        });
        (count, task)
    }

    /// Lets the keeper take its turn after the clock or the channel moved.
    async fn turn() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    fn synced() -> UiEvent {
        UiEvent { name: UI_EVENT_HISTORY_SYNCED.into(), payload: serde_json::json!({ "since": 0 }) }
    }

    /// The keeper waits for the history to catch up, sweeps, and sweeps
    /// again every period; events that are not the end of the history,
    /// more of them than the channel holds, change nothing.
    #[tokio::test(start_paused = true)]
    async fn the_keeper_sweeps_once_the_history_caught_up_and_then_by_the_period() {
        let (ui, _keep_open) = broadcast::channel(2);
        let (count, task) = kept(&ui, Duration::from_secs(120), Duration::from_secs(3600));
        turn().await;
        tokio::time::advance(Duration::from_secs(119)).await;
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "nothing before the history caught up");
        for _ in 0..5 {
            let _ = ui.send(UiEvent { name: "chats.updated".into(), payload: serde_json::json!({}) });
        }
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "fell behind the channel, still waiting");
        let _ = ui.send(synced());
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "swept as the history caught up");
        tokio::time::advance(Duration::from_secs(3599)).await;
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "once a period");
        tokio::time::advance(Duration::from_secs(3600)).await;
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
        task.abort();
    }

    /// Without the end of the history (relays or no relays) the first sweep
    /// is at the deadline exactly.
    #[tokio::test(start_paused = true)]
    async fn the_keeper_waits_for_the_history_no_longer_than_the_deadline() {
        let (ui, _keep_open) = broadcast::channel(2);
        let (count, task) = kept(&ui, Duration::from_secs(120), Duration::from_secs(3600));
        turn().await;
        tokio::time::advance(Duration::from_millis(119_999)).await;
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        tokio::time::advance(Duration::from_millis(1)).await;
        turn().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "at the deadline");
        task.abort();
    }

    /// The page gone before the history caught up, the keeper is over.
    #[tokio::test(start_paused = true)]
    async fn the_keeper_ends_with_the_page() {
        let (ui, _) = broadcast::channel(2);
        let (count, task) = kept(&ui, Duration::from_secs(120), Duration::from_secs(3600));
        turn().await;
        drop(ui);
        tokio::time::timeout(Duration::from_secs(1), task).await.expect("over").unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    /// A group this device does not hold refuses every act by its name;
    /// an invitation that is not one is refused by its name too.
    #[tokio::test]
    async fn an_unknown_group_and_a_wrong_invitation_refuse_by_name() {
        let (_dir, rt, _me) = started().await;
        let code = |r: Result<()>| match r {
            Err(MessengerError::Invalid(m)) => m,
            other => panic!("{other:?}"),
        };
        let somebody = Keys::generate().public_key().to_hex();
        assert_eq!(code(rt.group_queue("g", QueueFilter::default(), None, 10).await.map(|_| ())), "group_not_permitted");
        assert_eq!(code(rt.group_mod("g", ModAction::Unmute { who: somebody.clone() }).await), "group_unknown");
        assert_eq!(code(rt.group_mod("g", ModAction::Hide { targets: vec!["ab".repeat(32)] }).await), "group_unknown");
        assert_eq!(code(rt.group_log("g", None, 10).await.map(|_| ())), "group_not_permitted");
        assert_eq!(code(rt.group_mod_settings("g").await.map(|_| ())), "group_unknown");
        let settings = GroupModSettings { post_mode: "all".into(), schedule: None, premod: "off".into() };
        assert_eq!(code(rt.group_mod_settings_set("g", settings).await), "group_unknown");
        assert_eq!(code(rt.group_inherit("g", &somebody).await.map(|_| ())), "group_unknown");
        let preset = |kind: &str| InvitePreset { kind: kind.into(), max_uses: None, ttl_secs: None, for_pubkey: None, label: String::new() };
        assert_eq!(code(rt.invite_create(preset("ticket")).await.map(|_| ())), "invite_kind");
        assert_eq!(code(rt.invite_create(InvitePreset { ttl_secs: Some(0), ..preset("link") }).await.map(|_| ())), "invite_ttl");
        assert_eq!(code(rt.invite_create(InvitePreset { max_uses: Some(0), ..preset("code") }).await.map(|_| ())), "invite_max_uses");
        let me = rt.session_pubkey().await.unwrap().as_hex().to_string();
        assert_eq!(code(rt.invite_create(InvitePreset { for_pubkey: Some(me), ..preset("code") }).await.map(|_| ())), "invite_for");
        assert_eq!(code(rt.invite_revoke("i").await), "invite_unknown");
        assert_eq!(code(rt.invite_qr("i").await.map(|_| ())), "invite_unknown");
        assert!(rt.invite_list().await.unwrap().is_empty());
        rt.shutdown().await;
    }

    /// The presets: what each kind of invitation is by default, what a
    /// preset may say otherwise, the link and the QR of it, the list with
    /// the open ones first, the revocation, and the pause.
    #[tokio::test]
    async fn an_invitation_is_made_by_its_preset_listed_revoked_and_the_pause_is_told() {
        let (_dir, rt, me) = started().await;
        let preset = |kind: &str| InvitePreset { kind: kind.into(), max_uses: None, ttl_secs: None, for_pubkey: None, label: " the link for Bob ".into() };
        let t0 = now();
        let link = rt.invite_create(preset("link")).await.unwrap();
        assert_eq!((link.kind.as_str(), link.max_uses, link.used, link.revoked, link.label.as_str()), ("link", Some(1), 0, false, "the link for Bob"));
        assert!(messenger_links::is_invite_secret(&link.secret), "{}", link.secret);
        assert!(link.expires_at.is_some_and(|e| (e - t0 - LINK_TTL_SECS).abs() <= 2));
        let url = link.link.clone().expect("a link has a link");
        assert!(url.starts_with(&format!("veydan://contact/{}?", messenger_links::contact::npub_of(&me).unwrap())), "{url}");
        assert!(url.ends_with(&format!("&i={}", link.secret)) || url.ends_with(&format!("?i={}", link.secret)), "{url}");
        match rt.links_inspect(std::slice::from_ref(&url)).await.unwrap().remove(0) {
            LinkView::Contact { invite, is_me, .. } => assert_eq!((invite.as_deref(), is_me), (Some(link.secret.as_str()), true)),
            other => panic!("{other:?}"),
        }
        let svg = rt.invite_qr(&link.id).await.unwrap();
        assert!(svg.contains("<svg") || svg.starts_with("<?xml"));

        let bob = Keys::generate().public_key().to_hex();
        let code = rt.invite_create(InvitePreset { for_pubkey: Some(bob.clone()), ..preset("code") }).await.unwrap();
        assert_eq!(code.secret.len(), CODE_DIGITS);
        assert!(code.secret.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!((code.link.as_deref(), code.max_uses, code.for_pubkey.as_deref()), (None, Some(1), Some(bob.as_str())));
        assert!(code.expires_at.is_some_and(|e| (e - t0 - CODE_TTL_SECS).abs() <= 2));
        assert_eq!(code_of(rt.invite_qr(&code.id).await), "invite_no_link");

        let screen = rt.invite_create(InvitePreset { ttl_secs: Some(QR_TTL_SECS), ..preset("qr") }).await.unwrap();
        assert!(screen.link.is_some() && screen.max_uses.is_none());
        assert!(screen.expires_at.is_some_and(|e| (e - t0 - QR_TTL_SECS).abs() <= 2));
        let forever = rt.invite_create(preset("qr")).await.unwrap();
        assert_eq!((forever.expires_at, forever.max_uses), (None, None), "neither a term nor a number: for good");
        let ten = rt.invite_create(InvitePreset { max_uses: Some(10), ..preset("qr") }).await.unwrap();
        assert!(ten.expires_at.is_some() && ten.max_uses == Some(10), "a number alone: the quarter of an hour");
        let long = rt.invite_create(InvitePreset { ttl_secs: Some(3), max_uses: Some(3), ..preset("link") }).await.unwrap();
        assert_eq!((long.max_uses, long.expires_at.map(|e| e - t0 <= 5)), (Some(3), Some(true)));

        // The list: open ones first, newest first; a revoked one goes behind.
        rt.invite_revoke(&forever.id).await.unwrap();
        rt.invite_revoke(&forever.id).await.unwrap();
        let list = rt.invite_list().await.unwrap();
        let ids: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.last(), Some(&forever.id.as_str()));
        assert!(list.last().unwrap().revoked);
        assert_eq!(list.len(), 6);
        assert_eq!(rt.invite_status().await.unwrap(), InviteStatus { paused_until: None });

        // Bob comes in by the code: a full chat, no request, the use counted.
        let bob_keys = Keys::generate();
        let bob_pk = pk_of(&bob_keys);
        let wrong = rt.invite_create(InvitePreset { for_pubkey: Some(bob_pk.as_hex().into()), ..preset("code") }).await.unwrap();
        let accept = |secret: &str, at: i64| dm_inbound(&bob_pk, &me, messenger_core::Envelope::control_with_invite("dm_accept", secret).encode(), at);
        let before = queued(&rt).await;
        let fx = rt.dm().apply_inbound(accept(&wrong.secret, now()), &ctx_of(&me, now())).await.unwrap();
        forward_effects(&rt.outbox, &rt.ui, fx).await.unwrap();
        assert_eq!(rt.dm_relation(bob_pk.as_hex()).await.unwrap().mode, "full_chat");
        assert_eq!(rt.requests_count().await.unwrap(), 0);
        assert_eq!(rt.invite_list().await.unwrap().iter().find(|i| i.id == wrong.id).unwrap().used, 1);
        assert_eq!(queued(&rt).await - before, 2, "my accept goes back, with its copy");

        // Ten guesses of a stranger within the hour: paused, and the page
        // told by the effect of the tenth, forwarded as the session's sink
        // forwards every effect (`forward_effects`); the nine before it
        // tell nothing.
        let mut events = rt.ui.subscribe();
        for n in 0..10 {
            let who = pk_of(&Keys::generate());
            let guess = dm_inbound(&who, &me, messenger_core::Envelope::control_with_invite("dm_accept", &format!("{n:06}")).encode(), now());
            let fx = rt.dm().apply_inbound(guess, &ctx_of(&me, now())).await.unwrap();
            let told = fx.iter().filter(|e| matches!(e, Effect::Emit(_))).count();
            assert_eq!(told, usize::from(n == 9), "guess {n}");
            forward_effects(&rt.outbox, &rt.ui, fx).await.unwrap();
        }
        let status = rt.invite_status().await.unwrap();
        assert!(status.paused_until.is_some_and(|u| u > now()));
        let ev = events.recv().await.unwrap();
        assert_eq!((ev.name.as_str(), ev.payload["until"].as_i64()), (UI_EVENT_DM_INVITE_PAUSED, status.paused_until));

        // The blocked list speaks npub and a name.
        rt.dm_act(bob_pk.as_hex(), Action::Block).await.unwrap();
        let blocked = rt.blocked().await.unwrap();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].peer, bob_pk.as_hex());
        assert!(blocked[0].npub.starts_with("npub1"));
        assert!(blocked[0].name.starts_with("npub1") && blocked[0].name.contains('…'), "{}", blocked[0].name);
        rt.shutdown().await;
    }

    /// A request by an invitation from this side: the accept carries the
    /// secret; with a text, the text goes first and the accept after it.
    #[tokio::test]
    async fn a_request_by_an_invitation_carries_the_secret() {
        let (_dir, rt, _me) = started().await;
        let alice = Keys::generate().public_key().to_hex();
        let before = queued(&rt).await;
        let r = rt.dm_request(&alice, Some("AAECAwQFBgcICQoLDA0O_w"), None).await.unwrap();
        assert_eq!(r.mode, "request_sent");
        assert_eq!(queued(&rt).await - before, 2, "the accept and its copy");
        assert_eq!(controls_of(&rt, &alice).await, vec![Some("AAECAwQFBgcICQoLDA0O_w".to_string())]);

        let carol = Keys::generate().public_key().to_hex();
        let before = queued(&rt).await;
        let r = rt.dm_request(&carol, Some("123456"), Some("hi, it is me")).await.unwrap();
        assert_eq!(r.mode, "request_sent");
        assert_eq!(queued(&rt).await - before, 4, "the text and the accept, each with its copy");
        let rows = messenger_store::messages::list(rt.store(), &messenger_store::chats::dm_chat_id(&carol, None), None, 10, true).await.unwrap();
        assert!(rows.iter().any(|m| m.text.as_deref() == Some("hi, it is me")));
        assert_eq!(controls_of(&rt, &carol).await, vec![Some("123456".to_string())]);
        assert!(rt.contacts().is_contact(&PubKey::parse(&carol).unwrap()).await.unwrap());

        // Without an invitation: the request of today.
        let dave = Keys::generate().public_key().to_hex();
        let before = queued(&rt).await;
        assert_eq!(rt.dm_request(&dave, None, None).await.unwrap().mode, "request_sent");
        assert_eq!(queued(&rt).await - before, 0, "a quiet add sends nothing");
        assert_eq!(rt.dm_request(&dave, None, Some("hello")).await.unwrap().mode, "request_sent");
        assert_eq!(queued(&rt).await - before, 4);
        assert_eq!(code_of(rt.dm_request(&dave, Some("123456"), Some("again")).await), "dm_waiting_approval", "one text until approval, secret or not");
        assert_eq!(code_of(rt.dm_request(&dave, Some(""), None).await), "invite_secret");
        rt.shutdown().await;
    }

    fn code_of<T: std::fmt::Debug>(r: Result<T>) -> String {
        match r {
            Err(MessengerError::Invalid(m)) => m,
            other => panic!("{other:?}"),
        }
    }

    /// The invitations on the hidden rows of my signals in the chat with
    /// `peer`, in their order.
    async fn controls_of(rt: &MessengerRuntime, peer: &str) -> Vec<Option<String>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT envelope_json FROM msg_messages WHERE chat_id = ? AND content_type = 'control' ORDER BY created_at")
                .bind(messenger_store::chats::dm_chat_id(peer, None))
                .fetch_all(rt.store().pool())
                .await
                .unwrap();
        rows.iter().map(|j| messenger_core::Envelope::parse(j).unwrap().control_invite().map(String::from)).collect()
    }

    /// The phone reads the wire with a reader that shows `null` as a
    /// word: what is absent is left out.
    #[test]
    fn nothing_absent_is_written_as_null() {
        let page = RequestsPage { items: vec![], next_before: None, total: 0 };
        assert_eq!(serde_json::to_value(&page).unwrap(), serde_json::json!({ "items": [], "total": 0 }));
        let action = ModAction::Mute { who: "w".into(), until: None };
        assert_eq!(serde_json::to_value(&action).unwrap(), serde_json::json!({ "t": "mute", "who": "w" }));
        assert_eq!(serde_json::from_value::<ModAction>(serde_json::json!({ "t": "mute", "who": "w" })).unwrap(), action);
        assert_eq!(serde_json::to_value(RequestAction::Accept).unwrap(), serde_json::json!("accept"));
        let filter: QueueFilter = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(filter, QueueFilter::default());
    }
}

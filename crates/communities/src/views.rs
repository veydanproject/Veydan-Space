// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the commands hand the page and take from it. The core keeps rows
//! and states of its own making; these are their shapes for the UI, written
//! as TypeScript by `bindings` so the two cannot drift. A view that mirrors
//! a type of the core (`ModAction`, the operations of the journal) has the
//! same serde shape: the command reads it as the core's type.

use communities_core::db::channels::{ChannelRow, InviteRow, RequestRow};
use communities_core::db::content::UploadRow;
use communities_core::moderation::{LogLine, QueueLine};
use communities_core::db::levels::LevelRequestRow;
use communities_core::levels::{scope_shown, Names};
use std::collections::HashMap;
use communities_core::state::{ChannelState, Staff, BASE_LEVEL, MAX_LEVEL_ID, SCOPE_EVERYONE};
use communities_core::LevelCan;
use messenger_core::PubKey;
use communities_core::transport::{FeedItem, FeedPage, MdBlock};
use communities_core::link::ChannelLink;
use communities_core::wire::Card;
use communities_core::InviteStatus;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The chat of a channel in the list of the messenger, whose pin, mute and
/// archive are kept by the messenger under this id.
pub fn chat_id(community_id: &str) -> String {
    format!("channel:{community_id}")
}

/// The word serde writes for a value of the core (`open`, `from_join`, …).
pub(crate) fn word<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

// ─── Channels ───────────────────────────────────────────────────────────────

/// The newest post of a channel, for its row in the list of chats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LastPost {
    pub id: String,
    /// Hex pubkey.
    pub author: String,
    /// None for a file without a caption or a post that is gone.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub has_media: bool,
    /// A recording made in the app: `voice` or `circle` (the row says it by its kind); absent for any other file.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_kind: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
}

/// A channel as the list of chats shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChannelView {
    /// The id of the community (64 hex).
    pub id: String,
    /// `channel:<id>`: the chat whose pin, mute and archive the messenger keeps.
    pub chat_id: String,
    pub name: String,
    pub about: String,
    /// The address of the cover on a host of the channel.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Hex pubkey of the owner.
    pub owner: String,
    /// `open` | `request` | `invite`
    pub join: String,
    /// `none` | `joining` | `requested` | `joined` | `rejected` | `out` | `disbanded`
    pub membership: String,
    /// `owner` | `admin` | `moderator` | `author` | `member`; none for a reader.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_role: Option<String>,
    /// Readers by the newest snapshot of the keeper.
    #[ts(type = "number")]
    pub members: i64,
    pub relays: Vec<String>,
    pub storage: Vec<String>,
    pub link_epoch: u32,
    /// This device keeps the channel (its keeper runs here).
    pub kept_here: bool,
    /// Something of the channel is newer than this build reads.
    pub needs_update: bool,
    /// Why I am out: `left` | `removed` | `banned` | `disbanded`; none while in.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_reason: Option<String>,
    /// When the keeper was last heard of by every reader: the time of the
    /// newest key (epoch) it handed out, unix seconds.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper_last_seen: Option<i64>,
    #[ts(type = "number")]
    pub unread: i64,
    /// What the user chose of the chat `chat_id`, kept by the messenger
    /// (`messenger_chat_set_pinned`, `…_muted`, `…_archived`).
    pub pinned: bool,
    pub muted: bool,
    pub archived: bool,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastPost>,
    /// What the list of chats orders the row by and shows as its time: the
    /// date of `last`, or, with no post yet, when the channel came here
    /// (`created_at`). Opening, reading, comments, reactions, edits and the
    /// news of the journal never move it (owner 2026-10-23).
    #[ts(type = "number")]
    pub last_at: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

impl ChannelView {
    pub(crate) fn of(row: ChannelRow, kept_here: bool, unread: i64, prefs: Option<&veydan_shell::seams::ChatPref>, last: Option<LastPost>) -> Self {
        Self {
            last_at: last.as_ref().map_or(row.created_at, |l| l.at),
            pinned: prefs.is_some_and(|p| p.pinned),
            muted: prefs.is_some_and(|p| p.muted),
            archived: prefs.is_some_and(|p| p.archived),
            chat_id: chat_id(&row.id),
            picture: Some(row.picture).filter(|p| !p.is_empty()),
            relays: serde_json::from_str(&row.relays_json).unwrap_or_default(),
            storage: serde_json::from_str(&row.storage_json).unwrap_or_default(),
            link_epoch: u32::try_from(row.link_epoch).unwrap_or(0),
            id: row.id,
            name: row.name,
            about: row.about,
            owner: row.owner,
            join: row.join_mode,
            membership: row.membership,
            my_role: row.my_role,
            members: row.members,
            kept_here,
            needs_update: row.needs_update,
            out_reason: row.out_reason,
            keeper_last_seen: row.keeper_last_seen,
            unread,
            last,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// When readers may post: `days` are 1 (Monday) to 7, `from` and `to`
/// are `HH:MM` in `tz`. A schedule without days takes the schedule away.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScheduleView {
    pub tz: String,
    pub days: Vec<u8>,
    pub from: String,
    pub to: String,
}

/// Somebody the journal names: staff, or a reader it mutes or trusts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StaffView {
    /// Hex pubkey.
    pub pubkey: String,
    /// `owner` | `admin` | `moderator` | `author` | `member`
    pub role: String,
    pub muted: bool,
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_until: Option<i64>,
    pub trusted: bool,
    #[ts(type = "number")]
    pub since: i64,
    /// The levels `set_levels` gave (ids; the base level 0 is everybody's
    /// and left out); always given, optional to a page made before them.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<u8>>,
    /// Uploads into his own quota, not the owner's (`set_quota`,
    /// communities-wire §9.2): `true`; left out in the owner's.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_own: Option<bool>,
}

impl StaffView {
    fn of(pubkey: &str, s: &Staff) -> Self {
        Self {
            pubkey: pubkey.to_string(),
            role: s.role.as_str().to_string(),
            muted: s.muted,
            muted_until: s.muted_until.filter(|_| s.muted),
            trusted: s.trusted,
            since: s.since,
            levels: Some(level_ids(s.levels)),
            quota_own: s.quota_own.then_some(true),
        }
    }
}

/// A channel with everything its page shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChannelDetail {
    pub channel: ChannelView,
    /// `open` | `from_join`: how much of the past a newcomer reads.
    pub history: String,
    /// `all` | `authors`: who posts.
    pub post_mode: String,
    /// `on` | `off`
    pub comments: String,
    /// `off` | `newcomers` | `all`
    pub premod: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleView>,
    /// Ids of the pinned posts.
    pub pinned: Vec<String>,
    pub staff: Vec<StaffView>,
    pub disbanded: bool,
    /// A key of the channel is here; without one the page says «waiting
    /// for the keeper's key».
    pub has_key: bool,
    /// Posts and comments here that wait for a key newer than the one
    /// held: with `has_key` the page says «the key is old, catching up».
    pub waiting: u32,
    /// The keeper of the current series, hex pubkey.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper: Option<String>,
    /// The link to hand on (with its secret when this device holds it).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The sections of the channel, `everyone` first (communities-wire §14).
    /// These three are always given, optional to a page made before them.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<ScopeView>>,
    /// The levels, by id; 0 is the base level of everyone in.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<LevelView>>,
    /// The levels I hold (the base level 0 left out).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_levels: Option<Vec<u8>>,
    /// The channel handed to a new owner (wire §16): an offer to me not
    /// answered yet, or mine not taken yet.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferView>,
    /// What of mine in this channel still waits for the relays (always
    /// given; optional to a page made before it).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbox: Option<OutboxView>,
}

/// What a channel has in the outbox: operations and posts no relay took
/// yet (`queued`), those refused many times (`failed`, still tried), and
/// what the relays said last (`channel_outbox_retry` tries them now).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OutboxView {
    pub queued: u32,
    pub failed: u32,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<communities_core::db::outbox::Summary> for OutboxView {
    fn from(s: communities_core::db::outbox::Summary) -> Self {
        Self { queued: u32::try_from(s.queued).unwrap_or(u32::MAX), failed: u32::try_from(s.failed).unwrap_or(u32::MAX), error: s.oldest_error }
    }
}

/// A handover of the channel as the page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TransferView {
    /// `in`: offered to me, «Вам передают канал»; `out`: offered by me.
    #[ts(type = "\"in\" | \"out\"")]
    pub direction: String,
    /// The owner who offers it (`in`), the one offered it (`out`), hex.
    pub peer: String,
    /// `offered` | `declined`
    #[ts(type = "\"offered\" | \"declined\"")]
    pub status: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

impl ChannelDetail {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn of(
        channel: ChannelView,
        state: &ChannelState,
        has_key: bool,
        waiting: u32,
        link: Option<String>,
        names: &Names,
        me: Option<&PubKey>,
        unread: &HashMap<String, i64>,
    ) -> Self {
        Self {
            scopes: Some(ScopeView::all(state, names, me, unread)),
            levels: Some(LevelView::all(state, names, me, u32::try_from(channel.members).unwrap_or(0))),
            my_levels: Some(me.map(|m| level_ids(state.levels_of(m))).unwrap_or_default()),
            channel,
            history: word(&state.history),
            post_mode: word(&state.post_mode),
            comments: word(&state.comments),
            premod: word(&state.premod),
            schedule: state.schedule.as_ref().map(|s| ScheduleView { tz: s.tz.clone(), days: s.days.clone(), from: s.from.clone(), to: s.to.clone() }),
            pinned: state.pinned.iter().cloned().collect(),
            staff: state.staff.iter().map(|(k, s)| StaffView::of(k, s)).collect(),
            disbanded: state.disbanded,
            has_key,
            waiting,
            keeper: state.keeper().map(|k| k.pubkey.as_hex().to_string()),
            link,
            transfer: None,
            outbox: None,
        }
    }
}

/// The ids of the levels of a bit set, the base level (bit 0) left out:
/// everybody in holds it.
pub(crate) fn level_ids(bits: u64) -> Vec<u8> {
    (1..=MAX_LEVEL_ID).filter(|i| bits & (1u64 << i) != 0).collect()
}

// ─── Levels and sections (communities-wire §14) ─────────────────────────────

/// A section of the channel: a scope of its own keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScopeView {
    /// `everyone`, or 32 hex.
    pub scope_id: String,
    /// Sealed, read by members; empty for `everyone` unless the owner gave
    /// it one (the page names it).
    pub title: String,
    /// `sd-v1` | `flat-v1`
    #[ts(type = "\"sd-v1\" | \"flat-v1\"")]
    pub scheme: String,
    /// `open` | `from_join`
    #[ts(type = "\"open\" | \"from_join\"")]
    pub history: String,
    /// I belong to it: a level of mine gives it, or I am a moderator or above.
    pub mine: bool,
    /// I may post into it (2026-10-23): an author and above where I belong,
    /// a reader where a level of mine that lets post gives it.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<bool>,
    /// The look of the tile (2026-10-23), from the sealed meta: a key of
    /// the app's set of icons.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The section has a photo: its id (the hash of its blob), to cache by;
    /// `channel_scope_photo` gives the picture.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    /// One of the ten colours of the palette, 0…9.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
    /// Where the owner put it; the list comes sorted by it already.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    /// The line under the title on the tile.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Hidden: only its members are shown it (and they are, with this
    /// mark); its title and look are sealed in its own feed. Always given.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Posts in it not read yet, by the rule of the channel's count.
    /// Always given.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread: Option<u32>,
}

impl ScopeView {
    /// The sections shown to `me` (`levels::scope_shown`: a hidden one only
    /// to its members), `everyone` first, then by the owner's `order`,
    /// then as the journal keeps them; `unread` by scope.
    pub(crate) fn all(state: &ChannelState, names: &Names, me: Option<&PubKey>, unread: &HashMap<String, i64>) -> Vec<Self> {
        let mine = me.map(|m| state.scopes_of(m)).unwrap_or_default();
        let post = me.map(|m| state.post_scopes_of(m)).unwrap_or_default();
        let mut out: Vec<Self> = state
            .scopes
            .iter()
            .filter(|(id, _)| scope_shown(state, names, id, me))
            .map(|(id, def)| {
                let look = names.look(id);
                Self {
                    scope_id: id.clone(),
                    title: names.scope(id),
                    scheme: def.scheme.as_str().to_string(),
                    history: word(&def.history),
                    mine: mine.contains(id),
                    post: Some(post.contains(id)),
                    icon: look.icon,
                    photo: look.photo.map(|p| p.sha256),
                    color: look.color,
                    order: look.order,
                    note: look.note,
                    hidden: Some(def.hidden),
                    unread: Some(unread.get(id).map_or(0, |n| u32::try_from(*n).unwrap_or(u32::MAX))),
                }
            })
            .collect();
        // `everyone` first unless the owner ordered otherwise; the rest by
        // their order, then as the journal keeps them.
        out.sort_by_key(|s| (s.order.unwrap_or(0), s.scope_id != SCOPE_EVERYONE));
        out
    }
}

/// What a level lets do beyond reading its sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LevelRights {
    pub comment: bool,
    pub discuss: bool,
    pub post: bool,
}

impl From<LevelCan> for LevelRights {
    fn from(c: LevelCan) -> Self {
        Self { comment: c.comment, discuss: c.discuss, post: c.post }
    }
}

impl From<LevelRights> for LevelCan {
    fn from(c: LevelRights) -> Self {
        Self { comment: c.comment, discuss: c.discuss, post: c.post }
    }
}

/// A level: its name (sealed), the sections it opens, its rights, and how
/// many the journal gives it to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LevelView {
    pub id: u8,
    pub name: String,
    /// The sections it opens; left out: `everyone`, and a section shown
    /// me not (`levels::scope_shown`: not even the id of a hidden one).
    pub scopes: Vec<String>,
    pub can: LevelRights,
    /// How many people hold it by the journal (everybody in, for the base
    /// level: the readers the last snapshot counted).
    pub members: u32,
}

impl LevelView {
    pub(crate) fn all(state: &ChannelState, names: &Names, me: Option<&PubKey>, readers: u32) -> Vec<Self> {
        state
            .levels
            .iter()
            .map(|(id, l)| {
                let bit = 1u64 << id;
                // Everybody in holds the base level: the readers the last
                // snapshot counted.
                let held = if *id == BASE_LEVEL { readers } else { u32::try_from(state.staff.values().filter(|s| s.levels & bit != 0).count()).unwrap_or(u32::MAX) };
                Self {
                    id: *id,
                    name: names.level(*id),
                    scopes: l.scopes.iter().filter(|s| *s != SCOPE_EVERYONE && scope_shown(state, names, s, me)).cloned().collect(),
                    can: l.can.into(),
                    members: held,
                }
            })
            .collect()
    }
}

/// A request for a level waiting for a moderator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LevelRequestView {
    /// Hex pubkey.
    pub requester: String,
    pub level: u8,
    pub note: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

impl From<LevelRequestRow> for LevelRequestView {
    fn from(r: LevelRequestRow) -> Self {
        Self { requester: r.requester, level: u8::try_from(r.level).unwrap_or(u8::MAX), note: r.note, created_at: r.created_at }
    }
}

/// What `channel_scope_add` takes: a new section, its look and whether it
/// is hidden (the owner alone); its photo is set after
/// (`channel_scope_photo_set`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NewScopeInput {
    pub title: String,
    /// `sd-v1` (default) | `flat-v1`
    #[ts(optional, type = "\"sd-v1\" | \"flat-v1\"")]
    #[serde(default)]
    pub scheme: Option<String>,
    /// `open` (default) | `from_join`
    #[ts(optional, type = "\"open\" | \"from_join\"")]
    #[serde(default)]
    pub history: Option<String>,
    /// A key of the app's set of icons, `[a-z0-9-]{1,32}`.
    #[ts(optional)]
    #[serde(default)]
    pub icon: Option<String>,
    /// 0…9
    #[ts(optional)]
    #[serde(default)]
    pub color: Option<u8>,
    #[ts(optional)]
    #[serde(default)]
    pub order: Option<i32>,
    /// The line under the title on the tile, ≤ 80 characters.
    #[ts(optional)]
    #[serde(default)]
    pub note: Option<String>,
    #[ts(optional)]
    #[serde(default)]
    pub hidden: Option<bool>,
}

/// What `channel_scope_set` takes (the owner): what changes of a section;
/// a field left out stays. `icon` or `note` empty and `color` -1 take
/// that away, `no_photo` the photo; `hidden` hides or shows the section
/// (never `everyone`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScopeSetInput {
    #[ts(optional)]
    #[serde(default)]
    pub title: Option<String>,
    #[ts(optional)]
    #[serde(default)]
    pub icon: Option<String>,
    /// 0…9, or -1 for none.
    #[ts(optional)]
    #[serde(default)]
    pub color: Option<i16>,
    #[ts(optional)]
    #[serde(default)]
    pub order: Option<i32>,
    #[ts(optional)]
    #[serde(default)]
    pub note: Option<String>,
    #[ts(optional)]
    #[serde(default)]
    pub hidden: Option<bool>,
    #[ts(optional)]
    #[serde(default)]
    pub no_photo: Option<bool>,
}

/// What `channel_level_set` takes: a level made (no `id`: the lowest free)
/// or changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LevelInput {
    #[ts(optional)]
    #[serde(default)]
    pub id: Option<u8>,
    pub name: String,
    pub scopes: Vec<String>,
    pub can: LevelRights,
}

/// What a new channel is made with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NewChannelInput {
    pub name: String,
    #[ts(optional)]
    #[serde(default)]
    pub about: Option<String>,
    /// `open` | `request` | `invite`
    pub join: String,
    /// `open` | `from_join`; `open` when not given.
    #[ts(optional)]
    #[serde(default)]
    pub history: Option<String>,
    /// The relays of the channel: relays of the messenger's pool (those of
    /// the home set, as `messenger_relays_list` gives them), at least one.
    pub relays: Vec<String>,
    /// The blob hosts of the channel (`storage[]`): at least one.
    pub storage: Vec<String>,
}

// ─── The feed ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: u32,
    /// I am one of those who set it (on any of my devices).
    pub mine: bool,
}

/// A post or a comment as a page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FeedItemView {
    pub id: String,
    pub community_id: String,
    /// Of a comment: its post. Empty for a post.
    pub post_id: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Hex pubkey.
    pub author: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `md` when `text` is written in Markdown; absent for plain text.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmt: Option<String>,
    /// Of a Markdown `text`: its blocks, rendered as a chat's message is
    /// (MessageContent); absent when plain, or too long to be Markdown.
    #[ts(optional, type = "Array<MdBlock>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich: Option<Vec<MdBlock>>,
    /// The media envelope (`t: media`) as JSON, when it is a file.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_json: Option<String>,
    /// `ok` | `pending` | `approved` | `rejected` | `released`
    pub review: String,
    pub hidden: bool,
    pub deleted: bool,
    pub pinned: bool,
    pub reactions: Vec<ReactionCount>,
    /// Of a post: how many comments are here.
    pub comments: u32,
    /// The section it is in (`everyone`, or the id of a scope); always
    /// given, optional to a page made before the sections.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// When it was edited last (the edit's own date, unix seconds); absent
    /// for a message never edited. Of a file's post, an edit is its caption.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    /// Mine, and no relay has it yet: shown as on its way until
    /// `channel.outbox` says it went (or failed).
    #[serde(default)]
    pub pending: bool,
}

impl From<FeedItem> for FeedItemView {
    fn from(i: FeedItem) -> Self {
        Self {
            pending: i.pending,
            id: i.id,
            community_id: i.community_id,
            post_id: i.post_id,
            parent_id: i.parent_id,
            author: i.author,
            created_at: i.created_at,
            text: i.text,
            fmt: i.fmt,
            rich: i.rich,
            media_json: i.media_json,
            review: i.review,
            hidden: i.hidden,
            deleted: i.deleted,
            pinned: i.pinned,
            reactions: i.reactions.into_iter().map(|(emoji, n, mine)| ReactionCount { emoji, count: u32::try_from(n).unwrap_or(0), mine }).collect(),
            comments: u32::try_from(i.comments).unwrap_or(0),
            scope_id: Some(i.scope_id),
            edited_at: i.edited_at,
        }
    }
}

/// Where the next page of the feed or of a thread starts: the last item
/// of a page, by its date and id, so items of one second are not skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FeedCursorView {
    #[ts(type = "number")]
    pub created_at: i64,
    pub id: String,
}

/// A page of the feed or of a thread. `asked`: the relays were asked for
/// more; what they send comes as `channel.message`, and the page read
/// again shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FeedPageView {
    pub items: Vec<FeedItemView>,
    /// The date of `next` alone, for a page that still pages by `before`.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before: Option<i64>,
    /// What to pass as `next` for the next page.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<FeedCursorView>,
    pub asked: bool,
}

impl From<FeedPage> for FeedPageView {
    fn from(p: FeedPage) -> Self {
        Self {
            items: p.items.into_iter().map(Into::into).collect(),
            next_before: p.next_before,
            next: p.next.map(|c| FeedCursorView { created_at: c.created_at, id: c.id }),
            asked: p.asked,
        }
    }
}

/// The card of a channel shown from its link before joining
/// (`channel_preview_link`): what the owner's signed card says, and what
/// the link lets me do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChannelPreview {
    pub community_id: String,
    pub name: String,
    pub about: String,
    /// The cover's address on a host of the channel.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Readers by the last snapshot: an estimate.
    #[ts(type = "number")]
    pub members: u64,
    /// `open` | `request` | `invite`.
    pub join: String,
    /// Hex pubkey of the owner, whose signature the card carries.
    pub owner: String,
    /// The relay of the link the card came from.
    pub relay: String,
    /// The link carries the secret of the channel: it lets in at once.
    pub has_secret: bool,
    /// The link names an invitation.
    pub invited: bool,
    /// Where I stand, when the channel is here already (`joined`, `requested`, …).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<String>,
}

impl ChannelPreview {
    pub(crate) fn of(card: Card, link: &ChannelLink, here: Option<ChannelRow>) -> Self {
        Self {
            community_id: card.community_id,
            name: card.name,
            about: card.about,
            picture: Some(card.picture).filter(|p| !p.is_empty()),
            members: card.members,
            join: word(&card.join),
            owner: link.owner.as_hex().to_string(),
            relay: link.relay.as_str().to_string(),
            has_secret: link.secret.is_some(),
            invited: link.invite_id.is_some(),
            membership: here.map(|r| r.membership),
        }
    }
}

/// The folder the copies of the files a phone hands over by their URI wait
/// in until they are sent; a copy is named `<unix>-<name>`.
pub(crate) const OUTGOING_DIR: &str = "outgoing";

/// The name of a file on its way as its sender knows it: a copy waiting in
/// `OUTGOING_DIR` loses the time it was made at.
pub(crate) fn upload_name(path: &str) -> String {
    let p = std::path::Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let copied = p.parent().and_then(|d| d.file_name()).is_some_and(|d| d == OUTGOING_DIR);
    match name.split_once('-') {
        Some((stamp, rest)) if copied && !rest.is_empty() && stamp.bytes().all(|b| b.is_ascii_digit()) => rest.to_string(),
        _ => name,
    }
}

/// A file of mine on its way to the hosts of a channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UploadView {
    pub id: String,
    pub community_id: String,
    /// The name of the file, as the page shows it while it goes.
    pub name: String,
    /// The post a comment goes under; empty for a post.
    pub post_id: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// `uploading` | `paused` | `failed`
    pub status: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[ts(type = "number")]
    pub done: i64,
    #[ts(type = "number")]
    pub total: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    /// A recording made in the app: `voice` or `circle` (`channel_media_record`); absent for a file picked.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl From<UploadRow> for UploadView {
    fn from(u: UploadRow) -> Self {
        Self {
            name: upload_name(&u.path),
            kind: communities_core::media::UploadMeta::recorded_kind(&u.meta_json),
            id: u.id,
            community_id: u.community_id,
            post_id: u.post_id,
            parent_id: u.parent_id,
            caption: u.caption,
            status: u.status,
            error: u.error,
            done: u.done,
            total: u.total,
            created_at: u.created_at,
        }
    }
}

// ─── Getting in ─────────────────────────────────────────────────────────────

/// An invitation to a channel: one that came to me (`in`) or one I sent (`out`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InviteView {
    pub invite_id: String,
    pub community_id: String,
    /// `in` | `out`
    pub direction: String,
    /// The other side, hex pubkey: who invited me, or whom I invited.
    pub peer: String,
    /// `sent` | `accepted` | `declined` | `done` | `expired` | `cancelled`
    pub status: String,
    /// The name of the channel as the invitation carries it.
    pub name: String,
    pub about: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
    /// Of one that came: the journal here bears out that its sender is a
    /// moderator; until then the page says it is not checked yet.
    pub verified: bool,
}

impl From<InviteRow> for InviteView {
    fn from(r: InviteRow) -> Self {
        let payload: serde_json::Value = serde_json::from_str(&r.payload_json).unwrap_or_default();
        let text = |k: &str| payload[k].as_str().unwrap_or_default().to_string();
        Self {
            name: text("name"),
            about: text("about"),
            picture: Some(text("picture")).filter(|p| !p.is_empty()),
            invite_id: r.invite_id,
            community_id: r.community_id,
            direction: r.direction,
            peer: r.peer,
            status: r.status,
            created_at: r.created_at,
            expires_at: r.expires_at,
            verified: r.verified,
        }
    }
}

/// A request to be let into a channel, waiting for a moderator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RequestView {
    pub community_id: String,
    /// Hex pubkey of who asks.
    pub requester: String,
    /// `pending` | `approved` | `rejected`
    pub status: String,
    /// What they wrote with it.
    pub note: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

impl From<RequestRow> for RequestView {
    fn from(r: RequestRow) -> Self {
        let payload: serde_json::Value = serde_json::from_str(&r.payload_json).unwrap_or_default();
        Self {
            note: payload["note"].as_str().unwrap_or_default().to_string(),
            community_id: r.community_id,
            requester: r.requester,
            status: r.status,
            created_at: r.created_at,
        }
    }
}

// ─── Moderation ─────────────────────────────────────────────────────────────

/// A message waiting for a moderator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct QueueItem {
    pub message_id: String,
    /// Hex pubkey.
    pub author: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
    /// The version an approval names.
    pub version: String,
    /// How many more of the same author wait behind it.
    pub more_from_author: u32,
}

impl QueueItem {
    pub(crate) fn of(line: QueueLine, text: Option<String>) -> Self {
        Self { message_id: line.id, author: line.author, text, at: line.created_at, version: line.version, more_from_author: line.more_from_author }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct QueuePage {
    pub items: Vec<QueueItem>,
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before: Option<i64>,
    /// How many messages wait (of the author asked for, when one was).
    pub total: u32,
}

/// One line of the log of moderation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LogEntry {
    #[ts(type = "number")]
    pub at: i64,
    /// Hex pubkey of who acted.
    pub by: String,
    /// `hide` | `unhide` | `approve` | `reject` | `ban` | `unban` | `mute`
    /// | `unmute` | `trust` | `untrust` | `remove` | `pin` | `unpin` | `settings`
    pub action: String,
    /// The message, the post or the person; empty for settings.
    pub target: String,
    /// JSON with what else the line says.
    pub detail: String,
    /// Of a line about a post or a comment here: the start of its words, so
    /// the log names it as a reader would; none for a person, the rules, or
    /// a message gone from this device.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

impl From<LogLine> for LogEntry {
    fn from(l: LogLine) -> Self {
        Self { at: l.at, by: l.by, action: l.action, target: l.target, detail: l.detail, excerpt: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LogPage {
    pub items: Vec<LogEntry>,
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before: Option<i64>,
}

/// The rules of a channel the moderation page sets. Every field left out
/// stays as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ModSettings {
    /// `authors`, always in stage 1 (wire §3, 2026-10-22); one sent is not
    /// applied.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_mode: Option<String>,
    /// `on` | `off`
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<String>,
    /// `off` | `newcomers` | `all`
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premod: Option<String>,
    /// A schedule without days takes the schedule away.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleView>,
    /// `open` | `request` | `invite`
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
    /// `open` | `from_join`
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

/// One act of moderation, tagged by `t` as the groups' is: the core's
/// `ModAction` word for word.
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
    /// `version` empty: the version held now.
    Approve {
        target: String,
        #[serde(default)]
        version: String,
    },
    Reject {
        target: String,
    },
    /// Ban the author when I may, and hide what they wrote.
    Spam {
        author: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    /// `until` absent: for good.
    Mute {
        who: String,
        #[ts(optional, type = "number")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// An operation of the journal the page asks for (`channel_act`), tagged
/// by `op` as the journal writes it: the part of the core's `OpBody` a
/// page makes. Keys are hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
// Read once per command and handed on: the size of `Settings` costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum ChannelAction {
    /// The card and the settings; every field left out stays as it is.
    Settings {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<String>,
        /// `open` | `from_join`
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relays: Option<Vec<String>>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        storage: Option<Vec<String>>,
        /// `all` | `authors`
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_mode: Option<String>,
        /// `off` | `newcomers` | `all`
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        premod: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schedule: Option<ScheduleView>,
        /// `on` | `off`
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        comments: Option<String>,
        /// `open` | `request` | `invite`
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join: Option<String>,
    },
    /// `member` takes the role away.
    SetRole { who: String, role: String },
    Ban {
        who: String,
        /// `spam` or nothing.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Unban {
        who: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Out without a ban.
    Remove { who: String },
    SetMuted {
        who: String,
        muted: bool,
        #[ts(optional, type = "number")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<i64>,
    },
    SetTrusted { who: String, trusted: bool },
    /// Whose quota a member of the staff uploads into: his own (`own`), or
    /// the owner's, as everyone starts (communities-wire §9.2; the owner).
    SetQuota { who: String, own: bool },
    /// Asks the keeper for an epoch now.
    RotateNow {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope_id: Option<String>,
    },
    Pin { post_id: String, pinned: bool },
    TransferOwnership { to: String },
    /// Leave the channel (not an operation: a note to the keeper).
    Leave,
    Disband,
}

/// The invitations open now against their limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InviteStatusView {
    /// Open invitations of mine, of every channel.
    pub open: u32,
    pub max_open: u32,
    /// Open invitations of this channel.
    pub of_channel: u32,
    pub max_per_channel: u32,
    pub max_per_person: u32,
}

impl From<InviteStatus> for InviteStatusView {
    fn from(s: InviteStatus) -> Self {
        let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
        Self { open: n(s.open), max_open: n(s.max_open), of_channel: n(s.of_channel), max_per_channel: n(s.max_per_channel), max_per_person: n(s.max_per_person) }
    }
}

/// The tickets a moderator holds (communities-wire §17): handed to a
/// newcomer while the keeper is away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TicketsView {
    /// Tickets held and good for a while yet.
    pub held: u32,
    /// The most a moderator holds at once.
    pub max: u32,
}

/// Where the files of a new channel of mine live, by the account (the
/// owner's rule of 2026-10-23, communities-wire §9): without Veydan Cloud
/// they go to the free public media of the messenger, as a chat file does,
/// kept `retention_days`; a long history (`from_join`) needs Cloud or a
/// server of mine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StoragePlanView {
    /// The account has no Veydan Cloud that keeps files longer than the
    /// free media: the files of its channels go away after `retention_days`.
    pub free: bool,
    /// How long the media of the account keeps a file, days; 0: no limit.
    pub retention_days: u32,
}

/// The keeper of a channel as its page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KeeperStatus {
    /// This device keeps the channel; the rest is known only then.
    pub kept_here: bool,
    /// The keeper runs: the module is on and has a session.
    pub online: bool,
    /// The keeper of the current series, hex pubkey.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper: Option<String>,
    /// The newest epoch published.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u32>,
    /// An epoch built and still waiting for its hosts or its relays.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_epoch: Option<u32>,
    /// When the newest epoch was built, unix seconds.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_epoch_at: Option<i64>,
    /// When the cycle runs next, unix seconds.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_wake: Option<i64>,
    /// Readers the keeper holds.
    pub readers: u32,
    /// Live generations of the tree (two during a migration).
    pub generations: u32,
    /// Blobs waiting to be removed from the hosts.
    #[ts(type = "number")]
    pub garbage: i64,
    /// An epoch asked for now (`channel_rotate_now`) and not built yet.
    pub rotate_asked: bool,
    /// The phone runs the keeper with the app off the screen (Android):
    /// the owner's switch is shown. False on a desktop, whose keeper runs
    /// while the app does.
    pub background: bool,
    /// The owner keeps the keeper of this channel on (`channel_keeper_keep_on`).
    pub keep_on: bool,
    /// The keeper's service of the phone runs now: «работает в фоне».
    pub background_running: bool,
    /// I am the owner: «Сделать хранителем это устройство» is mine to press
    /// on a device that does not keep the channel (wire §15).
    #[serde(default)]
    pub owner: bool,
    /// This device waits for the state of the keeper it takes over:
    /// `handover` (from the device before) or `emergency` (recovering).
    #[ts(optional, type = "\"handover\" | \"emergency\"")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taking: Option<String>,
    /// Since when it waits, unix seconds.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taking_since: Option<i64>,
    /// The keeper kept here belongs to a series the journal has gone past:
    /// it hands its state over and acts no more («Передаём…»).
    #[serde(default)]
    pub handing: bool,
    /// This device holds the root of the channel: the keeper can be
    /// recovered here if the device that keeps it is lost.
    #[serde(default)]
    pub has_root: bool,
    /// What a blob host said when it last refused a blob of the keeper kept
    /// here, until every host takes them again: «Хранилище отказало: …».
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields of stage 3 (wire §15–§17) are read from a status of before
    /// them as nothing moving, and written only when there is something.
    #[test]
    fn the_keeper_status_of_stage_3_reads_an_older_one() {
        let old = serde_json::json!({ "kept_here": true, "online": true, "readers": 3, "generations": 1, "garbage": 0, "rotate_asked": false,
            "background": false, "keep_on": false, "background_running": false });
        let s: KeeperStatus = serde_json::from_value(old).unwrap();
        assert!(!s.owner && !s.handing && !s.has_root && s.taking.is_none());
        let json = serde_json::to_value(KeeperStatus { taking: Some("handover".into()), taking_since: Some(5), ..s }).unwrap();
        assert_eq!((json["taking"].as_str(), json["taking_since"].as_i64()), (Some("handover"), Some(5)));
        let t = serde_json::to_value(TicketsView { held: 12, max: 50 }).unwrap();
        assert_eq!(t, serde_json::json!({ "held": 12, "max": 50 }));
        let tr = TransferView { direction: "in".into(), peer: "ab".repeat(32), status: "offered".into(), created_at: 1 };
        assert_eq!(serde_json::to_value(&tr).unwrap()["direction"], "in");
    }

    /// A copy made for sending is shown by the name it was picked with; a
    /// file sent from where it lies keeps its own (ревью UX п. 9).
    #[test]
    fn a_file_on_its_way_has_the_name_it_was_picked_with() {
        assert_eq!(upload_name("/data/communities-files/outgoing/1792600000-photo-1.jpg"), "photo-1.jpg");
        assert_eq!(upload_name("/home/me/2026-report.pdf"), "2026-report.pdf");
        assert_eq!(upload_name("/data/communities-files/outgoing/noprefix.png"), "noprefix.png");
    }

    /// The mirrors read and write as the core's types do: a command takes
    /// the page's value as the core's.
    #[test]
    fn the_mirrors_have_the_serde_shape_of_the_core() {
        let key = "ab".repeat(32);
        let acts = [
            ModAction::Hide { targets: vec![key.clone()] },
            ModAction::HideAuthor { author: key.clone(), from: 1, to: 2 },
            ModAction::Approve { target: key.clone(), version: String::new() },
            ModAction::Spam { author: key.clone(), target: None },
            ModAction::Mute { who: key.clone(), until: Some(5) },
            ModAction::Trust { who: key.clone(), trusted: true },
        ];
        for act in acts {
            let core: communities_core::ModAction = serde_json::from_value(serde_json::to_value(&act).unwrap()).unwrap();
            let back: ModAction = serde_json::from_value(serde_json::to_value(core).unwrap()).unwrap();
            assert_eq!(back, act);
        }
        let ops = [
            ChannelAction::Settings {
                name: Some("n".into()),
                about: None,
                history: Some("from_join".into()),
                relays: None,
                storage: None,
                post_mode: Some("authors".into()),
                premod: Some("newcomers".into()),
                schedule: Some(ScheduleView { tz: "UTC".into(), days: vec![1], from: "09:00".into(), to: "18:00".into() }),
                comments: Some("off".into()),
                join: Some("request".into()),
            },
            ChannelAction::SetRole { who: key.clone(), role: "moderator".into() },
            ChannelAction::Ban { who: key.clone(), reason: Some("spam".into()) },
            ChannelAction::SetMuted { who: key.clone(), muted: true, until: None },
            ChannelAction::RotateNow { scope_id: None },
            ChannelAction::Pin { post_id: key.clone(), pinned: true },
            ChannelAction::TransferOwnership { to: key.clone() },
            ChannelAction::Disband,
        ];
        for op in ops {
            let core: communities_core::OpBody = serde_json::from_value(serde_json::to_value(&op).unwrap()).unwrap();
            let back: ChannelAction = serde_json::from_value(serde_json::to_value(core).unwrap()).unwrap();
            assert_eq!(back, op);
        }
    }

    /// The tiles of the sections (2026-10-23): a hidden one is shown to its
    /// members only, with its mark; one whose meta I cannot read is not
    /// shown; `everyone` first, then by the owner's order; the look and
    /// the unread of each come along.
    #[test]
    fn the_sections_shown_are_mine_or_open_and_come_in_the_owners_order() {
        use communities_core::wire::{ScopePhoto, ScopeLook};
        use communities_core::{History, JoinMode, KeeperRef, Op, OpBody, OpId, SchemeId};
        let cid = "aa".repeat(32);
        let pk = |c: &str| PubKey::parse(&c.repeat(64)).unwrap();
        let (owner, reader) = (pk("0"), pk("c"));
        let create = Op::new(&cid, &owner, vec![], 1, OpBody::Create {
            join: JoinMode::Open,
            name: "News".into(),
            about: String::new(),
            picture: String::new(),
            history: History::Open,
            relays: vec!["wss://r1.example".into()],
            storage: vec!["https://s1.example".into()],
            scheme: SchemeId::SdV1,
            keeper: KeeperRef { pubkey: pk("f"), serial: 0, from_epoch: 1, inbox: None },
            link_epoch: 0,
        });
        let mut state = ChannelState::genesis(&create).unwrap();
        let (open, secret, unnamed) = ("01".repeat(16), "02".repeat(16), "03".repeat(16));
        for (i, (id, hidden)) in [(&open, false), (&secret, true), (&unnamed, false)].into_iter().enumerate() {
            let op = Op::new(&cid, &owner, vec![OpId(format!("{i:064}"))], 2, OpBody::ScopeAdd { scope_id: id.clone(), scheme: SchemeId::SdV1, history: History::Open, hidden });
            state.apply(&op).unwrap();
        }
        let mut names = Names::default();
        names.scopes.insert(open.clone(), "Lounge".into());
        names.scopes.insert(secret.clone(), "Backstage".into());
        let photo = ScopePhoto { sha256: "cd".repeat(32), key: "k".into() };
        names.looks.insert(open.clone(), ScopeLook { icon: Some("star".into()), color: Some(4), order: Some(-1), note: Some("Hi".into()), photo: Some(photo) });
        names.looks.insert(SCOPE_EVERYONE.into(), ScopeLook { order: Some(0), ..ScopeLook::default() });
        let unread = HashMap::from([(open.clone(), 3), (SCOPE_EVERYONE.to_string(), 1)]);
        // A reader: everyone and the open section, ordered by the owner.
        let seen = ScopeView::all(&state, &names, Some(&reader), &unread);
        assert_eq!(seen.iter().map(|s| s.scope_id.as_str()).collect::<Vec<_>>(), [open.as_str(), SCOPE_EVERYONE]);
        let lounge = &seen[0];
        assert_eq!((lounge.icon.as_deref(), lounge.color, lounge.note.as_deref(), lounge.photo.as_deref()), (Some("star"), Some(4), Some("Hi"), Some("cd".repeat(32).as_str())));
        assert_eq!((lounge.unread, lounge.hidden, lounge.mine), (Some(3), Some(false), false));
        assert_eq!(seen[1].unread, Some(1));
        // The owner belongs to every section: the hidden one with its mark,
        // the unnamed one too.
        let seen = ScopeView::all(&state, &names, Some(&owner), &HashMap::new());
        assert_eq!(seen.len(), 4);
        assert_eq!(seen.iter().find(|s| s.scope_id == secret).and_then(|s| s.hidden), Some(true));
        assert!(seen.iter().all(|s| s.unread == Some(0)));
        // A level that gives the hidden section names it to the owner only.
        let gold = Op::new(&cid, &owner, vec![OpId("9".repeat(64))], 3, OpBody::LevelSet { id: 1, scopes: vec![secret.clone(), open.clone()], can: LevelCan::BASE });
        state.apply(&gold).unwrap();
        let levels = LevelView::all(&state, &names, Some(&reader), 0);
        assert_eq!(levels.iter().find(|l| l.id == 1).map(|l| l.scopes.clone()), Some(vec![open.clone()]));
        let levels = LevelView::all(&state, &names, Some(&owner), 0);
        assert_eq!(levels.iter().find(|l| l.id == 1).map(|l| l.scopes.len()), Some(2));
    }
}

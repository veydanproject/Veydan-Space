// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The levels and the scopes of a channel on a device
//! (internal/communities-wire.md §14): the operations with their sealed
//! names, and which name the journal gives each level and scope now.
//!
//! The journal holds the ids, the scopes a level gives and its rights; a
//! name of a level and the title of a scope travel sealed in the feed of
//! `everyone` (`t:"meta"`), so that the relays see that sections exist
//! and nothing of what they are called.

use crate::db::{self, levels::{LevelRequestRow, REQ_GRANTED, REQ_PENDING, REQ_REFUSED}};
use crate::log::OpLog;
use crate::op::{History, LevelCan, Op, OpBody, OpId};
use crate::service::{ChannelService, Outcome, UI_EVENT_CHANNEL_UPDATED};
use crate::state::{ChannelState, BASE_LEVEL, MAX_LEVEL_ID, MAX_LEVEL_NAME_CHARS, MAX_SCOPES, MAX_SCOPE_TITLE_CHARS, SCOPE_EVERYONE};
use keydist::SchemeId;
use crate::wire::{self, LevelRequest, LevelResolve, Meta, Payload, Place, ScopeLook, ScopePhoto};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;
use std::collections::{BTreeMap, HashMap};

/// A request for a level came to me as a moderator, or one was answered
/// (wire §14.4): `{community_id, requester}`.
pub const UI_EVENT_CHANNEL_LEVEL_REQUEST: &str = "channel.level_request";

/// The name of the base level until the owner gives it one.
pub const BASE_LEVEL_NAME: &str = "Базовый";

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel: {what}"))
}

/// The names the journal gives now: the titles of the scopes and the names
/// of the levels (`everyone` and a level without a name of its own have
/// none here; the base level has `BASE_LEVEL_NAME`), and the look of each
/// scope whose meta carries one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    pub scopes: BTreeMap<String, String>,
    pub looks: BTreeMap<String, ScopeLook>,
    pub levels: BTreeMap<u8, String>,
}

impl Names {
    pub fn level(&self, id: u8) -> String {
        match self.levels.get(&id) {
            Some(n) => n.clone(),
            None if id == BASE_LEVEL => BASE_LEVEL_NAME.to_string(),
            None => String::new(),
        }
    }

    pub fn scope(&self, id: &str) -> String {
        self.scopes.get(id).cloned().unwrap_or_default()
    }

    /// The look of a scope by its meta in force (none: the default).
    pub fn look(&self, id: &str) -> ScopeLook {
        self.looks.get(id).cloned().unwrap_or_default()
    }

    /// Whether the meta of a scope is here: its title, or a look of
    /// `everyone` (which has no title of its own).
    pub fn knows(&self, id: &str) -> bool {
        self.scopes.get(id).is_some_and(|t| !t.is_empty()) || self.looks.contains_key(id)
    }
}

/// Whether a section is shown to `me` at all (2026-10-23): `everyone`
/// always; a hidden one only to its members; one I am not in whose meta I
/// cannot read (none sealed where I read) neither — it would be a tile of
/// nothing.
pub fn scope_shown(state: &ChannelState, names: &Names, scope_id: &str, me: Option<&PubKey>) -> bool {
    if scope_id == SCOPE_EVERYONE {
        return true;
    }
    let Some(def) = state.scopes.get(scope_id) else { return false };
    me.is_some_and(|m| state.in_scope(m, scope_id)) || (!def.hidden && names.knows(scope_id))
}

/// What `scope_set` changes of a section (2026-10-23); a field left out
/// stays as the meta in force says. `icon` and `note` empty, `photo` and
/// `color` `Some(None)` take that away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopeChange {
    pub title: Option<String>,
    pub icon: Option<String>,
    pub photo: Option<Option<ScopePhoto>>,
    pub color: Option<Option<u8>>,
    pub order: Option<i32>,
    pub note: Option<String>,
    pub hidden: Option<bool>,
}

impl ScopeChange {
    /// The title and the look with this change made.
    pub fn apply(&self, title: &mut String, look: &mut ScopeLook) {
        let words = |s: &String| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(t) = &self.title {
            *title = t.trim().to_string();
        }
        if let Some(i) = &self.icon {
            look.icon = words(i);
        }
        if let Some(p) = &self.photo {
            look.photo = p.clone();
        }
        if let Some(c) = self.color {
            look.color = c;
        }
        if let Some(o) = self.order {
            look.order = Some(o);
        }
        if let Some(n) = &self.note {
            look.note = words(n);
        }
    }
}

/// Whether two names of levels are the same to a person: case and the
/// spaces around do not count.
fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl ChannelService {
    /// An operation of the scopes or the levels with its name (wire
    /// §14.1): the operation in the open, the name sealed into the feed of
    /// `everyone` by me (`t:"meta"`), kept here at once. A name of a level
    /// that another level of the channel has already is refused, whatever
    /// its case.
    pub async fn act_named(&self, keys: &Keys, community_id: &str, body: OpBody, name: &str) -> Result<Outcome> {
        self.act_meta(keys, community_id, body, name, ScopeLook::default()).await
    }

    /// `act_named` with the look of a scope (2026-10-23). The meta of a
    /// scope hidden by the operation (or hidden already, for a `scope_set`
    /// that leaves it so) is sealed into the feed of that scope, not of
    /// `everyone`: those outside it never read what it is.
    pub(crate) async fn act_meta(&self, keys: &Keys, community_id: &str, body: OpBody, name: &str, look: ScopeLook) -> Result<Outcome> {
        let name = name.trim().to_string();
        let (scope_id, level, max) = match &body {
            OpBody::ScopeAdd { scope_id, .. } | OpBody::ScopeSet { scope_id, .. } => (Some(scope_id.clone()), None, MAX_SCOPE_TITLE_CHARS),
            OpBody::LevelSet { id, .. } => (None, Some(*id), MAX_LEVEL_NAME_CHARS),
            _ => return Err(invalid("a name belongs to a scope or a level")),
        };
        // `everyone` has no title of its own unless the owner gives it one.
        let untitled = scope_id.as_deref() == Some(SCOPE_EVERYONE) && name.is_empty();
        if !untitled && wire::check_meta_name(&name, max).is_err() {
            return Err(invalid(if level.is_some() { "level name" } else { "scope title" }));
        }
        if level.is_some() && !look.is_empty() {
            return Err(invalid("a level has no look"));
        }
        look.check().map_err(|what| invalid(&format!("scope {what}")))?;
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        if let Some(id) = level {
            let names = self.names(&log).await?;
            if log.state().levels.keys().any(|other| *other != id && same_name(&names.level(*other), &name)) {
                return Err(invalid("level name"));
            }
        }
        let seal_in = match &body {
            OpBody::ScopeAdd { scope_id, hidden: true, .. } | OpBody::ScopeSet { scope_id, hidden: Some(true) } => scope_id.clone(),
            OpBody::ScopeSet { scope_id, hidden: None } if log.state().scopes.get(scope_id).is_some_and(|d| d.hidden) => scope_id.clone(),
            _ => SCOPE_EVERYONE.to_string(),
        };
        let (op, out) = self.act_op(keys, community_id, body).await?;
        let me = Self::me(keys)?;
        let meta = Meta { op_id: op.id().0, scope_id, level, name, look };
        self.keep_meta(community_id, &me, &meta, op.created_at).await?;
        db::levels::owe_meta(&self.env.db, community_id, &meta.op_id, &serde_json::to_string(&meta)?, &seal_in).await?;
        self.publish_owed_metas(keys).await?;
        Ok(out)
    }

    /// A new section: a scope of a fresh id with its title. More than
    /// `MAX_SCOPES` (with `everyone`) is refused before anything goes out.
    pub async fn scope_add(&self, keys: &Keys, community_id: &str, title: &str, scheme: SchemeId, history: History) -> Result<(String, Outcome)> {
        self.scope_add_with(keys, community_id, title, ScopeLook::default(), scheme, history, false).await
    }

    /// `scope_add` with the look of the section, and hidden (the owner
    /// alone): one operation `scope_add` with `hidden`, its meta sealed into
    /// the section's own feed once its first key is here (wire §14.1,
    /// 2026-10-23).
    #[allow(clippy::too_many_arguments)]
    pub async fn scope_add_with(&self, keys: &Keys, community_id: &str, title: &str, look: ScopeLook, scheme: SchemeId, history: History, hidden: bool) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        if state.scopes.len() >= MAX_SCOPES {
            return Err(invalid("scope limit"));
        }
        if hidden && state.owner != Self::me(keys)? {
            return Err(invalid("not permitted"));
        }
        let mut raw = [0u8; 16];
        getrandom::fill(&mut raw).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let scope_id = hex::encode(raw);
        let out = self.act_meta(keys, community_id, OpBody::ScopeAdd { scope_id: scope_id.clone(), scheme, history, hidden }, title, look).await?;
        Ok((scope_id, out))
    }

    /// The title, the look or the hiding of a section changes (the owner
    /// alone, wire §14.1 `scope_set`): what `change` leaves out stays as the
    /// meta in force says; the whole of it goes out as a fresh meta. A photo
    /// replaced or taken away is removed from the hosts.
    pub async fn scope_set(&self, keys: &Keys, community_id: &str, scope_id: &str, change: &ScopeChange) -> Result<Outcome> {
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        let state = log.state().clone();
        if !state.scopes.contains_key(scope_id) {
            return Err(invalid("unknown scope"));
        }
        if state.owner != Self::me(keys)? {
            return Err(invalid("not permitted"));
        }
        if scope_id == SCOPE_EVERYONE && change.hidden == Some(true) {
            return Err(invalid("scope everyone"));
        }
        let names = self.names(&log).await?;
        let (mut title, mut look) = (names.scope(scope_id), names.look(scope_id));
        let before = look.photo.clone();
        change.apply(&mut title, &mut look);
        let photo = look.photo.clone();
        let out = self.act_meta(keys, community_id, OpBody::ScopeSet { scope_id: scope_id.to_string(), hidden: change.hidden }, &title, look).await?;
        if let Some(old) = before.filter(|b| Some(b) != photo.as_ref()) {
            self.drop_scope_photo(keys, &state.storage, &old).await;
        }
        Ok(out)
    }

    /// The sections in this order (the owner): `order` of each becomes its
    /// place in `ids`, by a `scope_set` of each whose order differs.
    pub async fn scopes_order(&self, keys: &Keys, community_id: &str, ids: &[String]) -> Result<Outcome> {
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        let distinct: std::collections::BTreeSet<&String> = ids.iter().collect();
        if distinct.len() != ids.len() || ids.iter().any(|id| !log.state().scopes.contains_key(id)) {
            return Err(invalid("unknown scope"));
        }
        let names = self.names(&log).await?;
        let mut out = Outcome::default();
        for (i, id) in ids.iter().enumerate() {
            let want = i32::try_from(i).unwrap_or(i32::MAX);
            if names.look(id).order != Some(want) {
                out.merge(self.scope_set(keys, community_id, id, &ScopeChange { order: Some(want), ..ScopeChange::default() }).await?);
            }
        }
        Ok(out)
    }

    /// A section goes (the owner alone); `everyone` never.
    pub async fn scope_del(&self, keys: &Keys, community_id: &str, scope_id: &str) -> Result<Outcome> {
        if scope_id == SCOPE_EVERYONE {
            return Err(invalid("scope everyone"));
        }
        self.act(keys, community_id, OpBody::ScopeDel { scope_id: scope_id.to_string() }).await
    }

    /// A level made (`id` none: the lowest id free, at most 63) or changed,
    /// with its name; its id.
    pub async fn level_set(&self, keys: &Keys, community_id: &str, id: Option<u8>, name: &str, scopes: Vec<String>, can: LevelCan) -> Result<(u8, Outcome)> {
        let state = self.state(community_id).await?;
        let id = match id {
            Some(id) => id,
            None => (0..=MAX_LEVEL_ID).find(|i| !state.levels.contains_key(i)).ok_or_else(|| invalid("level limit"))?,
        };
        if id > MAX_LEVEL_ID {
            return Err(invalid("level limit"));
        }
        let out = self.act_named(keys, community_id, OpBody::LevelSet { id, scopes, can }, name).await?;
        Ok((id, out))
    }

    /// A level goes; the base level never (it is the last one everybody
    /// holds).
    pub async fn level_del(&self, keys: &Keys, community_id: &str, id: u8) -> Result<Outcome> {
        if id == BASE_LEVEL {
            return Err(invalid("level last"));
        }
        self.act(keys, community_id, OpBody::LevelDel { id }).await
    }

    /// The levels of a person: the base level always among them.
    pub async fn set_levels(&self, keys: &Keys, community_id: &str, who: &PubKey, levels: &[u8]) -> Result<Outcome> {
        let state = self.state(community_id).await?;
        let mut bits = 1u64;
        for l in levels {
            if !state.levels.contains_key(l) {
                return Err(invalid("unknown level"));
            }
            bits |= 1u64 << l;
        }
        self.act(keys, community_id, OpBody::SetLevels { who: who.clone(), levels: bits }).await
    }

    // ─── Requests for a level (wire §14.4) ───────────────────────────────────

    /// Asks the moderators for a level: `channel.level_request` to each of
    /// them, through the relays of the channel as `channel.request` goes;
    /// kept here as mine until an answer comes.
    pub async fn level_request(&self, keys: &Keys, community_id: &str, level: u8, note: &str) -> Result<Outcome> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        if !state.levels.contains_key(&level) || level == BASE_LEVEL {
            return Err(invalid("unknown level"));
        }
        if state.levels_of(&me) & (1u64 << level) != 0 {
            return Err(invalid("the level is held already"));
        }
        let now = self.now();
        let body = LevelRequest { community_id: community_id.to_string(), level, note: note.trim().to_string(), created_at: now };
        body.check(now).map_err(invalid)?;
        for m in state.moderators() {
            self.send_note(Some(&state), community_id, &m, wire::T_LEVEL_REQUEST, &body, None).await?;
        }
        let row = LevelRequestRow {
            community_id: community_id.to_string(),
            requester: me.as_hex().to_string(),
            level: i64::from(level),
            direction: "out".into(),
            status: REQ_PENDING.into(),
            note: body.note.clone(),
            reason: None,
            created_at: now,
            updated_at: now,
        };
        db::levels::put_level_request(&self.env.db, &row).await?;
        let mut out = Outcome::default();
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id }));
        Ok(out)
    }

    /// The requests for a level waiting for me as a moderator, oldest
    /// first; one the journal has answered meanwhile (another moderator's
    /// `set_levels`) is closed here and not listed.
    pub async fn level_requests(&self, community_id: &str) -> Result<Vec<LevelRequestRow>> {
        let state = self.state(community_id).await?;
        let mut out = Vec::new();
        for r in db::levels::level_requests(&self.env.db, community_id, "in", REQ_PENDING).await? {
            let level = u8::try_from(r.level).unwrap_or(u8::MAX);
            let held = PubKey::parse(&r.requester).is_some_and(|who| state.levels_of(&who) & (1u64.checked_shl(u32::from(level)).unwrap_or(0)) != 0);
            if held || !state.levels.contains_key(&level) {
                db::levels::set_level_request(&self.env.db, community_id, &r.requester, level, "in", REQ_GRANTED, None, self.now()).await?;
                continue;
            }
            out.push(r);
        }
        Ok(out)
    }

    /// A moderator answers a request: a grant is `set_levels` of the
    /// journal (the levels held and this one) and a note that says so; a
    /// refusal is a note with the reason, the journal untouched.
    pub async fn level_request_act(&self, keys: &Keys, community_id: &str, requester: &PubKey, level: u8, grant: bool, reason: Option<&str>) -> Result<Outcome> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        if !state.role_of(&me).is_some_and(|r| crate::roles::permits(r, crate::roles::Action::SetLevels)) {
            return Err(invalid("not permitted"));
        }
        let mut out = Outcome::default();
        if grant {
            if !state.levels.contains_key(&level) {
                return Err(invalid("unknown level"));
            }
            let bits = state.levels_of(requester) | (1u64 << level);
            out.merge(self.act(keys, community_id, OpBody::SetLevels { who: requester.clone(), levels: bits }).await?);
        }
        let reason = reason.map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
        let body = LevelResolve { community_id: community_id.to_string(), level, granted: grant, reason: if grant { None } else { reason.clone() } };
        body.check().map_err(invalid)?;
        self.send_note(Some(&state), community_id, requester, wire::T_LEVEL_RESOLVE, &body, None).await?;
        let status = if grant { REQ_GRANTED } else { REQ_REFUSED };
        db::levels::set_level_request(&self.env.db, community_id, requester.as_hex(), level, "in", status, body.reason.as_deref(), self.now()).await?;
        out.event(UI_EVENT_CHANNEL_LEVEL_REQUEST, serde_json::json!({ "community_id": community_id, "requester": requester.as_hex() }));
        Ok(out)
    }

    /// `channel.level_request` heard by me (a moderator by the journal):
    /// kept, at most `MAX_REQUESTS` waiting a channel.
    pub(crate) async fn on_level_request(&self, me: &PubKey, state: &crate::state::ChannelState, from: &PubKey, created_at: i64, body: &LevelRequest) -> Result<Outcome> {
        let mut out = Outcome::default();
        let now = self.now();
        if body.check(now).is_err() || !state.role_of(me).is_some_and(crate::roles::Role::is_moderator) || state.is_banned(from) || !state.levels.contains_key(&body.level) {
            return Ok(out);
        }
        let waiting = db::levels::level_requests(&self.env.db, &state.community_id, "in", REQ_PENDING).await?;
        let again = waiting.iter().any(|r| r.requester == from.as_hex() && r.level == i64::from(body.level));
        if waiting.len() >= crate::service::MAX_REQUESTS && !again {
            out.note("too many requests for levels");
            return Ok(out);
        }
        let row = LevelRequestRow {
            community_id: state.community_id.clone(),
            requester: from.as_hex().to_string(),
            level: i64::from(body.level),
            direction: "in".into(),
            status: REQ_PENDING.into(),
            note: body.note.clone(),
            reason: None,
            created_at,
            updated_at: now,
        };
        db::levels::put_level_request(&self.env.db, &row).await?;
        out.event(UI_EVENT_CHANNEL_LEVEL_REQUEST, serde_json::json!({ "community_id": state.community_id, "requester": from.as_hex() }));
        Ok(out)
    }

    /// `channel.level_resolve` to me, from a moderator by the journal: my
    /// request answered.
    pub(crate) async fn on_level_resolve(&self, me: &PubKey, state: &crate::state::ChannelState, from: &PubKey, body: &LevelResolve) -> Result<Outcome> {
        let mut out = Outcome::default();
        let moderator = state.role_of(from).is_some_and(crate::roles::Role::is_moderator) && !state.is_banned(from);
        if body.check().is_err() || !moderator {
            return Ok(out);
        }
        let status = if body.granted { REQ_GRANTED } else { REQ_REFUSED };
        db::levels::set_level_request(&self.env.db, &state.community_id, me.as_hex(), body.level, "out", status, body.reason.as_deref(), self.now()).await?;
        out.event(
            UI_EVENT_CHANNEL_UPDATED,
            serde_json::json!({ "community_id": state.community_id, "level_resolve": { "level": body.level, "granted": body.granted, "reason": body.reason } }),
        );
        Ok(out)
    }

    /// My own request for a level and what became of it.
    pub async fn my_level_requests(&self, community_id: &str) -> Result<Vec<LevelRequestRow>> {
        let mut all = Vec::new();
        for status in [REQ_PENDING, REQ_GRANTED, REQ_REFUSED] {
            all.extend(db::levels::level_requests(&self.env.db, community_id, "out", status).await?);
        }
        Ok(all)
    }

    /// A name as its author sealed it: kept by the operation it names.
    pub(crate) async fn keep_meta(&self, community_id: &str, signer: &PubKey, meta: &Meta, at: i64) -> Result<bool> {
        match (&meta.scope_id, meta.level) {
            (Some(s), None) => {
                let look = serde_json::to_string(&meta.look)?;
                db::levels::put_scope_title(&self.env.db, community_id, s, &meta.op_id, signer.as_hex(), &meta.name, &look, at).await
            }
            (None, Some(l)) => db::levels::put_level_name(&self.env.db, community_id, l, &meta.op_id, signer.as_hex(), &meta.name, at).await,
            _ => Ok(false),
        }
    }

    /// My names that wait for a key of their channel, sealed under the
    /// newest key I hold of the feed each goes into (`everyone`, or the
    /// hidden section it is of); one whose operation the journal refused
    /// (or that is not mine) is dropped.
    pub async fn publish_owed_metas(&self, keys: &Keys) -> Result<()> {
        let me = Self::me(keys)?;
        for row in db::levels::metas_owed(&self.env.db).await? {
            if let Err(e) = self.publish_owed_meta(keys, &me, &row).await {
                tracing::warn!(community = %row.community_id, error = %e, "a name of a level or a scope did not go out: kept for later");
            }
        }
        Ok(())
    }

    async fn publish_owed_meta(&self, keys: &Keys, me: &PubKey, row: &db::levels::OwedMeta) -> Result<()> {
        let (cid, op_id) = (row.community_id.as_str(), row.op_id.as_str());
        let Some(log) = self.log(cid).await? else { return Ok(()) };
        let id = OpId(op_id.to_string());
        let mine = log.get(&id).is_some_and(|op| &op.author == me);
        let Ok(meta) = serde_json::from_str::<Meta>(&row.meta_json) else { return db::levels::meta_paid(&self.env.db, cid, op_id).await };
        if !log.contains(&id) {
            // Not in the journal here yet (it may be on its way): kept.
            return Ok(());
        }
        if !mine || log.rejection(&id).is_some() {
            return db::levels::meta_paid(&self.env.db, cid, op_id).await;
        }
        // A section hidden since the meta was made: it goes into the
        // section's feed after all, never into that of everyone. One gone
        // takes its meta with it.
        let scope = match meta.scope_id.as_deref() {
            Some(s) if s != SCOPE_EVERYONE => match log.state().scopes.get(s) {
                None if row.scope_id != SCOPE_EVERYONE => return db::levels::meta_paid(&self.env.db, cid, op_id).await,
                Some(d) if d.hidden => s.to_string(),
                _ => row.scope_id.clone(),
            },
            _ => SCOPE_EVERYONE.to_string(),
        };
        let Some(key) = self.member(keys).newest_key(cid, &scope).await? else { return Ok(()) };
        let now = self.now();
        let place = Place::feed_in(cid, &scope);
        let inner = wire::sign_meta(keys, cid, &scope, &meta, now)?;
        let event = wire::seal_content(&place, &key, None, &Payload::event(wire::T_META, &inner)?, now)?;
        self.enqueue(cid, db::outbox::LANE_CHANNEL, wire::KIND_CONTENT, &event, Some(&inner.id.to_hex())).await?;
        db::levels::meta_paid(&self.env.db, cid, op_id).await
    }

    /// The names the journal gives now: of each scope and level, the name
    /// of the newest operation of it (in the order of the journal) whose
    /// own author sealed one; a name signed by anybody else names nothing.
    /// The look of a scope comes with its title, from the same meta
    /// (`scope_add` or `scope_set`, 2026-10-23).
    pub async fn names(&self, log: &OpLog) -> Result<Names> {
        let state = log.state();
        let cid = &state.community_id;
        let pos: HashMap<String, (usize, &Op)> =
            log.ordered().filter(|o| log.rejection(&o.id()).is_none()).enumerate().map(|(i, o)| (o.id().0, (i, o))).collect();
        let mut out = Names::default();
        let mut best_scope: BTreeMap<String, usize> = BTreeMap::new();
        for row in db::levels::scope_titles(&self.env.db, cid).await? {
            let Some((p, op)) = pos.get(&row.op_id) else { continue };
            let names_it = matches!(&op.body, OpBody::ScopeAdd { scope_id, .. } | OpBody::ScopeSet { scope_id, .. } if *scope_id == row.id);
            if !names_it || op.author.as_hex() != row.signer || !state.scopes.contains_key(&row.id) {
                continue;
            }
            if best_scope.get(&row.id).is_none_or(|b| p > b) {
                best_scope.insert(row.id.clone(), *p);
                let look: ScopeLook = serde_json::from_str(&row.look).unwrap_or_default();
                if look.is_empty() {
                    out.looks.remove(&row.id);
                } else {
                    out.looks.insert(row.id.clone(), look);
                }
                out.scopes.insert(row.id, row.name);
            }
        }
        let mut best_level: BTreeMap<u8, usize> = BTreeMap::new();
        for row in db::levels::level_names(&self.env.db, cid).await? {
            let Ok(level) = row.id.parse::<u8>() else { continue };
            let Some((p, op)) = pos.get(&row.op_id) else { continue };
            let names_it = matches!(&op.body, OpBody::LevelSet { id, .. } if *id == level);
            if !names_it || op.author.as_hex() != row.signer || !state.levels.contains_key(&level) {
                continue;
            }
            if best_level.get(&level).is_none_or(|b| p > b) {
                best_level.insert(level, *p);
                out.levels.insert(level, row.name);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_of_levels_are_the_same_whatever_the_case() {
        assert!(same_name("Gold", " gold "));
        assert!(same_name("Золото", "ЗОЛОТО"));
        assert!(!same_name("Gold", "Silver"));
        let names = Names::default();
        assert_eq!(names.level(BASE_LEVEL), BASE_LEVEL_NAME);
        assert_eq!(names.level(3), "");
    }
}

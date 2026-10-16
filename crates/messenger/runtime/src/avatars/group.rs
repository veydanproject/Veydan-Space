// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The picture of a group, made the way my avatar is: a picked picture,
//! cropped, encoded again as a 512px JPEG, kept as
//! `avatars/group/<sha256>.jpg`, put open (not encrypted) on up to two of
//! my media servers and named in the log of the group by `edit_settings`
//! as `<public base>/<sha256>` (`set`). Only a manager may, as for every
//! setting; nothing else writes `picture`.
//!
//! The keeper (`keep_groups`, run with the keeper of my avatar) looks after
//! the pictures of the groups I manage: it takes on a picture another
//! manager or another device of mine set and fetches its file when it is
//! on one of my servers, asks that server once a day whether it still has
//! it and puts it back at the same address when it does not, and fetches
//! it every five days so that a server that forgets what nobody reads
//! keeps it; a picture on a server of somebody else is fetched every five
//! days all the same. A picture the log no longer names is taken down
//! from my server, unless another group of mine or my own avatar is the
//! same picture. What it does on a run is decided from the state alone
//! (`group_plan`).
//!
//! Unlike my avatar it never moves a picture and never writes the log: the
//! log of a group is everybody's, and a device that has not caught up with
//! it would bring back a picture another manager took away. A blob put
//! back at its own address names nothing new; when the log moves on, the
//! next run takes it down again.

use super::{copies_of_parts, hash_of, invalid, is_own_encode, joined, server_of, unique, write_whole, Avatars};
use super::{CHECK_EVERY_SECS, JPEG, MAX_FETCH_BYTES, OWN_COPIES, TOUCH_EVERY_SECS, UI_EVENT_AVATAR_READY};
use crate::MessengerRuntime;
use messenger_avatar::{self as avatar, CropRect};
use messenger_core::{MessengerError, Result};
use messenger_groups::{GroupView, OpBody};
use messenger_media::PublicBlob;
use messenger_store::{group_avatar, groups as repo, own_avatar, profiles};
use nostr::key::Keys;
use std::path::PathBuf;

/// What the keeper knows of one group when it starts a run.
#[derive(Clone, Debug, Default)]
pub struct GroupFacts {
    /// The picture of the group as this device looks after it.
    pub row: Option<group_avatar::GroupAvatar>,
    /// `picture` of the log of the group as this device holds it (empty for
    /// none); `None` for a group this device no longer knows.
    pub picture: Option<String>,
    /// I manage the group now: a member, owner or admin.
    pub manager: bool,
    /// `(server id, public base)` of my enabled media servers that can be
    /// written to now.
    pub bases: Vec<(String, String)>,
    /// The same of every enabled one, also those that cannot be written to
    /// now: a server still in use.
    pub known: Vec<(String, String)>,
    /// Hashes of the files in `avatars/group/`.
    pub files: Vec<String>,
    pub now: i64,
}

/// One thing the keeper does for a group, in the order of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupStep {
    /// The log no longer names the picture this device looks after, or I
    /// no longer manage the group: let it go, and its file unless the next
    /// one is the same picture. `take_down`: the log named another picture
    /// (or none) and this one is on a server of mine.
    Forget { sha: String, keep_file: bool, take_down: bool },
    /// The log names a picture this device does not look after yet.
    Adopt { url: String, sha: String, server_id: Option<String> },
    /// Fetch its file, missing on this device: what is put back.
    Download { url: String, sha: String },
    /// Ask its server whether it still has it; put it back if not.
    Check { server_id: String, sha: String },
    /// Fetch it past the caches to keep it alive; put it back if gone.
    Touch { url: String, server_id: String, sha: String },
    /// On a server not mine: fetch it past the caches all the same, so
    /// that the server keeps it.
    KeepAlive { url: String, sha: String },
}

/// What the keeper does for one group with `facts`.
pub fn group_plan(facts: &GroupFacts) -> Vec<GroupStep> {
    let named = facts.picture.as_deref().filter(|p| !p.is_empty());
    let mut steps = Vec::new();
    // (url, sha, checked_at, touched_at) of the picture to look after.
    let mut current = None;
    if let Some(row) = &facts.row {
        if facts.manager && named == Some(row.url.as_str()) {
            current = Some((row.url.clone(), row.sha256.clone(), row.checked_at, row.touched_at));
        } else {
            let next = named.filter(|_| facts.manager).and_then(hash_of);
            // Only when the log said something else: a group I stopped
            // managing, or forgot, still names it.
            let replaced = facts.picture.is_some() && named != Some(row.url.as_str());
            steps.push(GroupStep::Forget {
                sha: row.sha256.clone(),
                keep_file: next.as_deref() == Some(row.sha256.as_str()),
                take_down: replaced && server_of(&row.url, &facts.bases).is_some(),
            });
        }
    }
    if current.is_none() {
        if !facts.manager {
            return steps;
        }
        let Some(url) = named else { return steps };
        // A picture this app made names its hash; any other address is
        // left as it is.
        let Some(sha) = hash_of(url) else { return steps };
        let server_id = server_of(url, &facts.bases).map(str::to_string);
        steps.push(GroupStep::Adopt { url: url.to_string(), sha: sha.clone(), server_id });
        current = Some((url.to_string(), sha, facts.now, facts.now));
    }
    let (url, sha, checked_at, touched_at) = current.expect("set above");
    match server_of(&url, &facts.bases) {
        Some(server_id) => {
            if !facts.files.contains(&sha) {
                steps.push(GroupStep::Download { url: url.clone(), sha: sha.clone() });
            }
            if facts.now - checked_at >= CHECK_EVERY_SECS {
                steps.push(GroupStep::Check { server_id: server_id.into(), sha: sha.clone() });
            }
            if facts.now - touched_at >= TOUCH_EVERY_SECS {
                steps.push(GroupStep::Touch { url, server_id: server_id.into(), sha });
            }
        }
        // Still in use, only not now: the next run sees.
        None if server_of(&url, &facts.known).is_some() => {}
        None => {
            if facts.now - touched_at >= TOUCH_EVERY_SECS {
                steps.push(GroupStep::KeepAlive { url, sha });
            }
        }
    }
    steps
}

/// A manager of the group by its row: a member, owner or admin.
fn manages(row: &repo::GroupRow) -> bool {
    row.membership == "joined" && matches!(row.my_role.as_deref(), Some("owner" | "admin"))
}

impl Avatars {
    pub(super) fn group_path(&self, sha: &str) -> PathBuf {
        self.dir.join("group").join(format!("{sha}.jpg"))
    }

    async fn group_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(mut dir) = tokio::fs::read_dir(self.dir.join("group")).await else { return out };
        while let Ok(Some(entry)) = dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(sha) = name.strip_suffix(".jpg").filter(|s| super::is_sha(s)) {
                out.push(sha.to_string());
            }
        }
        out
    }

    /// Whether some picture of mine is `sha`: of a group this device knows,
    /// or my own avatar. Such a blob is not taken down, nor its file.
    async fn sha_named(&self, keys: &Keys, sha: &str) -> Result<bool> {
        if repo::list(&self.store).await?.iter().any(|g| hash_of(g.picture.trim()).as_deref() == Some(sha)) {
            return Ok(true);
        }
        if own_avatar::get(&self.store).await?.is_some_and(|o| o.sha256 == sha) {
            return Ok(true);
        }
        let me = keys.public_key().to_hex();
        Ok(profiles::get(&self.store, &me).await?.and_then(|p| p.picture).and_then(|p| hash_of(p.trim())).as_deref() == Some(sha))
    }

    /// Let go of the picture `sha` of a group: its file, and its blob on
    /// `copies` — unless something of mine is still that picture.
    async fn let_go(&self, keys: &Keys, sha: &str, copies: Vec<PublicBlob>) -> Result<()> {
        if self.sha_named(keys, sha).await? {
            return Ok(());
        }
        if group_avatar::list(&self.store).await?.iter().all(|r| r.sha256 != sha) {
            let _ = tokio::fs::remove_file(self.group_path(sha)).await;
        }
        if !copies.is_empty() {
            self.take_down(keys, sha, copies);
        }
        Ok(())
    }

    /// Where the picture `url` of a group was put, as far as this device
    /// knows: the list of its row, else the server of mine it is on.
    async fn copies_named(&self, keys: &Keys, url: &str, row: Option<&group_avatar::GroupAvatar>) -> Vec<PublicBlob> {
        if let Some(row) = row.filter(|r| r.url == url) {
            return copies_of_parts(&row.copies_json, row.server_id.as_deref(), &row.url);
        }
        let bases = self.media.enabled_public_bases(keys).await.unwrap_or_default();
        server_of(url, &bases).map(|id| vec![PublicBlob { server_id: id.into(), url: url.into() }]).unwrap_or_default()
    }

    // ─── The keeper ─────────────────────────────────────────────────────────

    /// One run of the keeper over the groups I manage and those this
    /// device looked after; see `group_plan`. A group whose step fails is
    /// tried again on the next run; the others go on.
    pub async fn keep_groups(&self, keys: &Keys) -> Result<Vec<(String, GroupStep)>> {
        let _one = self.group_lock.lock().await;
        let rows = group_avatar::list(&self.store).await?;
        let groups = repo::list(&self.store).await?;
        let mut ids: Vec<String> = groups.iter().filter(|g| manages(g)).map(|g| g.id.clone()).collect();
        for r in &rows {
            if !ids.contains(&r.group_id) {
                ids.push(r.group_id.clone());
            }
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let bases = self.media.enabled_public_bases(keys).await?;
        let known = self.media.enabled_server_bases().await?;
        let mut done = Vec::new();
        for id in ids {
            let group = groups.iter().find(|g| g.id == id);
            let facts = GroupFacts {
                row: rows.iter().find(|r| r.group_id == id).cloned(),
                picture: group.map(|g| g.picture.trim().to_string()),
                manager: group.is_some_and(manages),
                bases: bases.clone(),
                known: known.clone(),
                files: self.group_files().await,
                now: self.now(),
            };
            for step in group_plan(&facts) {
                if let Err(e) = self.run_group_step(keys, &id, &step, &facts).await {
                    eprintln!("messenger avatar: group {id}: {e}");
                    break;
                }
                done.push((id.clone(), step));
            }
        }
        Ok(done)
    }

    async fn run_group_step(&self, keys: &Keys, group_id: &str, step: &GroupStep, facts: &GroupFacts) -> Result<()> {
        let now = facts.now;
        match step {
            GroupStep::Forget { sha, keep_file, take_down } => {
                group_avatar::clear(&self.store, group_id).await?;
                let row = facts.row.as_ref().filter(|r| r.sha256 == *sha);
                let copies = match row {
                    Some(r) if *take_down => copies_of_parts(&r.copies_json, r.server_id.as_deref(), &r.url),
                    _ => Vec::new(),
                };
                if !keep_file || !copies.is_empty() {
                    self.let_go(keys, sha, copies).await?;
                }
            }
            GroupStep::Adopt { url, sha, server_id } => {
                let copies = match server_id {
                    Some(id) => serde_json::to_string(&[PublicBlob { server_id: id.clone(), url: url.clone() }])?,
                    None => "[]".into(),
                };
                group_avatar::set(
                    &self.store,
                    &group_avatar::GroupAvatar {
                        group_id: group_id.into(),
                        sha256: sha.clone(),
                        url: url.clone(),
                        server_id: server_id.clone(),
                        copies_json: copies,
                        set_at: now,
                        checked_at: now,
                        touched_at: now,
                    },
                )
                .await?;
            }
            GroupStep::Download { url, sha } => {
                let bytes = self.fetch_blob(url, sha, &facts.bases).await?;
                if !is_own_encode(&bytes).await {
                    // Not a picture this app made: it is not put back.
                    group_avatar::clear(&self.store, group_id).await?;
                    return Err(MessengerError::Invalid("avatar_not_ours".into()));
                }
                write_whole(&self.group_path(sha), &bytes).await?;
                self.emit(UI_EVENT_AVATAR_READY, serde_json::json!({ "url": url }));
            }
            GroupStep::Check { server_id, sha } => match self.media.public_exists(keys, server_id, sha).await {
                Ok(true) => group_avatar::mark_checked(&self.store, group_id, now).await?,
                Ok(false) => {
                    self.group_put_back(keys, server_id, sha).await?;
                    group_avatar::mark_checked(&self.store, group_id, now).await?;
                }
                Err(e) => eprintln!("messenger avatar: {server_id} not asked: {e}"),
            },
            GroupStep::Touch { url, server_id, sha } => match self.net.own(&unique(url), MAX_FETCH_BYTES, true).await {
                Ok(Some(body)) if avatar::sha256_hex(&body) == *sha => group_avatar::mark_touched(&self.store, group_id, now).await?,
                Ok(_) => {
                    self.group_put_back(keys, server_id, sha).await?;
                    group_avatar::mark_touched(&self.store, group_id, now).await?;
                }
                Err(e) => eprintln!("messenger avatar: {url} not fetched: {e}"),
            },
            GroupStep::KeepAlive { url, sha } => match self.net.foreign(&unique(url), MAX_FETCH_BYTES).await {
                Ok(body) if avatar::sha256_hex(&body) == *sha => group_avatar::mark_touched(&self.store, group_id, now).await?,
                Ok(_) => eprintln!("messenger avatar: {url} is not the picture it names"),
                Err(e) => eprintln!("messenger avatar: {url} not fetched: {e}"),
            },
        }
        Ok(())
    }

    /// The picture back at its own address on a server of mine that lost it.
    async fn group_put_back(&self, keys: &Keys, server_id: &str, sha: &str) -> Result<()> {
        let bytes = tokio::fs::read(self.group_path(sha)).await?;
        self.media.put_public(keys, server_id, bytes, JPEG).await?;
        Ok(())
    }
}

// ─── The runtime ────────────────────────────────────────────────────────────

impl MessengerRuntime {
    /// The group, when I may change its settings (`group_not_permitted`).
    async fn group_i_manage(&self, group_id: &str) -> Result<GroupView> {
        let g = self.group_get(group_id).await?;
        if g.membership != "joined" || !matches!(g.my_role.as_deref(), Some("owner" | "admin")) {
            return Err(MessengerError::Invalid("group_not_permitted".into()));
        }
        Ok(g)
    }

    fn picture_op(url: &str) -> OpBody {
        OpBody::EditSettings {
            name: None,
            about: None,
            picture: Some(url.to_string()),
            history_for_new: None,
            call_node: None,
            call_node_key: None,
            post_mode: None,
            post_schedule: None,
            premod: None,
        }
    }

    /// Make the part `rect` of the prepared picture (`avatar_prepare`) the
    /// picture of a group I manage: on my media servers, named in its log.
    /// The picture it replaces is taken off my servers. Errors:
    /// `group_not_permitted`, `avatar_expired`, `avatar_bad_crop`,
    /// `err.media_no_server` and those of the servers.
    pub async fn group_avatar_set(&self, group_id: &str, token: &str, rect: CropRect) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        let before = self.group_i_manage(group_id).await?;
        let av = &self.avatars;
        let image = av.prepared(token)?;
        let jpeg = tokio::task::spawn_blocking(move || avatar::crop_square(&image, rect, avatar::OWN_SIDE))
            .await
            .map_err(joined)?
            .map_err(invalid)?;
        let sha = avatar::sha256_hex(&jpeg);
        let _one = av.group_lock.lock().await;
        let old = group_avatar::get(&av.store, group_id).await?;
        write_whole(&av.group_path(&sha), &jpeg).await?;
        let blobs = match av.media.upload_public(&keys, jpeg, JPEG, OWN_COPIES).await {
            Ok(b) => b,
            Err(e) => {
                av.let_go(&keys, &sha, Vec::new()).await?;
                return Err(e);
            }
        };
        let first = blobs[0].clone();
        if let Err(e) = self.group_act(group_id, Self::picture_op(&first.url)).await {
            av.let_go(&keys, &sha, blobs).await?;
            return Err(e);
        }
        let now = av.now();
        group_avatar::set(
            &av.store,
            &group_avatar::GroupAvatar {
                group_id: group_id.into(),
                sha256: sha.clone(),
                url: first.url.clone(),
                server_id: Some(first.server_id.clone()),
                copies_json: serde_json::to_string(&blobs)?,
                set_at: now,
                checked_at: now,
                touched_at: now,
            },
        )
        .await?;
        av.release(token);
        let previous = before.picture.trim();
        if !previous.is_empty() && previous != first.url {
            if let Some(old_sha) = hash_of(previous).filter(|s| *s != sha) {
                let copies = av.copies_named(&keys, previous, old.as_ref()).await;
                av.let_go(&keys, &old_sha, copies).await?;
            }
        }
        self.group_get(group_id).await
    }

    /// The group without a picture: the log says so, and the picture goes
    /// off my servers.
    pub async fn group_avatar_remove(&self, group_id: &str) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        let before = self.group_i_manage(group_id).await?;
        let previous = before.picture.trim().to_string();
        if previous.is_empty() {
            return Ok(before);
        }
        let av = &self.avatars;
        let _one = av.group_lock.lock().await;
        let old = group_avatar::get(&av.store, group_id).await?;
        let view = self.group_act(group_id, Self::picture_op("")).await?;
        group_avatar::clear(&av.store, group_id).await?;
        if let Some(sha) = hash_of(&previous) {
            let copies = av.copies_named(&keys, &previous, old.as_ref()).await;
            av.let_go(&keys, &sha, copies).await?;
        }
        Ok(view)
    }

    /// Whether `url` is the picture of a group this device knows, or of one
    /// a manager or a contact of mine invites me to: a face I have a reason
    /// to fetch.
    pub(crate) async fn group_face(&self, url: &str) -> Result<bool> {
        let url = url.trim();
        if repo::list(&self.store).await?.iter().any(|g| g.picture.trim() == url) {
            return Ok(true);
        }
        Ok(self.groups().invites("in").await?.iter().any(|i| i.vouched && i.picture.trim() == url))
    }
}

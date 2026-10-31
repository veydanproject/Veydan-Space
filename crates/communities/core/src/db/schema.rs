// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/store/migrations/009_groups.sql, 029_group_marks.sql
// and 031_group_avatar.sql at 1f2ef31d, then changed for channels.

//! The schema of `communities.db`: `001_channels`, `002_keeper`, and
//! what each sub-stage added after them.
//!
//! The messenger embeds its migrations from SQL files (`sqlx::migrate!`);
//! here the SQL is a literal of the crate instead, run by the same
//! migrator of sqlx and recorded the same way: the check of the
//! boundaries reads the tables a crate creates from its literals
//! (scripts/boundaries.mjs, `tables`), and a crate of the platform names
//! only the tables it creates. A later change of the schema is a new
//! migration appended to `migrations()`, never an edit of one applied.

use sqlx::migrate::{Migration, MigrationType, Migrator};
use std::borrow::Cow;

/// `001_channels`: every table of the module (internal/communities-stage1.md
/// §3.1, §10.9). The truth about a channel is its journal (`cm_ops`);
/// `cm_channels`, `cm_staff` and `cm_keepers` cache what lists need. The
/// content of the feed is here, not in the messenger's chats; what was
/// read stays on the device (`cm_read`). Secrets of the keeper are not
/// here: they live in the secret store.
pub(crate) const MIGRATION_001: &str = r#"
CREATE TABLE IF NOT EXISTS cm_channels (
    id          TEXT PRIMARY KEY NOT NULL,     -- community_id, 64 hex
    name        TEXT NOT NULL,
    about       TEXT NOT NULL DEFAULT '',
    picture     TEXT NOT NULL DEFAULT '',
    relays_json TEXT NOT NULL DEFAULT '[]',
    storage_json TEXT NOT NULL DEFAULT '[]',
    owner       TEXT NOT NULL,
    join_mode   TEXT NOT NULL,                 -- open | request | invite
    -- joined | pending | left | removed | banned | disbanded
    membership  TEXT NOT NULL,
    my_role     TEXT,
    members     INTEGER NOT NULL DEFAULT 0,    -- by the last snapshot
    link_epoch  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    -- Something of the channel is newer than this build reads.
    needs_update INTEGER NOT NULL DEFAULT 0
);

-- What the journal says of people (`ChannelState::staff`), for lists.
CREATE TABLE IF NOT EXISTS cm_staff (
    community_id TEXT NOT NULL,
    pubkey       TEXT NOT NULL,
    role         TEXT NOT NULL,
    muted        INTEGER NOT NULL DEFAULT 0,
    muted_until  INTEGER,
    trusted      INTEGER NOT NULL DEFAULT 0,
    levels       INTEGER NOT NULL DEFAULT 0,
    since        INTEGER NOT NULL,
    added_by     TEXT NOT NULL,
    PRIMARY KEY (community_id, pubkey)
);

CREATE TABLE IF NOT EXISTS cm_ops (
    op_id        TEXT PRIMARY KEY NOT NULL,
    community_id TEXT NOT NULL,
    op_json      TEXT NOT NULL,                -- the signed event
    received_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cm_ops_channel ON cm_ops(community_id);

-- The keeper series by the journal.
CREATE TABLE IF NOT EXISTS cm_keepers (
    community_id TEXT NOT NULL,
    serial       INTEGER NOT NULL,
    pubkey       TEXT NOT NULL,
    from_epoch   INTEGER NOT NULL,
    PRIMARY KEY (community_id, serial)
);

-- Sealed events that came before the key that opens them (§4: kept 7
-- days, the epoch `n` asked for in the meantime).
CREATE TABLE IF NOT EXISTS cm_pending (
    event_id     TEXT PRIMARY KEY NOT NULL,
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    key_id       TEXT NOT NULL,
    event_json   TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    received_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cm_pending_key ON cm_pending(community_id, key_id);

-- The epochs a member took (§5, §6): the digest of the event, the
-- newest generation it carried, and the key once opened (NULL until).
CREATE TABLE IF NOT EXISTS cm_epochs (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    digest       TEXT NOT NULL,
    gen          INTEGER,
    key_hex      TEXT,
    event_json   TEXT NOT NULL,
    at           INTEGER NOT NULL,
    PRIMARY KEY (community_id, scope_id, epoch)
);

-- The bundles of a member (keydist `Bundle::to_bytes`), at most two
-- generations a scope.
CREATE TABLE IF NOT EXISTS cm_bundles (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    gen          INTEGER NOT NULL,
    bundle       BLOB NOT NULL,
    emergency    INTEGER NOT NULL DEFAULT 0,
    received_at  INTEGER NOT NULL,
    PRIMARY KEY (community_id, scope_id, gen)
);

-- Posts of the feed and comments under them: the inner events, as
-- opened. `review` and `mod_state` are what the marks say
-- (`marks::resolve`); `version` is the id of the newest edit, or the id.
CREATE TABLE IF NOT EXISTS cm_posts (
    id            TEXT PRIMARY KEY NOT NULL,
    community_id  TEXT NOT NULL,
    author        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    epoch         INTEGER NOT NULL,
    envelope_json TEXT NOT NULL,
    text          TEXT,
    media_json    TEXT,
    version       TEXT NOT NULL,
    review        TEXT NOT NULL DEFAULT 'ok',
    mod_state     TEXT NOT NULL DEFAULT '',
    deleted       INTEGER NOT NULL DEFAULT 0,
    received_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cm_posts_feed ON cm_posts(community_id, created_at);

CREATE TABLE IF NOT EXISTS cm_comments (
    id            TEXT PRIMARY KEY NOT NULL,
    community_id  TEXT NOT NULL,
    post_id       TEXT NOT NULL,
    parent_id     TEXT,
    author        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    epoch         INTEGER NOT NULL,
    envelope_json TEXT NOT NULL,
    text          TEXT,
    media_json    TEXT,
    version       TEXT NOT NULL,
    review        TEXT NOT NULL DEFAULT 'ok',
    mod_state     TEXT NOT NULL DEFAULT '',
    deleted       INTEGER NOT NULL DEFAULT 0,
    received_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cm_comments_thread ON cm_comments(post_id, created_at);
CREATE INDEX IF NOT EXISTS cm_comments_author ON cm_comments(community_id, author, created_at);

CREATE TABLE IF NOT EXISTS cm_reactions (
    community_id TEXT NOT NULL,
    target_id    TEXT NOT NULL,
    author       TEXT NOT NULL,
    emoji        TEXT NOT NULL,
    at           INTEGER NOT NULL,
    PRIMARY KEY (target_id, author, emoji)
);

-- Marks of moderation (`cmod.*`): a note names one or more messages
-- (`target_id`, one row per target, so that each row carries whether its
-- own mark is in force) or an author and a span of their dates (`author`,
-- `from_at`, `to_at`). Whether a mark is in force (`active`) is a function
-- of the journal as it stands now, recomputed whenever it changes.
CREATE TABLE IF NOT EXISTS cm_marks (
    note_id      TEXT NOT NULL,
    community_id TEXT NOT NULL,
    kind         TEXT NOT NULL,
    target_id    TEXT NOT NULL DEFAULT '',
    author       TEXT,
    from_at      INTEGER,
    to_at        INTEGER,
    version      TEXT,
    reason       TEXT NOT NULL DEFAULT '',
    by           TEXT NOT NULL,
    at           INTEGER NOT NULL,
    active       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (note_id, target_id)
);
CREATE INDEX IF NOT EXISTS cm_marks_target ON cm_marks(community_id, target_id);
CREATE INDEX IF NOT EXISTS cm_marks_author ON cm_marks(community_id, author);

CREATE TABLE IF NOT EXISTS cm_invites (
    invite_id    TEXT PRIMARY KEY NOT NULL,
    community_id TEXT NOT NULL,
    direction    TEXT NOT NULL,                -- in | out
    peer         TEXT NOT NULL,                -- inviter (in) or invitee (out)
    -- sent | received | accepted | declined | done | expired | cancelled
    status       TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cm_invites_channel ON cm_invites(community_id, status);

-- Requests of those who ask a channel by request. Incoming ones sit with the
-- moderators; the outgoing one is the proof that an answer was asked for.
CREATE TABLE IF NOT EXISTS cm_requests (
    community_id TEXT NOT NULL,
    requester    TEXT NOT NULL,
    direction    TEXT NOT NULL,                -- in | out
    status       TEXT NOT NULL,                -- pending | approved | rejected
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, requester, direction)
);

-- Admissions (`channel.admit`, §7.1): those a moderator made, and at the
-- keeper those it took; an `invite_id` is used once.
CREATE TABLE IF NOT EXISTS cm_admits (
    admit_id     TEXT PRIMARY KEY NOT NULL,
    community_id TEXT NOT NULL,
    who          TEXT NOT NULL,
    by           TEXT NOT NULL,
    levels       INTEGER NOT NULL DEFAULT 0,
    invite_id    TEXT,
    expires_at   INTEGER NOT NULL,
    status       TEXT NOT NULL,                -- open | used | expired
    created_at   INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS cm_admits_invite ON cm_admits(community_id, invite_id) WHERE invite_id IS NOT NULL;

-- Snapshots of the readers (§8): the signed events, the last two kept.
CREATE TABLE IF NOT EXISTS cm_snapshots (
    community_id TEXT NOT NULL,
    at           INTEGER NOT NULL,
    event_json   TEXT NOT NULL,
    PRIMARY KEY (community_id, at)
);

-- The cache of opened shards: the readers of one range.
CREATE TABLE IF NOT EXISTS cm_readers (
    community_id TEXT NOT NULL,
    shard_from   TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    entries      BLOB NOT NULL,                -- the shard, opened
    fetched_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, shard_from)
);

-- What I read of a channel, on this device only (spec §10.4).
CREATE TABLE IF NOT EXISTS cm_read (
    community_id TEXT PRIMARY KEY NOT NULL,
    read_at      INTEGER NOT NULL
);

-- The pictures of the channels I manage, as this device looks after them
-- (`avatars/channel/<sha256>.jpg`): the hash, the address the journal
-- names, the server it is on (when it is one of mine), where it was put,
-- when it was set, checked and last fetched to keep it alive.
CREATE TABLE IF NOT EXISTS cm_avatar (
    community_id TEXT PRIMARY KEY NOT NULL,
    sha256       TEXT NOT NULL,
    url          TEXT NOT NULL,
    server_id    TEXT,
    copies_json  TEXT NOT NULL DEFAULT '[]',
    set_at       INTEGER NOT NULL,
    checked_at   INTEGER NOT NULL DEFAULT 0,
    touched_at   INTEGER NOT NULL DEFAULT 0
);
"#;

/// `002_keeper`: the keeper of the channels this device keeps
/// (internal/communities-spec.md §8.2), its state as rows of keydist's
/// change log (`keydist::Change`) so that a join writes one row and not
/// the whole state. The masters of the generations are not here: they
/// are derived from the root secret in the secret store whenever the
/// state is put together (`db::keeper`), and the header keeps zeros in
/// their place. What the cycle of §8.3 needs to survive a crash is here
/// too: which host holds which blob of the pending epoch, the signed
/// event of that epoch (sent again as it is, never signed anew), the
/// blobs waiting to be removed, the invitations already used and the
/// operations of the journal already acted on.
pub(crate) const MIGRATION_002: &str = r#"
-- One row per channel I keep: whose key keeps it, and the secret of the
-- link it last put in the feed.
CREATE TABLE IF NOT EXISTS cm_keeper (
    community_id   TEXT PRIMARY KEY NOT NULL,
    keeper_pubkey  TEXT NOT NULL,
    link_sent      INTEGER,                    -- link_epoch put in the feed
    -- A `rotate_link` whose secret has not come yet: its link epoch and
    -- when I saw it (the previous link is taken for 5 minutes).
    link_wait_epoch INTEGER,
    link_wait_since INTEGER,
    created_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cm_keeper_header (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    header_json  TEXT NOT NULL,                -- keydist KeeperHeader, masters zeroed
    PRIMARY KEY (community_id, scope_id)
);

CREATE TABLE IF NOT EXISTS cm_keeper_members (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    member       TEXT NOT NULL,                -- pubkey, hex
    rec_json     TEXT NOT NULL,                -- keydist MemberRec
    PRIMARY KEY (community_id, scope_id, member)
);

CREATE TABLE IF NOT EXISTS cm_keeper_revoked (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    gen          INTEGER NOT NULL,
    leaf         INTEGER NOT NULL,
    PRIMARY KEY (community_id, scope_id, gen, leaf)
);

-- The recipe of the epoch built and not yet published.
CREATE TABLE IF NOT EXISTS cm_keeper_pending (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    recipe_json  TEXT NOT NULL,
    PRIMARY KEY (community_id, scope_id)
);

CREATE TABLE IF NOT EXISTS cm_keeper_missed (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    key_hex      TEXT NOT NULL,
    PRIMARY KEY (community_id, scope_id, epoch)
);

-- The blobs of the newest published epoch (the next one retires them),
-- in the order keydist lists them.
CREATE TABLE IF NOT EXISTS cm_keeper_blobs (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    pos          INTEGER NOT NULL,
    PRIMARY KEY (community_id, scope_id, sha256)
);

-- Which host holds which blob of an epoch: the upload goes on from here
-- after a crash, and the event names the hosts that hold everything.
CREATE TABLE IF NOT EXISTS cm_keeper_uploads (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    host         TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    PRIMARY KEY (community_id, scope_id, epoch, host, sha256)
);

-- The signed event of the pending epoch, once made: sent again byte for
-- byte, so that a member never sees two versions of one epoch.
CREATE TABLE IF NOT EXISTS cm_keeper_events (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    event_json   TEXT NOT NULL,
    PRIMARY KEY (community_id, scope_id, epoch)
);

-- Blobs to remove from a host from `due_at` (`open`: at once, retried;
-- `from_join`: after the owner's retention).
CREATE TABLE IF NOT EXISTS cm_keeper_retired (
    community_id TEXT NOT NULL,
    host         TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    due_at       INTEGER NOT NULL,
    PRIMARY KEY (community_id, host, sha256)
);
CREATE INDEX IF NOT EXISTS cm_keeper_retired_due ON cm_keeper_retired(due_at);

-- Invitations an admission was taken for (wire §7.1): each once.
CREATE TABLE IF NOT EXISTS cm_keeper_invites (
    community_id TEXT NOT NULL,
    invite_id    TEXT NOT NULL,
    admit_id     TEXT NOT NULL,
    who          TEXT NOT NULL,
    used_at      INTEGER NOT NULL,
    PRIMARY KEY (community_id, invite_id)
);

-- Operations of the journal the keeper acted on (a ban, a removal, an
-- urgent epoch): each once, whatever order and how often they come.
CREATE TABLE IF NOT EXISTS cm_keeper_ops (
    community_id TEXT NOT NULL,
    op_id        TEXT NOT NULL,
    PRIMARY KEY (community_id, op_id)
);
"#;

/// `003_joining`: what the joining side and the author of a link owe the
/// network until it is done (internal/communities-wire.md §7, §10): a
/// `channel.join` sent again while no `channel.issue` came, and the
/// secret of a link epoch its author has not put in the feed yet (no key
/// of the channel here when the operation was made).
pub(crate) const MIGRATION_003: &str = r#"
CREATE TABLE IF NOT EXISTS cm_joining (
    community_id TEXT PRIMARY KEY NOT NULL,
    join_json    TEXT NOT NULL,                -- the note to the keeper, as JSON
    first_at     INTEGER NOT NULL,
    next_at      INTEGER NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS cm_link_owed (
    community_id TEXT NOT NULL,
    link_epoch   INTEGER NOT NULL,
    PRIMARY KEY (community_id, link_epoch)
);
"#;

/// `004_snapshot`: the keeper's side of the snapshot of the readers
/// (internal/communities-wire.md §8): when it let each reader in (the
/// `since` of a shard; keydist does not keep it), and the last snapshot
/// it published — when, of which readers, its shards and the hosts that
/// hold them, and the shards of the one before, which stay until the
/// next one goes out (§9: the last two).
pub(crate) const MIGRATION_004: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_readers (
    community_id TEXT NOT NULL,
    pubkey       TEXT NOT NULL,
    since        INTEGER NOT NULL,
    PRIMARY KEY (community_id, pubkey)
);

CREATE TABLE IF NOT EXISTS cm_keeper_snapshot (
    community_id     TEXT PRIMARY KEY NOT NULL,
    at               INTEGER NOT NULL,
    digest           TEXT NOT NULL,              -- sha256 of the entries
    shards_json      TEXT NOT NULL DEFAULT '[]',
    hosts_json       TEXT NOT NULL DEFAULT '[]', -- hosts holding every shard
    prev_shards_json TEXT NOT NULL DEFAULT '[]'
);
"#;

/// `005_sync`: how far this device has heard each kind of event of a
/// channel (`journal`, `epochs`, `snapshots`, `card`, `feed`), and the
/// notes to each key it reads (`community_id` empty, `part` `notes:<key>`):
/// the catch-up after a start asks from there (`Outbound::Sync`).
pub(crate) const MIGRATION_005: &str = r#"
CREATE TABLE IF NOT EXISTS cm_sync (
    community_id TEXT NOT NULL,
    part         TEXT NOT NULL,
    last_at      INTEGER NOT NULL,
    PRIMARY KEY (community_id, part)
);
"#;

/// `006_uploads`: files of mine on their way to the hosts of a channel
/// (internal/communities-spec.md §10.3): the row a page shows in place of
/// the post or the comment until the blob is stored and the message sent,
/// and why it failed when it did.
pub(crate) const MIGRATION_006: &str = r#"
CREATE TABLE IF NOT EXISTS cm_uploads (
    id           TEXT PRIMARY KEY NOT NULL,    -- local:…
    community_id TEXT NOT NULL,
    post_id      TEXT NOT NULL DEFAULT '',     -- of a comment: its post
    parent_id    TEXT,
    path         TEXT NOT NULL,
    caption      TEXT,
    status       TEXT NOT NULL,                -- uploading | failed
    error        TEXT,
    done         INTEGER NOT NULL DEFAULT 0,
    total        INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cm_uploads_channel ON cm_uploads(community_id, created_at);
"#;

/// `007_genesis`: the owner a link (`o`) or an accepted invitation named
/// for a channel (internal/communities-wire.md §3, 2026-10-22): the only
/// author whose `create` this device takes for that id.
pub(crate) const MIGRATION_007: &str = r#"
CREATE TABLE IF NOT EXISTS cm_genesis (
    community_id TEXT PRIMARY KEY NOT NULL,
    owner        TEXT NOT NULL,
    set_at       INTEGER NOT NULL
);
"#;

/// `008_edits`: the date of the edit a post or a comment shows (0: none),
/// so that an older edit coming later, or replayed, does not win.
pub(crate) const MIGRATION_008: &str = r#"
ALTER TABLE cm_posts ADD COLUMN edited_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cm_comments ADD COLUMN edited_at INTEGER NOT NULL DEFAULT 0;
"#;

/// `009_held_joins`: a `channel.join` whose admission has not reached the
/// keeper yet waits for it (internal/communities-wire.md §7.1,
/// 2026-10-22): one per person, until the admission comes or expires.
pub(crate) const MIGRATION_009: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_held_joins (
    community_id TEXT NOT NULL,
    who          TEXT NOT NULL,
    admit_id     TEXT NOT NULL,
    join_json    TEXT NOT NULL,
    held_at      INTEGER NOT NULL,
    PRIMARY KEY (community_id, who)
);
CREATE INDEX IF NOT EXISTS cm_keeper_held_admit ON cm_keeper_held_joins(community_id, admit_id);
"#;

/// `010_link_after`: the secret of a rotated link goes into the feed under
/// a key of an epoch after the one held when it was rotated, so that a
/// reader put out meanwhile never reads it (wire §10, 2026-10-22).
pub(crate) const MIGRATION_010: &str = r#"
ALTER TABLE cm_link_owed ADD COLUMN after_epoch INTEGER NOT NULL DEFAULT 0;
"#;

/// `011_invite_verified`: an invitation that came before the journal of
/// its channel names a sender nobody checked yet (wire §7, 2026-10-22).
pub(crate) const MIGRATION_011: &str = r#"
ALTER TABLE cm_invites ADD COLUMN verified INTEGER NOT NULL DEFAULT 1;
"#;

/// `012_keeper_wraps`: the gift wraps a keeper kept here has taken, so that
/// a relay replaying one (a leaver's join) within the week of a note is
/// not heard twice.
pub(crate) const MIGRATION_012: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_wraps (
    community_id TEXT NOT NULL,
    wrap_id      TEXT NOT NULL,
    at           INTEGER NOT NULL,
    PRIMARY KEY (community_id, wrap_id)
);
"#;

/// `013_sync_rounds`: a catch-up of a part of a channel as rounds of pages
/// (`round_since`: from where, none while no round is open; `page_count`,
/// `page_min`: the page coming in; `round_max`: the newest event heard
/// since the cursor last moved). The cursor (`last_at`) moves only when a
/// round came back short of its limit: a relay that caps a page must not
/// make the older events of the gap be skipped for good.
pub(crate) const MIGRATION_013: &str = r#"
ALTER TABLE cm_sync ADD COLUMN round_since INTEGER;
ALTER TABLE cm_sync ADD COLUMN round_started INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cm_sync ADD COLUMN page_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cm_sync ADD COLUMN page_min INTEGER;
ALTER TABLE cm_sync ADD COLUMN round_max INTEGER;
"#;

/// `014_out_reason`: why I am out of a channel (`left`, `removed`,
/// `banned`, `disbanded`; none while in), for the row of the list.
pub(crate) const MIGRATION_014: &str = r#"
ALTER TABLE cm_channels ADD COLUMN out_reason TEXT;
"#;

/// `015_keeper_inbox`: the notes to a keeper kept here that wait for its
/// turn (the trial of 1.6, problem 6): a catch-up of the inbox brings them
/// by pages, newest first, and the keeper takes them oldest first; a join
/// the keeper has no room for this minute waits here, not for the
/// joiner's next try.
pub(crate) const MIGRATION_015: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_inbox (
    community_id TEXT NOT NULL,
    note_id      TEXT NOT NULL,
    sender       TEXT NOT NULL,
    content      TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    queued_at    INTEGER NOT NULL,
    PRIMARY KEY (community_id, note_id)
);
CREATE INDEX IF NOT EXISTS cm_keeper_inbox_order ON cm_keeper_inbox(community_id, created_at, note_id);
"#;

/// `016_keeper_outbox`: the notes of a keeper kept here that did not go out
/// (a relay answered `timeout`; the trial of 1.6, problem 9), sent again
/// with a growing wait on its next turns instead of waiting for the
/// reader's next join ten minutes later.
pub(crate) const MIGRATION_016: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_outbox (
    community_id TEXT NOT NULL,
    out_id       TEXT NOT NULL,
    recipient    TEXT NOT NULL,
    content      TEXT NOT NULL,
    hint_json    TEXT NOT NULL,
    attempts     INTEGER NOT NULL,
    next_at      INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, out_id)
);
"#;

/// `017_levels`: the names of the levels and the titles of the scopes of
/// a channel (internal/communities-wire.md §14.1, 2026-10-22), as their
/// authors sealed them into the feed (`t:"meta"`): one row per operation
/// that named one, whoever signed it, so that the newest operation of the
/// journal with a meta of its own author names it whatever order they
/// came in. The ids, the scopes a level gives and its rights are the
/// journal's. `cm_meta_owed`: a meta of mine made with no key of the
/// channel here yet, sealed when one comes.
pub(crate) const MIGRATION_017: &str = r#"
CREATE TABLE IF NOT EXISTS cm_scopes (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    op_id        TEXT NOT NULL,
    signer       TEXT NOT NULL,
    title        TEXT NOT NULL,
    at           INTEGER NOT NULL,
    PRIMARY KEY (community_id, op_id)
);

CREATE TABLE IF NOT EXISTS cm_levels (
    community_id TEXT NOT NULL,
    level_id     INTEGER NOT NULL,
    op_id        TEXT NOT NULL,
    signer       TEXT NOT NULL,
    name         TEXT NOT NULL,
    at           INTEGER NOT NULL,
    PRIMARY KEY (community_id, op_id)
);

CREATE TABLE IF NOT EXISTS cm_meta_owed (
    community_id TEXT NOT NULL,
    op_id        TEXT NOT NULL,
    meta_json    TEXT NOT NULL,
    PRIMARY KEY (community_id, op_id)
);
"#;

/// `018_keeper_flat`: the keeper of a `flat-v1` scope (wire §14.2): its
/// state is small (one record per member, at most 300) and keydist saves
/// it whole, so one row per scope; it holds the newest epoch keys, as the
/// rows of a tree hold its seeds.
pub(crate) const MIGRATION_018: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_flat (
    community_id TEXT NOT NULL,
    scope_id     TEXT NOT NULL,
    state_json   TEXT NOT NULL,
    PRIMARY KEY (community_id, scope_id)
);
"#;

/// `019_content_scope`: the scope (the section) a post or a comment is
/// sealed in (wire §14.3): a page shows those of the scopes the reader
/// belongs to; what came before is of `everyone`. A file on its way goes
/// into the scope it was sent to.
pub(crate) const MIGRATION_019: &str = r#"
ALTER TABLE cm_posts ADD COLUMN scope_id TEXT NOT NULL DEFAULT 'everyone';
ALTER TABLE cm_comments ADD COLUMN scope_id TEXT NOT NULL DEFAULT 'everyone';
ALTER TABLE cm_uploads ADD COLUMN scope_id TEXT NOT NULL DEFAULT 'everyone';
"#;

/// `020_level_requests`: requests for a level (wire §14.4), those that
/// came to me as a moderator (`in`) and mine (`out`), one per person and
/// level: `pending` until a moderator grants (`granted`) or refuses
/// (`refused`, with the reason).
pub(crate) const MIGRATION_020: &str = r#"
CREATE TABLE IF NOT EXISTS cm_level_requests (
    community_id TEXT NOT NULL,
    requester    TEXT NOT NULL,
    level        INTEGER NOT NULL,
    direction    TEXT NOT NULL,                -- in | out
    status       TEXT NOT NULL,                -- pending | granted | refused
    note         TEXT NOT NULL DEFAULT '',
    reason       TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, requester, level, direction)
);
"#;

/// `021_keeper_take`: a keeper this device takes over from another device of
/// the owner (internal/communities-wire.md §15, 2026-10-23): the series its
/// `keeper` operation named, from which epoch, and how (`handover`: the
/// state of the keeper before it comes as a blob; `emergency`: a new
/// generation from the readers of the snapshot). The row goes when the
/// keeper runs here.
pub(crate) const MIGRATION_021: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_take (
    community_id  TEXT PRIMARY KEY NOT NULL,
    keeper_pubkey TEXT NOT NULL,
    serial        INTEGER NOT NULL,
    from_epoch    INTEGER NOT NULL,
    mode          TEXT NOT NULL,               -- handover | emergency
    started_at    INTEGER NOT NULL
);
"#;

/// `022_transfers`: a channel handed to a new owner (internal/communities-wire.md
/// §16, 2026-10-23): the offer that came to me (`in`) or that I made (`out`),
/// one per channel and side, `offered` until accepted or declined.
pub(crate) const MIGRATION_022: &str = r#"
CREATE TABLE IF NOT EXISTS cm_transfers (
    community_id TEXT NOT NULL,
    direction    TEXT NOT NULL,                -- in | out
    peer         TEXT NOT NULL,                -- the old owner (in), the new one (out)
    status       TEXT NOT NULL,                -- offered | accepted | declined
    payload_json TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, direction)
);
"#;

/// `023_tickets`: the tickets of the moderators (internal/communities-wire.md
/// §17, 2026-10-23). `cm_keeper_tickets`: those the keeper kept here minted —
/// each a leaf of `everyone` held for a newcomer not named yet, by the
/// moderator it went to (`minted` until used or revoked). `cm_tickets`: those
/// I hold as a moderator, each an `Issued` to hand a newcomer while the keeper
/// is away (`held` until used).
pub(crate) const MIGRATION_023: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_tickets (
    community_id TEXT NOT NULL,
    ticket_id    TEXT NOT NULL,
    moderator    TEXT NOT NULL,
    member       TEXT NOT NULL,                -- the id of its leaf in keydist, hex
    expires_at   INTEGER NOT NULL,
    status       TEXT NOT NULL,                -- minted | used | revoked
    used_by      TEXT,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, ticket_id)
);
CREATE TABLE IF NOT EXISTS cm_tickets (
    community_id TEXT NOT NULL,
    ticket_id    TEXT NOT NULL,
    issued       TEXT NOT NULL,                -- base64 of Issued::to_bytes
    expires_at   INTEGER NOT NULL,
    status       TEXT NOT NULL,                -- held | used
    used_by      TEXT,
    received_at  INTEGER NOT NULL,
    PRIMARY KEY (community_id, ticket_id)
);
"#;

/// `024_heartbeats`: the newest sign of life of the keeper of a channel
/// heard here (internal/communities-wire.md §18, 2026-10-23): «был в сети»
/// is the newest of it and of the epochs.
pub(crate) const MIGRATION_024: &str = r#"
CREATE TABLE IF NOT EXISTS cm_heartbeats (
    community_id TEXT PRIMARY KEY,
    at           INTEGER NOT NULL,
    serial       INTEGER NOT NULL
);
"#;

/// `025_outbox`: what the commands and the keepers leave to send
/// (`db::outbox`, 2026-10-23): a journal operation, a post, the card, a
/// note by NIP-59, each a row the outbox worker sends off the channel's
/// lock, in order, again after a failure. The notes the keeper owed
/// (`cm_keeper_outbox`, 016) move into its `inbox` lane. `outer_id` of a
/// post or a comment: the id of the sealed event (kind 1390) it came in,
/// named to the relays as held when a page of the past is asked for.
pub(crate) const MIGRATION_025: &str = r#"
CREATE TABLE IF NOT EXISTS cm_outbox (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    community_id TEXT NOT NULL,                -- '' for a publish into my own inbox
    lane         TEXT NOT NULL,                -- channel | replaceable | inbox
    kind         INTEGER,
    event_id     TEXT NOT NULL,                -- the signed event; inbox: sha256(recipient:content)
    ref_id       TEXT,                         -- what the page shows pending: the inner id, the op id
    recipient    TEXT,
    signer       TEXT NOT NULL DEFAULT 'me',   -- me | keeper
    hint_json    TEXT,
    payload      TEXT NOT NULL,                -- the signed event; inbox: the note, wrapped anew each send
    after_seq    INTEGER NOT NULL DEFAULT 0,   -- inbox: goes once the channel rows up to it are gone
    state        TEXT NOT NULL DEFAULT 'queued', -- queued | failed (a label: a row sent is deleted)
    attempts     INTEGER NOT NULL DEFAULT 0,
    next_at      INTEGER NOT NULL,
    error        TEXT,
    created_at   INTEGER NOT NULL,
    UNIQUE (community_id, lane, event_id)
);
CREATE INDEX IF NOT EXISTS cm_outbox_due ON cm_outbox (next_at);
CREATE INDEX IF NOT EXISTS cm_outbox_lane ON cm_outbox (community_id, lane, seq);
INSERT INTO cm_outbox (community_id, lane, kind, event_id, recipient, signer, hint_json, payload, state, attempts, next_at, created_at)
    SELECT community_id, 'inbox', 1059, out_id, recipient, 'keeper', hint_json, content, 'queued', attempts, next_at, created_at FROM cm_keeper_outbox;
DROP TABLE IF EXISTS cm_keeper_outbox;
ALTER TABLE cm_posts ADD COLUMN outer_id TEXT;
ALTER TABLE cm_comments ADD COLUMN outer_id TEXT;
"#;

/// `026_upload_batch`: files of mine sent by one action (an album) carry
/// one `batch` in their descriptors (2026-10-23); kept with the row, so a
/// file sent again after a failure stays in its album.
pub(crate) const MIGRATION_026: &str = r#"
ALTER TABLE cm_uploads ADD COLUMN batch TEXT;
"#;

/// `027_upload_state`: a file goes up through the messenger's engine
/// (2026-10-23): its key and the chunks the hosts hold already, kept
/// after every chunk (messenger-media `UploadState`), so that a send
/// stopped by a pause, a failure or the end of the app goes on from there.
pub(crate) const MIGRATION_027: &str = r#"
ALTER TABLE cm_uploads ADD COLUMN state_json TEXT NOT NULL DEFAULT '{}';
"#;

/// `028_upload_meta`: what a file of mine carries besides its chunks, as a
/// chat's does (2026-10-23): the name it goes under, whether a photo goes
/// as it is, how large a picture or a video is shown, its preview.
pub(crate) const MIGRATION_028: &str = r#"
ALTER TABLE cm_uploads ADD COLUMN meta_json TEXT NOT NULL DEFAULT '{}';
"#;

/// `029_my_files`: the files of the channels this device has (2026-10-23).
/// `cm_local_files`: a file of mine stays where it was sent from, so the
/// post shows it without fetching it back. `cm_known_files`: what a file
/// was stored as on the hosts (its descriptor, under its own key), by its
/// plaintext hash, so the same file sent again goes as a new message of
/// the chunks the hosts hold, not as a second upload.
pub(crate) const MIGRATION_029: &str = r#"
CREATE TABLE IF NOT EXISTS cm_local_files (
    message_id TEXT PRIMARY KEY NOT NULL,
    path       TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS cm_known_files (
    sha256          TEXT NOT NULL,
    size            INTEGER NOT NULL,
    descriptor_json TEXT NOT NULL,
    stored_at       INTEGER NOT NULL,
    PRIMARY KEY (sha256, size)
);
CREATE INDEX IF NOT EXISTS cm_known_files_size ON cm_known_files (size);
"#;

/// `030_scope_look`: the look of a section (internal/communities-wire.md
/// §14.1, 2026-10-23) kept with its title, as its meta carries it (the
/// JSON of `wire::ScopeLook`; `{}` for a meta of before). `cm_meta_owed`:
/// the feed a meta of mine goes into, `everyone` or the hidden section it
/// is of.
pub(crate) const MIGRATION_030: &str = r#"
ALTER TABLE cm_scopes ADD COLUMN look_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE cm_meta_owed ADD COLUMN scope_id TEXT NOT NULL DEFAULT 'everyone';
"#;

/// `031_upload_keys`: the upload keys of a channel's own server
/// (internal/communities-wire.md §9.1, 2026-10-23). `cm_keeper_upload_keys`: those the
/// keeper kept here made, a member and a host each, until revoked (it moves
/// with the keeper). `cm_upload_keys`: those given to me, a host of a channel
/// each.
pub(crate) const MIGRATION_031: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_upload_keys (
    community_id TEXT NOT NULL,
    host         TEXT NOT NULL,
    who          TEXT NOT NULL,                -- the member, hex
    iat          INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    PRIMARY KEY (community_id, host, who)
);
CREATE TABLE IF NOT EXISTS cm_upload_keys (
    community_id TEXT NOT NULL,
    host         TEXT NOT NULL,
    key          TEXT NOT NULL,
    expires_at   INTEGER NOT NULL,
    received_at  INTEGER NOT NULL,
    PRIMARY KEY (community_id, host)
);
"#;

/// `032_cloud_keys`: the keys of the staff in the owner's Veydan Cloud
/// (internal/communities-wire.md §9.2, 2026-10-23). `cm_keeper_cloud_keys`: those
/// the keeper kept here registered with the owner's account, a member each (the
/// key itself is made again from the keeper's, never kept); `sent` = 0 until
/// its secret went to the member. `cm_cloud_keys`: the one given to me, a
/// channel each (its secret is in the secret store, `secrets::cloud_name`).
pub(crate) const MIGRATION_032: &str = r#"
CREATE TABLE IF NOT EXISTS cm_keeper_cloud_keys (
    community_id TEXT NOT NULL,
    who          TEXT NOT NULL,                -- the member, hex
    pubkey       TEXT NOT NULL,                -- the key of the channel for him
    owner        TEXT NOT NULL,                -- the account it is registered with
    expires_at   INTEGER NOT NULL,
    sent         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (community_id, who)
);
CREATE TABLE IF NOT EXISTS cm_cloud_keys (
    community_id TEXT NOT NULL PRIMARY KEY,
    pubkey       TEXT NOT NULL,
    owner        TEXT NOT NULL,
    expires_at   INTEGER NOT NULL,
    received_at  INTEGER NOT NULL
);
"#;

/// `033_reaction_tombstones`: a reaction taken back stays as a row with
/// `kept` 0 and the date of the retraction, so that a copy of the reaction
/// heard after it (the second relay's, a catch-up's) does not put it back:
/// the later word wins, and at the same second the retraction (живой
/// запуск 2026-10-24: the 👀 of a bot stayed on a post after its answer).
pub(crate) const MIGRATION_033: &str = r#"
ALTER TABLE cm_reactions ADD COLUMN kept INTEGER NOT NULL DEFAULT 1;
"#;

/// `034_epochs_heard`: the newest epoch number heard of each channel in an
/// epoch event of a keeper the journal names, taken or not (a new keeper
/// series is numbered past it, wire §15): kept across a restart.
pub(crate) const MIGRATION_034: &str = r#"
CREATE TABLE IF NOT EXISTS cm_epochs_heard (
    community_id TEXT PRIMARY KEY,
    epoch INTEGER NOT NULL
);
"#;

/// `036_keeper_asked`: of a channel I am in, when this device last asked
/// its keeper for a bundle (`channel.rebundle`, asked again after
/// `REBUNDLE_EVERY_SECS`), and when the keeper said I was never let in as
/// a reader (`channel.not_let_in`, none once a bundle came): the page says
/// «an invitation is needed», after a restart too. 035 is skipped: a
/// build of 2026-10-30 had one, reverted.
pub(crate) const MIGRATION_036: &str = r#"
ALTER TABLE cm_channels ADD COLUMN rebundle_at INTEGER;
ALTER TABLE cm_channels ADD COLUMN refused_at INTEGER;
"#;

/// `037_own_wait`: a note of my own keys (`own.channel_keys`) of a
/// channel whose journal is not here yet, kept while its journal is asked
/// of the relays the note names (internal/communities-wire.md §6.4,
/// 2026-10-31: a channel opened on one device shows on the other by
/// itself); and of a channel I am in, when this device last told my other
/// devices its keys (sent again after `OWN_KEYS_EVERY_SECS`, so that a
/// device added later finds the note on the relays).
pub(crate) const MIGRATION_037: &str = r#"
CREATE TABLE IF NOT EXISTS cm_own_wait (
    community_id TEXT PRIMARY KEY NOT NULL,
    owner        TEXT NOT NULL,
    relays_json  TEXT NOT NULL,
    body_json    TEXT NOT NULL,
    received_at  INTEGER NOT NULL
);
ALTER TABLE cm_channels ADD COLUMN own_keys_at INTEGER;
"#;

/// `038_level_asks`: the requests for a level that came to me as a
/// moderator, one row per request (its fingerprint, wire §14.4,
/// 2026-10-31: a request after a refusal is lost nowhere) until it is
/// answered, and the answers apart, so that a request heard after its
/// answer stays closed: by its fingerprint, or, an answer of an older
/// build or of before this migration (`fingerprint` empty), by
/// `asked_at`. `cm_level_requests` keeps mine (`out`) with the
/// fingerprint of the last one. What came to me before is moved over:
/// a request waiting without a fingerprint, an answer by its date.
pub(crate) const MIGRATION_038: &str = r#"
CREATE TABLE IF NOT EXISTS cm_level_asks (
    community_id TEXT NOT NULL,
    requester    TEXT NOT NULL,
    level        INTEGER NOT NULL,
    fingerprint  TEXT NOT NULL,                -- '' heard before this migration
    note         TEXT NOT NULL DEFAULT '',
    created_at   INTEGER NOT NULL,
    heard_at     INTEGER NOT NULL,
    PRIMARY KEY (community_id, requester, level, fingerprint)
);
CREATE TABLE IF NOT EXISTS cm_level_answers (
    community_id TEXT NOT NULL,
    requester    TEXT NOT NULL,
    level        INTEGER NOT NULL,
    fingerprint  TEXT NOT NULL,                -- '' answers every request up to asked_at
    asked_at     INTEGER NOT NULL,
    granted      INTEGER NOT NULL,
    reason       TEXT,
    at           INTEGER NOT NULL,
    PRIMARY KEY (community_id, requester, level, fingerprint, asked_at)
);
INSERT OR IGNORE INTO cm_level_asks (community_id, requester, level, fingerprint, note, created_at, heard_at)
    SELECT community_id, requester, level, '', note, created_at, updated_at FROM cm_level_requests WHERE direction = 'in' AND status = 'pending';
INSERT OR IGNORE INTO cm_level_answers (community_id, requester, level, fingerprint, asked_at, granted, reason, at)
    SELECT community_id, requester, level, '', created_at, status = 'granted', reason, updated_at FROM cm_level_requests WHERE direction = 'in' AND status != 'pending';
DELETE FROM cm_level_requests WHERE direction = 'in';
ALTER TABLE cm_level_requests ADD COLUMN fingerprint TEXT NOT NULL DEFAULT '';
"#;

/// The migrations of the database, oldest first.
pub(crate) fn migrations() -> Migrator {
    Migrator::with_migrations(vec![
        Migration::new(1, Cow::Borrowed("channels"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_001), false),
        Migration::new(2, Cow::Borrowed("keeper"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_002), false),
        Migration::new(3, Cow::Borrowed("joining"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_003), false),
        Migration::new(4, Cow::Borrowed("snapshot"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_004), false),
        Migration::new(5, Cow::Borrowed("sync"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_005), false),
        Migration::new(6, Cow::Borrowed("uploads"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_006), false),
        Migration::new(7, Cow::Borrowed("genesis"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_007), false),
        Migration::new(8, Cow::Borrowed("edits"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_008), false),
        Migration::new(9, Cow::Borrowed("held_joins"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_009), false),
        Migration::new(10, Cow::Borrowed("link_after"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_010), false),
        Migration::new(11, Cow::Borrowed("invite_verified"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_011), false),
        Migration::new(12, Cow::Borrowed("keeper_wraps"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_012), false),
        Migration::new(13, Cow::Borrowed("sync_rounds"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_013), false),
        Migration::new(14, Cow::Borrowed("out_reason"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_014), false),
        Migration::new(15, Cow::Borrowed("keeper_inbox"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_015), false),
        Migration::new(16, Cow::Borrowed("keeper_outbox"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_016), false),
        Migration::new(17, Cow::Borrowed("levels"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_017), false),
        Migration::new(18, Cow::Borrowed("keeper_flat"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_018), false),
        Migration::new(19, Cow::Borrowed("content_scope"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_019), false),
        Migration::new(20, Cow::Borrowed("level_requests"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_020), false),
        Migration::new(21, Cow::Borrowed("keeper_take"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_021), false),
        Migration::new(22, Cow::Borrowed("transfers"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_022), false),
        Migration::new(23, Cow::Borrowed("tickets"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_023), false),
        Migration::new(24, Cow::Borrowed("heartbeats"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_024), false),
        Migration::new(25, Cow::Borrowed("outbox"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_025), false),
        Migration::new(26, Cow::Borrowed("upload_batch"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_026), false),
        Migration::new(27, Cow::Borrowed("upload_state"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_027), false),
        Migration::new(28, Cow::Borrowed("upload_meta"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_028), false),
        Migration::new(29, Cow::Borrowed("my_files"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_029), false),
        Migration::new(30, Cow::Borrowed("scope_look"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_030), false),
        Migration::new(31, Cow::Borrowed("upload_keys"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_031), false),
        Migration::new(32, Cow::Borrowed("cloud_keys"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_032), false),
        Migration::new(33, Cow::Borrowed("reaction_tombstones"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_033), false),
        Migration::new(34, Cow::Borrowed("epochs_heard"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_034), false),
        Migration::new(36, Cow::Borrowed("keeper_asked"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_036), false),
        Migration::new(37, Cow::Borrowed("own_wait"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_037), false),
        Migration::new(38, Cow::Borrowed("level_asks"), MigrationType::Simple, sqlx::SqlSafeStr::into_sql_str(MIGRATION_038), false),
    ])
}

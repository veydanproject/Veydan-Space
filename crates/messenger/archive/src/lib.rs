// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The archive of the conversation in the user's sync storage.
//!
//! Relays keep a message for a season and a fresh device asks them for the
//! newest two thousand wraps; the archive keeps every event the messenger
//! received, by scope and month, as large files of the sync storage, and a
//! fresh device replays them through the same ingress the relays feed
//! (`internal/messenger-archive-plan.md` §4, `internal/messenger-wire.md`
//! §8, `internal/platform-spec.md` §9.6).
//!
//! This crate is the contract between the three parts and holds the pure
//! parts of the work:
//!
//! - **types** (here): what an archived event is ([`ArchivedEvent`]), how
//!   it is scoped ([`ArchiveScope`]) and dated ([`Month`]), the id of a
//!   segment ([`SegmentId`]), the summary of one ([`SegmentMeta`]), the
//!   names of the sync entities and of the UI events;
//! - **seams**: the runtime hands every event it received to an
//!   [`ArchiveSink`] and asks an [`ArchiveMarks`] how far the archive
//!   reaches (both together: [`ArchiveHooks`]); the archive hands every
//!   event it restores to a [`Replayer`], which the runtime implements
//!   over its ingress;
//! - **codec, writer, queue** (stage Э1): [`segment`] — the file of a
//!   segment, append-only; [`collect`] — the [`Archive`] of this device:
//!   which events it holds by segment, what must be written, the merge with
//!   the file of another device, compaction; [`queue`] — the order of the
//!   restore and the replay of a downloaded segment.
//!
//! What it never knows: the sync storage, the engine of large files and
//! Tauri. The handler of the sync entities lives in `messenger-app`
//! (`internal/messenger-spec.md` §4.2).

pub mod collect;
pub mod queue;
pub mod segment;

#[cfg(test)]
mod archive_tests;

pub use collect::{Archive, Built, Merged, ReplayReport};
pub use queue::{Enqueued, QueueItem, QueueProgress};

use async_trait::async_trait;
use messenger_core::{DmInbound, EventId, EventSource, MessengerError, PubKey, RawEvent, Timestamp};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The wire kind of a personal message: a gift wrap (NIP-59), the `kind`
/// of a `Dm` body (`messenger_ingress::classify::KIND_GIFT_WRAP`).
pub const KIND_GIFT_WRAP: u16 = 1059;

/// The first bytes of a segment file: Veydan Messenger Archive, version 1.
pub const SEGMENT_MAGIC: &[u8; 4] = b"VMA1";
/// The version of the segment format. A reader refuses any other.
pub const FORMAT_VERSION: u16 = 1;
/// The chunk of the large file a segment is stored as: appending a month
/// changes the last chunk alone, the rest are deduplicated by the engine
/// (the smallest the engine allows).
pub const CHUNK_SIZE: u64 = 1024 * 1024;
/// What a segment file holds at most: its bytes, the bytes of one record,
/// its records. A device reads a file whole into memory and keeps its
/// records, so a bigger one is not written (the month stays out of the
/// archive) nor downloaded (it waits, refused, for a build that reads it):
/// a phone restoring a month never runs out of memory for it, and is told
/// which months it could not take ([`is_too_large`]).
pub const MAX_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_RECORD_BYTES: usize = 1024 * 1024;
pub const MAX_RECORDS: usize = 100_000;

/// An open month is written no more often than this, when it has new
/// events; also on exit and when the app goes to the background.
pub const FLUSH_INTERVAL_SECS: i64 = 600;
/// A month is closed this long after it ended; a late event of a closed
/// month rewrites it once more.
pub const CLOSE_GRACE_SECS: i64 = 7 * 86400;
/// How far ahead of the time an event was received its `at` may count in
/// the high-water mark of a scope. `at` is the sender's clock; one event
/// dated far ahead would otherwise move the relay catch-up of every device
/// past the present for good. The same order as the skew the request
/// floor of personal messages allows (`messenger_dm::relationship::FLOOR_SKEW_SECS`).
pub const AT_SKEW_SECS: i64 = 300;

/// The sync entity of a segment: id `<owner>/<scope>/<yyyy-mm>` (the
/// owner's public key in hex: a storage two identities share holds the
/// archive of each beside the other's, and a device restores its own),
/// payload [`SegmentMeta`] plus the reference of the large file (`lf`,
/// `lf_refs`), added by the handler.
pub const ENTITY_SEGMENT: &str = "messenger_segment";
/// The sync entity of a secret of the lock box: id `<owner>/<name of the
/// secret>`, payload its bytes and the owner; deletion = an op with an
/// empty payload.
pub const ENTITY_SECRET: &str = "messenger_secret";

/// Why the runtime refuses to replay an event for now: a group this device
/// has no row for (its welcome has not come), a private server it does not
/// know. The segment is parked until such a row comes.
pub const REFUSED_GROUP_UNKNOWN: &str = "group_unknown";
pub const REFUSED_SERVER_UNKNOWN: &str = "server_unknown";

/// Whether a replay failed because the device lacks the group or the
/// server of the event ([`REFUSED_GROUP_UNKNOWN`], [`REFUSED_SERVER_UNKNOWN`]).
pub fn is_parked_reason(e: &MessengerError) -> bool {
    matches!(e, MessengerError::Invalid(m) if m == REFUSED_GROUP_UNKNOWN || m == REFUSED_SERVER_UNKNOWN)
}

/// Whether a failure of the handler stops a replay where it is, the rest
/// of the segment left unreplayed to be tried again: the handler could not
/// run (between sessions, the store busy or broken, the box locked) or
/// refuses the whole scope for now ([`is_parked_reason`]). Any other error
/// is the handler refusing that one event, which is left behind.
pub fn stops_replay(e: &MessengerError) -> bool {
    is_parked_reason(e)
        || matches!(e, MessengerError::NotLoggedIn | MessengerError::Storage(_) | MessengerError::Io(_) | MessengerError::SecretsLocked)
}

const FORMAT_PREFIX: &str = "archive format: ";

/// A file or a record this build does not understand: another version of
/// the format, a body of a later build.
pub fn format_error(what: impl std::fmt::Display) -> MessengerError {
    MessengerError::Invalid(format!("{FORMAT_PREFIX}{what}"))
}

pub fn is_format_error(e: &MessengerError) -> bool {
    matches!(e, MessengerError::Invalid(m) if m.starts_with(FORMAT_PREFIX))
}

const TOO_LARGE: &str = "too large: ";

/// A segment over [`MAX_SEGMENT_BYTES`], [`MAX_RECORD_BYTES`] or
/// [`MAX_RECORDS`]: a format error, so that it is refused for this build
/// and the rest of the archive goes on.
pub fn too_large(what: impl std::fmt::Display) -> MessengerError {
    format_error(format!("{TOO_LARGE}{what}"))
}

pub fn is_too_large(e: &MessengerError) -> bool {
    is_too_large_reason(&e.to_string())
}

/// Whether the reason a queue item was refused with ([`too_large`]) says
/// the segment was over the bounds.
pub fn is_too_large_reason(reason: &str) -> bool {
    reason.contains(&format!("{FORMAT_PREFIX}{TOO_LARGE}"))
}

/// The entity id of a secret of `owner`'s archive.
pub fn secret_entity_id(owner: &PubKey, name: &str) -> String {
    format!("{}/{name}", owner.as_hex())
}

/// The owner and the name of a secret from its entity id; `None` for an id
/// of another shape.
pub fn parse_secret_id(id: &str) -> Option<(PubKey, &str)> {
    let (owner, name) = id.split_once('/')?;
    let owner = PubKey::parse(owner)?;
    (!name.is_empty() && !name.contains('/')).then_some((owner, name))
}

/// UI event: the archive moves (`ArchiveProgress`).
pub const EVENT_PROGRESS: &str = "archive.progress";
/// UI event: every segment the storage holds has been replayed; the relay
/// catch-up follows (`history.synced`).
pub const EVENT_SYNCED: &str = "archive.synced";

/// Whether a secret of the lock box travels in the archive: the keys and
/// the link secrets of groups, nothing else (the nsec goes by escrow; the
/// secrets of media hosts, call nodes and push are made anew).
pub fn secret_is_archived(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("group.") else { return false };
    let Some((_, kind)) = rest.rsplit_once('.').and_then(|(head, _)| head.rsplit_once('.')) else { return false };
    matches!(kind, "key" | "link")
}

/// Where an event belongs: one segment stream per scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ArchiveScope {
    /// Personal messages of the home chat and the notes of my own devices.
    Dm,
    /// Personal messages of the private chat on a server (`msg_servers.id`).
    DmServer { id: String },
    /// The events of one group.
    Group { id: String },
    /// Replaceable events about me: the snapshot of my relations.
    Meta,
}

impl ArchiveScope {
    /// The scope as the segment id spells it: `dm`, `dm@<id>`,
    /// `group:<id>`, `meta`. Ids hold no `/`.
    pub fn as_key(&self) -> String {
        match self {
            Self::Dm => "dm".into(),
            Self::DmServer { id } => format!("dm@{id}"),
            Self::Group { id } => format!("group:{id}"),
            Self::Meta => "meta".into(),
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        if key.contains('/') {
            return None;
        }
        Some(match key {
            "dm" => Self::Dm,
            "meta" => Self::Meta,
            _ => {
                if let Some(id) = key.strip_prefix("dm@") {
                    Self::DmServer { id: id.into() }
                } else {
                    Self::Group { id: key.strip_prefix("group:")?.into() }
                }
            }
        })
        .filter(|s| !matches!(s, Self::DmServer { id } | Self::Group { id } if id.is_empty()))
    }
}

impl std::fmt::Display for ArchiveScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_key())
    }
}

/// A calendar month in UTC, `yyyy-mm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Month {
    pub year: i32,
    pub month: u8,
}

impl Month {
    /// The month of a unix time, in UTC.
    pub fn of(at: Timestamp) -> Self {
        let days = at.0.div_euclid(86400);
        let (year, month, _) = civil_from_days(days);
        Self { year, month }
    }

    /// The first second of the month.
    pub fn start(&self) -> Timestamp {
        Timestamp(days_from_civil(self.year, self.month, 1) * 86400)
    }

    /// The month after this one.
    pub fn next(&self) -> Self {
        if self.month == 12 {
            Self { year: self.year + 1, month: 1 }
        } else {
            Self { year: self.year, month: self.month + 1 }
        }
    }

    /// Whether the month is closed at `now`: ended more than
    /// [`CLOSE_GRACE_SECS`] ago.
    pub fn is_closed_at(&self, now: Timestamp) -> bool {
        self.next().start().0 + CLOSE_GRACE_SECS <= now.0
    }

    pub fn parse(s: &str) -> Option<Self> {
        let (y, m) = s.split_once('-')?;
        if y.len() != 4 || m.len() != 2 {
            return None;
        }
        let year: i32 = y.parse().ok()?;
        let month: u8 = m.parse().ok()?;
        (1..=12).contains(&month).then_some(Self { year, month })
    }
}

impl std::fmt::Display for Month {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

// Howard Hinnant's civil-from-days and days-from-civil, proleptic Gregorian.
fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One segment: whose archive, a scope and a month. Its sync entity id is
/// `<owner>/<scope>/<yyyy-mm>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SegmentId {
    pub owner: PubKey,
    pub scope: ArchiveScope,
    pub month: Month,
}

impl SegmentId {
    pub fn entity_id(&self) -> String {
        format!("{}/{}/{}", self.owner.as_hex(), self.scope.as_key(), self.month)
    }

    pub fn parse(id: &str) -> Option<Self> {
        let (owner, rest) = id.split_once('/')?;
        let (scope, month) = rest.rsplit_once('/')?;
        Some(Self { owner: PubKey::parse(owner)?, scope: ArchiveScope::parse(scope)?, month: Month::parse(month)? })
    }
}

/// The summary of a segment in the payload of its op. The handler adds the
/// reference of the large file (`lf`, `lf_refs`) beside these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentMeta {
    /// The owner's public key, hex: the first part of the entity id.
    pub owner: String,
    pub scope: String,
    pub month: String,
    /// Events in the file.
    pub count: u64,
    /// `at` of the earliest and the latest event.
    pub first_at: i64,
    pub last_at: i64,
    /// Bytes of the plaintext file.
    pub size: u64,
    /// sha256 of the plaintext file, hex.
    pub hash: String,
}

/// How an event is kept.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum Body {
    /// As the relay has it: the events of groups (their content sealed
    /// with the group key, which travels as `messenger_secret`), the
    /// snapshot of my relations (NIP-44 to myself). Nothing a wrap of two
    /// NIP-44 layers would make five times bigger.
    Wire { raw: RawEvent },
    /// A personal message or a note of my own devices after the wrap is
    /// opened: the rumor, as the ingress hands it to the DM handler.
    Dm { inbound: DmInbound },
}

/// One event of the archive.
///
/// `at` is the application time the month is taken from: the rumor's
/// `created_at` of a personal message (the wrap's is jittered by two days),
/// the wire `created_at` of anything else. `id` is the id of the wire
/// event, by which segments are merged and replays deduplicated.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivedEvent {
    pub id: EventId,
    pub kind: u16,
    pub at: Timestamp,
    /// When this device received it.
    pub received_at: Timestamp,
    pub scope: ArchiveScope,
    pub body: Body,
}

impl ArchivedEvent {
    /// Of a wire event (a group event, a snapshot); `scope` names the
    /// stream. The source is set to [`EventSource::Archive`]: a segment
    /// does not remember which relay one device heard an event by.
    pub fn wire(mut raw: RawEvent, scope: ArchiveScope, received_at: Timestamp) -> Self {
        raw.source = EventSource::Archive;
        Self { id: raw.id.clone(), kind: raw.kind, at: raw.created_at, received_at, scope, body: Body::Wire { raw } }
    }

    /// Of an opened personal message or own note.
    pub fn dm(mut inbound: DmInbound, received_at: Timestamp) -> Self {
        inbound.envelope.source = EventSource::Archive;
        let scope = match &inbound.srv {
            Some(id) => ArchiveScope::DmServer { id: id.clone() },
            None => ArchiveScope::Dm,
        };
        Self {
            id: inbound.envelope.wire_id.clone(),
            kind: KIND_GIFT_WRAP,
            at: inbound.created_at,
            received_at,
            scope,
            body: Body::Dm { inbound },
        }
    }

    /// The segment of `owner`'s archive the event belongs to.
    pub fn segment(&self, owner: &PubKey) -> SegmentId {
        SegmentId { owner: owner.clone(), scope: self.scope.clone(), month: Month::of(self.at) }
    }
}

/// What the runtime calls with every event it received and kept
/// (`msg_events_raw`): the archive remembers which segment it changes.
/// Implemented by the archive (stage Э1), called by the ingress (stage Э3).
#[async_trait]
pub trait ArchiveSink: Send + Sync {
    async fn record(&self, event: ArchivedEvent) -> messenger_core::Result<()>;
}

/// What the archive knows of its own extent: the time of the latest event
/// it holds of a scope, by which the relay catch-up of a fresh device
/// starts (`since` = mark − margin) instead of from the beginning. `None`:
/// the archive holds nothing of the scope, or could not say. Implemented
/// by the archive, read by the runtime (stage Э3).
#[async_trait]
pub trait ArchiveMarks: Send + Sync {
    async fn high_water(&self, scope: &ArchiveScope) -> Option<Timestamp>;
}

/// What the handler of the archive gives the runtime: where to record,
/// and how far the archive reaches. Every sink with marks is one.
pub trait ArchiveHooks: ArchiveSink + ArchiveMarks {}

impl<T: ArchiveSink + ArchiveMarks + ?Sized> ArchiveHooks for T {}

/// What the archive calls with every event it restores. Implemented by the
/// runtime over its ingress (stage Э3): a `Wire` body goes in as a raw
/// event with [`EventSource::Archive`], a `Dm` body straight to the DM
/// handler; both are deduplicated by `id` as events of relays are.
#[async_trait]
pub trait Replayer: Send + Sync {
    async fn replay(&self, event: ArchivedEvent) -> messenger_core::Result<Replayed>;
}

/// What a replay did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replayed {
    Applied,
    /// The device had it already.
    Duplicate,
    /// Nothing takes events of its kind (an old archive, a type dropped).
    Ignored,
}

/// The payload of [`EVENT_PROGRESS`]: one event per change of phase and at
/// most one a second while bytes move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ArchiveProgress {
    pub phase: ArchivePhase,
    /// Segments done and in all, in the current phase.
    pub done: u32,
    pub total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The month under way, `yyyy-mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month: Option<String>,
    /// Why the archive stands, in `Paused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PauseReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ArchivePhase {
    /// Nothing to do: every segment the storage holds is replayed, every
    /// change of this device is written.
    Idle,
    /// Segments are downloaded and replayed (step B of the restore before
    /// the first screen, step C in the background).
    Restoring,
    /// Segments of this device are written.
    Writing,
    /// The storage refuses writes; reading goes on.
    Paused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum PauseReason {
    /// The Cloud account is in grace or suspended: 403 `read_only`,
    /// until `until` when the server says.
    ReadOnly {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<i64>,
    },
    /// The quota of the storage is full.
    Quota,
    /// The storage does not answer.
    Offline,
    /// This device has no sync storage, or one the archive cannot use
    /// (a folder on a phone).
    NoStorage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_are_utc_and_round_trip() {
        // 2026-11-04 10:00 UTC
        let m = Month::of(Timestamp(1_793_793_600));
        assert_eq!(m, Month { year: 2026, month: 11 });
        assert_eq!(m.to_string(), "2026-11");
        assert_eq!(Month::parse("2026-11"), Some(m));
        assert_eq!(m.start(), Timestamp(1_793_491_200)); // 2026-11-01 00:00 UTC
        assert_eq!(m.next(), Month { year: 2026, month: 12 });
        assert_eq!(Month { year: 2026, month: 12 }.next(), Month { year: 2027, month: 1 });
        // The last second of October is October; the first of November is November.
        assert_eq!(Month::of(Timestamp(1_793_491_199)).month, 10);
        assert_eq!(Month::of(Timestamp(0)), Month { year: 1970, month: 1 });
        assert_eq!(Month::parse("2026-13"), None);
        assert_eq!(Month::parse("26-11"), None);
    }

    #[test]
    fn a_month_closes_a_week_after_it_ends() {
        let oct = Month { year: 2026, month: 10 };
        let nov_1 = oct.next().start();
        assert!(!oct.is_closed_at(nov_1));
        assert!(!oct.is_closed_at(Timestamp(nov_1.0 + CLOSE_GRACE_SECS - 1)));
        assert!(oct.is_closed_at(Timestamp(nov_1.0 + CLOSE_GRACE_SECS)));
    }

    #[test]
    fn scopes_and_segment_ids_round_trip() {
        let owner = PubKey::parse(&"ab".repeat(32)).unwrap();
        for scope in [
            ArchiveScope::Dm,
            ArchiveScope::Meta,
            ArchiveScope::DmServer { id: "srv-1".into() },
            ArchiveScope::Group { id: "ab12".into() },
        ] {
            assert_eq!(ArchiveScope::parse(&scope.as_key()), Some(scope.clone()), "{scope}");
            let seg = SegmentId { owner: owner.clone(), scope: scope.clone(), month: Month { year: 2026, month: 3 } };
            assert!(seg.entity_id().starts_with(&format!("{}/", owner.as_hex())));
            assert_eq!(SegmentId::parse(&seg.entity_id()), Some(seg));
        }
        assert_eq!(ArchiveScope::parse("dm@"), None);
        assert_eq!(ArchiveScope::parse("group:"), None);
        assert_eq!(ArchiveScope::parse("chat"), None);
        assert_eq!(ArchiveScope::parse("dm/x"), None);
        assert_eq!(SegmentId::parse("dm"), None);
        assert_eq!(SegmentId::parse("dm/2026-03"), None, "no owner");
        assert_eq!(SegmentId::parse(&format!("{}/dm/2026-3", owner.as_hex())), None);
        assert_eq!(SegmentId::parse("nobody/dm/2026-03"), None);

        let id = secret_entity_id(&owner, "group.ab12.key.0f3a");
        assert_eq!(parse_secret_id(&id), Some((owner.clone(), "group.ab12.key.0f3a")));
        assert_eq!(parse_secret_id("group.ab12.key.0f3a"), None, "no owner");
        assert_eq!(parse_secret_id(&format!("{}/", owner.as_hex())), None);
    }

    #[test]
    fn the_errors_of_a_replay_are_told_apart() {
        assert!(is_parked_reason(&MessengerError::Invalid(REFUSED_GROUP_UNKNOWN.into())));
        assert!(is_parked_reason(&MessengerError::Invalid(REFUSED_SERVER_UNKNOWN.into())));
        assert!(!is_parked_reason(&MessengerError::Invalid("other".into())));
        assert!(is_format_error(&format_error("version 2")));
        assert!(!is_format_error(&MessengerError::Invalid("archive: x".into())));
        assert!(!is_format_error(&MessengerError::NotLoggedIn));
        let big = too_large("18 MiB");
        assert!(is_format_error(&big) && is_too_large(&big));
        assert!(is_too_large_reason(&big.to_string()));
        assert!(!is_too_large(&format_error("version 2")));
    }

    #[test]
    fn only_the_keys_and_links_of_groups_are_archived() {
        assert!(secret_is_archived("group.ab12.key.0f3a"));
        assert!(secret_is_archived("group.ab12.link.3"));
        assert!(!secret_is_archived("identity.nsec"));
        assert!(!secret_is_archived("media.x.secret"));
        assert!(!secret_is_archived("call.device.node"));
        assert!(!secret_is_archived("push.key.abcd"));
        assert!(!secret_is_archived("group.ab12.other.1"));
        assert!(!secret_is_archived("group.ab12.key"));
    }

    #[test]
    fn progress_serializes_for_the_ui() {
        let p = ArchiveProgress {
            phase: ArchivePhase::Paused,
            done: 3,
            total: 10,
            bytes_done: 1,
            bytes_total: 2,
            month: Some("2026-10".into()),
            reason: Some(PauseReason::ReadOnly { until: Some(5) }),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["phase"], "paused");
        assert_eq!(json["reason"]["why"], "read_only");
        assert_eq!(json["reason"]["until"], 5);
        let back: ArchiveProgress = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);
        let idle = ArchiveProgress { phase: ArchivePhase::Idle, done: 0, total: 0, bytes_done: 0, bytes_total: 0, month: None, reason: None };
        assert!(serde_json::to_value(&idle).unwrap().get("reason").is_none());
    }
}

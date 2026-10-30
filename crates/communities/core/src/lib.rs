// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/lib.rs at 1f2ef31d, then changed for channels.

//! Channels: the engine of the communities of Veydan, stage 1
//! (internal/communities-wire.md, internal/communities-stage1.md).
//!
//! There is no server that owns a channel. Its journal is a log of
//! operations of the staff, each made by someone who was allowed to make
//! it, and every device computes the same state from the same set of
//! operations, whatever order they arrived in. The readers are not in it:
//! a keeper (keydist) lets them in and out and hands them the keys of the
//! epochs, and its snapshot says who they are.
//!
//! - `roles`: owner, admin, moderator, author, member, and what each may do.
//! - `op`: operations and their identity.
//! - `state`: the state of a channel and how one operation changes it;
//!   the rules of writing at a moment (`rules_at`).
//! - `log`: the journal, its linear order and the resulting state.
//! - `schedule`: the hours a channel is open for posting.
//! - `marks`, `moderation`: the marks of moderation and their force.
//! - `wire`: sealed content, the journal, the card, the notes.
//! - `link`: the link of a channel and its secret.
//! - `snapshot`: the keeper's snapshot of the readers.
//! - `epoch`: epochs and keys on the member's side.
//! - `db`: the module's own database.
//! - `qr`: a link as a QR code.
//! - `keeper`: the keeper of a channel kept on this device: keydist's
//!   state as rows, readers in and out, the cycle of epochs.
//! - `member`: the keys on the member's side, by the database: issue,
//!   epochs, shards, the chain, notes to my other devices.
//! - `notes`, `blobs`, `secrets`: NIP-59 notes, the blob hosts, what
//!   lives in the secret store.
//! - `transport`: the subscriptions and the catch-up of the channels, the
//!   pages of the feed and of a thread.
//! - `media`: files of posts and comments on the hosts of a channel, and
//!   its cover.
//! - `levels`: the scopes and the levels with their sealed names.
//! - `moderate`: the acts of moderation, the queue, the log, requests and
//!   invitations, as the page asks for them.
//! - `reader_list`: the readers of a channel, for its team only.
//!
//! Everything but `db`, `keeper`, `blobs` and `secrets` is pure: no
//! network, no clock. Signatures are checked where events enter (`wire`,
//! `snapshot`, `epoch`, `notes`).

pub mod blobs;
pub mod cloud_keys;
pub mod db;
pub mod epoch;
pub mod handler;
pub mod handover;
pub mod heartbeat;
pub mod inbound;
pub mod keeper;
pub mod levels;
pub mod link;
pub mod log;
pub mod marks;
pub mod media;
pub mod media_hosts;
pub mod member;
pub mod moderate;
pub mod moderation;
pub mod notes;
pub mod op;
pub mod outbox;
pub mod qr;
pub mod reader_list;
pub mod roles;
pub mod schedule;
pub mod secrets;
pub mod service;
pub mod settle;
pub mod snapshot;
pub mod staff_keys;
pub mod state;
pub mod tickets;
pub mod transfer;
pub mod transport;
pub mod upload_keys;
pub mod wire;

#[cfg(test)]
mod scenarios;
#[cfg(test)]
mod scenarios_fetch;
#[cfg(test)]
mod scenarios3;
#[cfg(test)]
mod scenarios_own;
#[cfg(test)]
mod scenarios_readers;
#[cfg(test)]
mod scenarios_sections;
#[cfg(test)]
mod scenarios_upload_keys;
#[cfg(test)]
mod scenarios_cloud_keys;
/// The fakes of the tests, also the world of `communities-bench`.
#[cfg(any(test, feature = "tools"))]
#[doc(hidden)]
pub mod testkit;
/// `communities-bench`: the keeper and a member at a million readers
/// over the fakes of `testkit` (stage 4, the offline half).
#[cfg(feature = "tools")]
#[doc(hidden)]
pub mod bench;

pub use keydist::SchemeId;
pub use link::{ChannelLink, LinkSecret};
pub use log::{Insert, OpLog};
pub use marks::{ModNote, Review, REASON_SPAM};
pub use moderate::{InviteStatus, ModAction};
pub use moderation::{LogLine, QueueLine, UI_EVENT_CHANNEL_MODERATION, UI_EVENT_CHANNEL_QUEUE};
pub use op::{Comments, History, JoinMode, KeeperRef, KeyId, LevelCan, Op, OpBody, OpId, PostMode, Premod, Schedule, OP_VERSION};
pub use roles::{permits, Action, Role};
pub use schedule::{Status as ScheduleStatus, UnknownTz};
pub use state::{rules_at, ChannelState, Closed, Content, LevelDef, Reader, Rejection, Rules, ScopeDef, Staff, Standing};

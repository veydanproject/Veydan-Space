// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Groups.
//!
//! There is no server that owns a group. The group is a log of operations,
//! each made by a member who was allowed to make it, and every device
//! computes the same state from the same set of operations, whatever
//! order they arrived in.
//!
//! - `roles`: owner, admin, moderator, member, and what each may do.
//! - `op`: operations and their identity.
//! - `state`: the state of a group and how one operation changes it.
//! - `schedule`: the hours a group is open for posting, on the clock of a
//!   named time zone.
//! - `marks`: the marks of moderation (hide, unhide, approve, reject),
//!   whether one is in force by the journal now, and what the marks about
//!   one message add up to.
//! - `log`: the set of operations of a group, its linear order, the
//!   resulting state, and whether the current key can still be trusted.
//!
//! Everything in this crate is pure: no network, no storage, no clock.
//! Signatures are checked where operations enter (the wire layer); here an
//! operation's `author` is already known to be genuine.

pub mod keys;
pub mod log;
pub mod marks;
pub mod media;
pub mod moderation;
pub mod op;
pub mod qr;
pub mod roles;
pub mod schedule;
pub mod handler;
pub mod inbound;
#[cfg(test)]
mod scenarios;
pub mod service;
pub mod state;
pub mod wire;

pub use keys::{GroupKey, GroupLink, LinkSecret};
pub use log::{Insert, KeyStatus, OpLog};
pub use op::{GroupKind, JoinProof, KeyId, Op, OpBody, OpId, PostMode, Premod, Schedule, OP_VERSION};
pub use roles::{permits, Action, Role};
pub use handler::{GroupDmHandler, GroupHandler, Signal, Signals};
pub use service::{GroupCallSink, GroupService, GroupView, InviteView, JoinRequestView, KeyView, MemberView, Outcome};
pub use marks::{ModNote, Review, REASON_SPAM};
pub use moderation::{LogLine, QueueLine, UI_EVENT_GROUP_MODERATION, UI_EVENT_GROUP_QUEUE};
pub use schedule::{Status as ScheduleStatus, UnknownTz};
pub use state::{Closed, GroupState, Member, Rejection, ReplacedKey};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Group calls on an SFU (stage 7b of the plan of calls): the room of a
//! node, the words of the group about it, the keys of the frames, the
//! word of identity of every seat.
//!
//! - `signal`: the `call.start`, `call.join`, `call.leave`, `call.epoch`
//!   and `call.end` notes of a group, read and written.
//! - `keys`: the keys of the frames from the secret of an epoch, and the
//!   signed and sealed word of identity and the sealed word of state
//!   (camera, microphone, screen) on the control channel.
//! - `ctl`: the control channel of the room, as the node speaks it.
//! - `access`: the door to the groups ([`GroupAccess`]).
//! - `service`: [`GroupCallService`], the room I am in and the calls the
//!   groups announced.
//! - `feed`: the record and the line in the chat.
//! - `view`: what the screen sees.

pub mod access;
pub mod ctl;
pub mod feed;
pub mod keys;
pub mod service;
pub mod signal;
pub mod view;

pub use access::GroupAccess;
pub use service::{GroupCallService, MY_VIDEO_MID};
pub use signal::GroupSignal;
pub use view::{
    AnnouncedCall, GroupCallView, GroupPhase, ParticipantView, UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_LEVEL,
    UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
};

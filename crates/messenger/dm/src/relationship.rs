// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The DM relationship matrix as pure functions.
//!
//! There is no server that knows "A and B are contacts". Each device keeps
//! four independent facts about a peer and derives everything else:
//!
//! - `my_contact`: what I decided (none / approved / declined)
//! - `blocked`: my block (the only block flag)
//! - `peer_signal`: the last will of the peer, learned from control
//!   messages; cleared locally when I decline, remove or re-request
//! - `was_ever_mutual`: both were approved at some point
//!
//! No transition table: each fact changes on its own, and the screen mode,
//! the inbound decision and the outbound permission are computed. Nothing
//! here touches storage or the network.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MyContact {
    None,
    Approved,
    Declined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSignal {
    None,
    Approved,
    Blocked,
    Left,
    Declined,
    Revoked,
}

macro_rules! str_enum {
    ($t:ty { $($v:ident => $s:literal),+ $(,)? }) => {
        impl $t {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$v => $s),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some(Self::$v),)+ _ => None }
            }
        }
    };
}

str_enum!(MyContact { None => "none", Approved => "approved", Declined => "declined" });
str_enum!(PeerSignal {
    None => "none", Approved => "approved", Blocked => "blocked",
    Left => "left", Declined => "declined", Revoked => "revoked",
});

/// Control actions carried in `{"t":"control","action":…}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Accept,
    Decline,
    Block,
    Unblock,
    ContactRemoved,
    RequestCancelled,
}

str_enum!(Signal {
    Accept => "dm_accept", Decline => "dm_decline", Block => "dm_block", Unblock => "dm_unblock",
    ContactRemoved => "contact_removed", RequestCancelled => "request_cancelled",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub my_contact: MyContact,
    pub blocked: bool,
    pub peer_signal: PeerSignal,
    pub was_ever_mutual: bool,
    /// Rumor time of the newest peer signal applied.
    pub last_signal_at: i64,
    /// Rumor time of my newest action.
    pub last_my_signal_at: i64,
}

impl Default for Relationship {
    fn default() -> Self {
        Self {
            my_contact: MyContact::None,
            blocked: false,
            peer_signal: PeerSignal::None,
            was_ever_mutual: false,
            last_signal_at: 0,
            last_my_signal_at: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenMode {
    FullChat,
    FirstContact,
    RequestSent,
    RequestReceived,
    /// The peer declined or withdrew; I still have them as a contact.
    RequestDeclined,
    RequestDeclinedByMe,
    RequestRevokedByPeer,
    RemovedByPeer,
    BothRemoved,
    MutualReconnect,
    Blocked,
    BlockedByPeer,
}

str_enum!(ScreenMode {
    FullChat => "full_chat", FirstContact => "first_contact", RequestSent => "request_sent",
    RequestReceived => "request_received", RequestDeclined => "request_declined",
    RequestDeclinedByMe => "request_declined_by_me", RequestRevokedByPeer => "request_revoked_by_peer",
    RemovedByPeer => "removed_by_peer", BothRemoved => "both_removed", MutualReconnect => "mutual_reconnect",
    Blocked => "blocked", BlockedByPeer => "blocked_by_peer",
});

/// What the chat screen shows. My block beats everything.
pub fn screen_mode(r: &Relationship) -> ScreenMode {
    use MyContact as M;
    use PeerSignal as P;
    if r.blocked {
        return ScreenMode::Blocked;
    }
    if r.peer_signal == P::Blocked {
        return ScreenMode::BlockedByPeer;
    }
    if r.my_contact == M::Approved {
        return match r.peer_signal {
            P::Approved => ScreenMode::FullChat,
            P::Left => ScreenMode::RemovedByPeer,
            P::Declined | P::Revoked => ScreenMode::RequestDeclined,
            P::None | P::Blocked => ScreenMode::RequestSent,
        };
    }
    if r.peer_signal == P::Approved {
        // Even if I declined before: a fresh accept is a fresh request.
        return ScreenMode::RequestReceived;
    }
    if r.my_contact == M::Declined {
        return ScreenMode::RequestDeclinedByMe;
    }
    match r.peer_signal {
        P::Left => ScreenMode::BothRemoved,
        P::Revoked => ScreenMode::RequestRevokedByPeer,
        _ if r.was_ever_mutual => ScreenMode::MutualReconnect,
        _ => ScreenMode::FirstContact,
    }
}

// ─── Inbound ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboundDecision {
    /// Store and show.
    Save,
    /// Store and show as the one message a stranger may send.
    SaveAsRequest,
    /// Discard silently; the reason is for logs and tests.
    Drop(DropReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    BlockedByMe,
    BlockedByPeer,
    ConnectionEnded,
    SecondMessageBeforeApproval,
}

/// Decision for a regular (non-control) message from the peer.
///
/// `enforced` is false for restored history (older than this device was
/// last online): it is stored as it was. `visible_incoming` counts the
/// messages of this peer already shown in the chat.
pub fn inbound_decision(r: &Relationship, enforced: bool, visible_incoming: i64) -> InboundDecision {
    use InboundDecision as D;
    if !enforced {
        return D::Save;
    }
    if r.blocked {
        return D::Drop(DropReason::BlockedByMe);
    }
    if r.peer_signal == PeerSignal::Blocked {
        return D::Drop(DropReason::BlockedByPeer);
    }
    if r.my_contact == MyContact::Approved {
        // Whatever the peer's last signal was, they are my contact: deliver.
        return D::Save;
    }
    if r.peer_signal == PeerSignal::Left {
        return D::Drop(DropReason::ConnectionEnded);
    }
    if visible_incoming == 0 {
        D::SaveAsRequest
    } else {
        D::Drop(DropReason::SecondMessageBeforeApproval)
    }
}

/// A regular message from the peer says something about the peer's will
/// when no explicit signal did (clients that never send control messages):
/// writing to me means "approved". Explicit negative signals are kept.
pub fn signal_implied_by_message(r: &Relationship) -> Option<PeerSignal> {
    match r.peer_signal {
        PeerSignal::None | PeerSignal::Revoked => Some(PeerSignal::Approved),
        _ => None,
    }
}

// ─── Outbound ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundPermission {
    Allow,
    /// Allowed as the single request message: I become `approved` and a
    /// `dm_accept` follows the text.
    AllowAsRequest,
    Deny(DenyReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    Blocked,
    BlockedByPeer,
    /// The peer asked first; accept or decline before writing.
    AnswerRequestFirst,
    WaitingApproval,
    RequestDeclined,
    RemovedByPeer,
    FirstMessageMustBeText,
}

str_enum!(DenyReason {
    Blocked => "dm_blocked", BlockedByPeer => "dm_blocked_by_peer",
    AnswerRequestFirst => "dm_answer_request_first", WaitingApproval => "dm_waiting_approval",
    RequestDeclined => "dm_request_declined", RemovedByPeer => "dm_contact_removed_by_peer",
    FirstMessageMustBeText => "dm_first_message_must_be_text",
});

/// May I send a regular message now? `visible_outgoing` counts my messages
/// already in the chat; `is_text` is false for media.
pub fn outbound_permission(r: &Relationship, visible_outgoing: i64, is_text: bool) -> OutboundPermission {
    use OutboundPermission as O;
    if r.blocked {
        return O::Deny(DenyReason::Blocked);
    }
    match r.peer_signal {
        PeerSignal::Blocked => return O::Deny(DenyReason::BlockedByPeer),
        PeerSignal::Approved if r.my_contact == MyContact::Approved => return O::Allow,
        PeerSignal::Approved => return O::Deny(DenyReason::AnswerRequestFirst),
        PeerSignal::Declined if r.my_contact == MyContact::Approved => return O::Deny(DenyReason::RequestDeclined),
        PeerSignal::Left if r.my_contact == MyContact::Approved => return O::Deny(DenyReason::RemovedByPeer),
        _ => {}
    }
    // No approval from the peer (none, revoked, or a negative signal that I
    // answered by removing them): one text message may go out as a request.
    if r.my_contact == MyContact::Approved && visible_outgoing > 0 {
        return O::Deny(DenyReason::WaitingApproval);
    }
    if !is_text {
        return O::Deny(DenyReason::FirstMessageMustBeText);
    }
    O::AllowAsRequest
}

// ─── Peer signals ───────────────────────────────────────────────────────────

/// System line shown in the chat as a consequence of a signal or action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemLine {
    RequestSent,
    RequestReceived,
    RequestAccepted,
    RequestDeclined,
    RequestCancelled,
    ContactRemoved,
    ContactLeft,
    Blocked,
    Unblocked,
}

str_enum!(SystemLine {
    RequestSent => "request_sent", RequestReceived => "request_received", RequestAccepted => "request_accepted",
    RequestDeclined => "request_declined", RequestCancelled => "request_cancelled",
    ContactRemoved => "contact_removed", ContactLeft => "contact_left", Blocked => "blocked", Unblocked => "unblocked",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalOutcome {
    pub next: Relationship,
    /// False when the signal was stale or not applicable: nothing changes.
    pub applied: bool,
    /// Answer with my own `dm_accept` so the peer leaves "request sent".
    pub confirm_back: bool,
    pub system_line: Option<SystemLine>,
}

/// Apply a control signal from the peer. `at` is the rumor time, `live`
/// is false for anything older than the running session.
///
/// Order is decided by `at` against the stored `last_signal_at` (a replay
/// of history in any order cannot undo a newer signal), and historical
/// signals never trigger an answer.
pub fn apply_peer_signal(r: &Relationship, signal: Signal, at: i64, live: bool) -> SignalOutcome {
    use PeerSignal as P;
    let unchanged = SignalOutcome { next: *r, applied: false, confirm_back: false, system_line: None };
    if at < r.last_signal_at {
        return unchanged;
    }
    let target = match signal {
        Signal::Accept => Some(P::Approved),
        Signal::Decline if r.peer_signal != P::Approved => Some(P::Declined),
        Signal::Block => Some(P::Blocked),
        Signal::Unblock if r.peer_signal == P::Blocked => Some(P::None),
        Signal::ContactRemoved if matches!(r.peer_signal, P::Approved | P::None) => Some(P::Left),
        Signal::RequestCancelled if r.peer_signal != P::Approved => Some(P::Revoked),
        _ => None,
    };
    let Some(target) = target else {
        // Not applicable, but it still is the newest word of the peer.
        let mut next = *r;
        next.last_signal_at = at;
        return SignalOutcome { next, applied: false, confirm_back: false, system_line: None };
    };
    let changed = target != r.peer_signal;
    let mut next = *r;
    next.peer_signal = target;
    next.last_signal_at = at;
    let mine_approved = r.my_contact == MyContact::Approved && !r.blocked;
    if target == P::Approved && r.my_contact == MyContact::Approved {
        next.was_ever_mutual = true;
    }
    let system_line = if !changed || r.blocked {
        None
    } else {
        match target {
            // Re-approval after a block or a pause is not news.
            P::Approved if r.my_contact == MyContact::Approved && r.was_ever_mutual => None,
            P::Approved if r.my_contact == MyContact::Approved => Some(SystemLine::RequestAccepted),
            P::Approved => Some(SystemLine::RequestReceived),
            P::Declined => Some(SystemLine::RequestDeclined),
            P::Left => Some(SystemLine::ContactLeft),
            P::Revoked => Some(SystemLine::RequestCancelled),
            P::Blocked | P::None => None,
        }
    };
    SignalOutcome {
        next,
        applied: true,
        confirm_back: live && changed && target == P::Approved && mine_approved,
        system_line,
    }
}

// ─── My actions ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Add to contacts / send a request / first message to a stranger.
    Request,
    Accept,
    Decline,
    Block,
    Unblock,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    pub next: Relationship,
    /// Signals to send, in order.
    pub signals: Vec<Signal>,
    pub system_line: Option<SystemLine>,
}

/// What one of my actions does. `has_history` = the chat has any visible
/// message; `with_message` = the action accompanies my first text.
pub fn apply_my_action(r: &Relationship, action: Action, has_history: bool, with_message: bool) -> ActionOutcome {
    use PeerSignal as P;
    let mut next = *r;
    let mut signals = Vec::new();
    let mut system_line = None;
    match action {
        Action::Request => {
            let was_declined = r.my_contact == MyContact::Declined;
            next.my_contact = MyContact::Approved;
            // A new request of mine supersedes the peer's old "no".
            if matches!(r.peer_signal, P::Declined | P::Left | P::Revoked) {
                next.peer_signal = P::None;
            }
            if r.blocked {
                // Adding a blocked peer to contacts changes nothing for them.
            } else if r.peer_signal == P::Approved {
                next.was_ever_mutual = true;
                signals.push(Signal::Accept);
                system_line = Some(SystemLine::RequestAccepted);
            } else if with_message || was_declined || r.was_ever_mutual {
                // Adding a contact is silent; the peer learns with the
                // first message, or at once when there is a past to mend.
                signals.push(Signal::Accept);
                system_line = Some(SystemLine::RequestSent);
            }
        }
        Action::Accept => {
            next.my_contact = MyContact::Approved;
            if !r.blocked {
                signals.push(Signal::Accept);
            }
            if r.peer_signal == P::Approved {
                next.was_ever_mutual = true;
                system_line = Some(SystemLine::RequestAccepted);
            }
        }
        Action::Decline => {
            next.my_contact = MyContact::Declined;
            next.peer_signal = P::None;
            if !r.blocked {
                signals.push(Signal::Decline);
            }
            system_line = Some(SystemLine::RequestDeclined);
        }
        Action::Block => {
            if !r.blocked {
                next.blocked = true;
                signals.push(Signal::Block);
                system_line = Some(SystemLine::Blocked);
            }
        }
        Action::Unblock => {
            if r.blocked {
                next.blocked = false;
                signals.push(Signal::Unblock);
                // Leave the peer in a defined state.
                if r.my_contact == MyContact::Approved {
                    signals.push(Signal::Accept);
                } else {
                    next.my_contact = MyContact::None;
                    signals.push(Signal::ContactRemoved);
                }
                system_line = Some(SystemLine::Unblocked);
            }
        }
        Action::Remove => {
            let had_relation = r.my_contact == MyContact::Approved;
            next.my_contact = MyContact::None;
            // Their approval was for the relation I just ended; if they
            // write or accept again, that is a new request.
            if r.peer_signal == P::Approved {
                next.peer_signal = P::None;
            }
            if had_relation && !r.blocked {
                let only_pending = !r.was_ever_mutual && r.peer_signal == P::None;
                if !only_pending {
                    signals.push(Signal::ContactRemoved);
                } else if has_history {
                    signals.push(Signal::RequestCancelled);
                }
                // A pending request without history: stay silent, do not
                // reveal that they were ever added.
            }
            if had_relation && has_history {
                system_line = Some(SystemLine::ContactRemoved);
            }
        }
    }
    ActionOutcome { next, signals, system_line }
}

/// My own signal seen through a self-copy (another device of mine acted).
/// Only my side changes; stale copies are ignored.
pub fn apply_own_signal(r: &Relationship, signal: Signal, at: i64) -> Option<Relationship> {
    if at <= r.last_my_signal_at {
        return None;
    }
    let mut next = *r;
    next.last_my_signal_at = at;
    match signal {
        Signal::Accept => {
            next.my_contact = MyContact::Approved;
            if r.peer_signal == PeerSignal::Approved {
                next.was_ever_mutual = true;
            }
        }
        Signal::Decline => {
            next.my_contact = MyContact::Declined;
            next.peer_signal = PeerSignal::None;
        }
        Signal::Block => next.blocked = true,
        Signal::Unblock => next.blocked = false,
        Signal::ContactRemoved | Signal::RequestCancelled => next.my_contact = MyContact::None,
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use MyContact as M;
    use PeerSignal as P;
    use ScreenMode as S;

    fn rel(m: M, blocked: bool, p: P, mutual: bool) -> Relationship {
        Relationship { my_contact: m, blocked, peer_signal: p, was_ever_mutual: mutual, ..Default::default() }
    }

    #[test]
    fn screen_mode_table() {
        let cases: &[(M, bool, P, bool, S)] = &[
            // My block beats everything.
            (M::None, true, P::None, false, S::Blocked),
            (M::Approved, true, P::Approved, true, S::Blocked),
            (M::Approved, true, P::Blocked, true, S::Blocked),
            (M::Declined, true, P::Left, false, S::Blocked),
            // Then the peer's block.
            (M::None, false, P::Blocked, false, S::BlockedByPeer),
            (M::Approved, false, P::Blocked, true, S::BlockedByPeer),
            (M::Declined, false, P::Blocked, false, S::BlockedByPeer),
            // I approved.
            (M::Approved, false, P::Approved, true, S::FullChat),
            (M::Approved, false, P::Approved, false, S::FullChat),
            (M::Approved, false, P::Left, true, S::RemovedByPeer),
            (M::Approved, false, P::Declined, false, S::RequestDeclined),
            (M::Approved, false, P::Revoked, false, S::RequestDeclined),
            (M::Approved, false, P::None, false, S::RequestSent),
            (M::Approved, false, P::None, true, S::RequestSent),
            // I did not.
            (M::None, false, P::Approved, false, S::RequestReceived),
            (M::Declined, false, P::Approved, false, S::RequestReceived),
            (M::None, false, P::Approved, true, S::RequestReceived),
            (M::Declined, false, P::None, false, S::RequestDeclinedByMe),
            (M::Declined, false, P::Left, false, S::RequestDeclinedByMe),
            (M::None, false, P::Left, false, S::BothRemoved),
            (M::None, false, P::Left, true, S::BothRemoved),
            (M::None, false, P::Revoked, false, S::RequestRevokedByPeer),
            (M::None, false, P::None, true, S::MutualReconnect),
            (M::None, false, P::Declined, true, S::MutualReconnect),
            (M::None, false, P::None, false, S::FirstContact),
            (M::None, false, P::Declined, false, S::FirstContact),
        ];
        for (m, b, p, mutual, want) in cases {
            assert_eq!(screen_mode(&rel(*m, *b, *p, *mutual)), *want, "({m:?}, blocked={b}, {p:?}, mutual={mutual})");
        }
    }

    #[test]
    fn inbound_table() {
        use DropReason as R;
        use InboundDecision as D;
        let cases: &[(M, bool, P, bool, i64, D)] = &[
            // Restored history is stored as it was, whatever the state.
            (M::None, true, P::Blocked, false, 9, D::Save),
            (M::None, false, P::Left, false, 3, D::Save),
            // Blocks, mine first.
            (M::Approved, true, P::Approved, true, 0, D::Drop(R::BlockedByMe)),
            (M::None, true, P::Blocked, true, 0, D::Drop(R::BlockedByMe)),
            (M::Approved, false, P::Blocked, true, 0, D::Drop(R::BlockedByPeer)),
            (M::None, false, P::Blocked, true, 0, D::Drop(R::BlockedByPeer)),
            // My contact: always delivered.
            (M::Approved, false, P::Approved, true, 5, D::Save),
            (M::Approved, false, P::None, true, 0, D::Save),
            (M::Approved, false, P::Left, true, 2, D::Save),
            (M::Approved, false, P::Declined, true, 2, D::Save),
            (M::Approved, false, P::Revoked, true, 2, D::Save),
            // Not my contact.
            (M::None, false, P::Left, true, 0, D::Drop(R::ConnectionEnded)),
            (M::Declined, false, P::Left, true, 0, D::Drop(R::ConnectionEnded)),
            (M::None, false, P::None, true, 0, D::SaveAsRequest),
            (M::None, false, P::Approved, true, 0, D::SaveAsRequest),
            (M::None, false, P::Approved, true, 1, D::Drop(R::SecondMessageBeforeApproval)),
            (M::None, false, P::None, true, 4, D::Drop(R::SecondMessageBeforeApproval)),
            (M::Declined, false, P::None, true, 1, D::Drop(R::SecondMessageBeforeApproval)),
            (M::Declined, false, P::Approved, true, 0, D::SaveAsRequest),
            (M::None, false, P::Revoked, true, 0, D::SaveAsRequest),
            (M::None, false, P::Declined, true, 1, D::Drop(R::SecondMessageBeforeApproval)),
        ];
        for (m, b, p, enforced, seen, want) in cases {
            assert_eq!(
                inbound_decision(&rel(*m, *b, *p, false), *enforced, *seen),
                *want,
                "({m:?}, blocked={b}, {p:?}, enforced={enforced}, seen={seen})"
            );
        }
    }

    #[test]
    fn a_message_implies_approval_only_without_an_explicit_no() {
        assert_eq!(signal_implied_by_message(&rel(M::Approved, false, P::None, false)), Some(P::Approved));
        assert_eq!(signal_implied_by_message(&rel(M::None, false, P::Revoked, false)), Some(P::Approved));
        for p in [P::Approved, P::Blocked, P::Left, P::Declined] {
            assert_eq!(signal_implied_by_message(&rel(M::Approved, false, p, false)), None, "{p:?}");
        }
    }

    #[test]
    fn outbound_table() {
        use DenyReason as R;
        use OutboundPermission as O;
        let cases: &[(M, bool, P, i64, bool, O)] = &[
            (M::Approved, true, P::Approved, 3, true, O::Deny(R::Blocked)),
            (M::None, true, P::None, 0, true, O::Deny(R::Blocked)),
            (M::Approved, false, P::Blocked, 3, true, O::Deny(R::BlockedByPeer)),
            (M::None, false, P::Blocked, 0, true, O::Deny(R::BlockedByPeer)),
            // Mutual.
            (M::Approved, false, P::Approved, 0, true, O::Allow),
            (M::Approved, false, P::Approved, 9, false, O::Allow),
            // The peer asked first.
            (M::None, false, P::Approved, 0, true, O::Deny(R::AnswerRequestFirst)),
            (M::Declined, false, P::Approved, 0, true, O::Deny(R::AnswerRequestFirst)),
            // The peer said no and I keep them.
            (M::Approved, false, P::Declined, 1, true, O::Deny(R::RequestDeclined)),
            (M::Approved, false, P::Declined, 0, true, O::Deny(R::RequestDeclined)),
            (M::Approved, false, P::Left, 5, true, O::Deny(R::RemovedByPeer)),
            // Request: exactly one text.
            (M::Approved, false, P::None, 0, true, O::AllowAsRequest),
            (M::Approved, false, P::None, 0, false, O::Deny(R::FirstMessageMustBeText)),
            (M::Approved, false, P::None, 1, true, O::Deny(R::WaitingApproval)),
            (M::Approved, false, P::Revoked, 1, true, O::Deny(R::WaitingApproval)),
            (M::Approved, false, P::Revoked, 0, true, O::AllowAsRequest),
            // Stranger: the first message is the request, whatever was before.
            (M::None, false, P::None, 0, true, O::AllowAsRequest),
            (M::None, false, P::None, 0, false, O::Deny(R::FirstMessageMustBeText)),
            (M::None, false, P::None, 4, true, O::AllowAsRequest),
            (M::Declined, false, P::None, 2, true, O::AllowAsRequest),
            (M::None, false, P::Left, 2, true, O::AllowAsRequest),
            (M::None, false, P::Declined, 1, true, O::AllowAsRequest),
            (M::None, false, P::Revoked, 0, true, O::AllowAsRequest),
        ];
        for (m, b, p, sent, text, want) in cases {
            assert_eq!(
                outbound_permission(&rel(*m, *b, *p, false), *sent, *text),
                *want,
                "({m:?}, blocked={b}, {p:?}, sent={sent}, text={text})"
            );
        }
    }

    #[test]
    fn peer_signal_transitions() {
        // (current, signal) → expected peer_signal, applied
        let cases: &[(P, Signal, P, bool)] = &[
            (P::None, Signal::Accept, P::Approved, true),
            (P::Declined, Signal::Accept, P::Approved, true),
            (P::Blocked, Signal::Accept, P::Approved, true),
            (P::None, Signal::Decline, P::Declined, true),
            (P::Approved, Signal::Decline, P::Approved, false),
            (P::Approved, Signal::Block, P::Blocked, true),
            (P::None, Signal::Block, P::Blocked, true),
            (P::Blocked, Signal::Unblock, P::None, true),
            (P::Approved, Signal::Unblock, P::Approved, false),
            (P::Approved, Signal::ContactRemoved, P::Left, true),
            (P::None, Signal::ContactRemoved, P::Left, true),
            (P::Declined, Signal::ContactRemoved, P::Declined, false),
            (P::Blocked, Signal::ContactRemoved, P::Blocked, false),
            (P::None, Signal::RequestCancelled, P::Revoked, true),
            (P::Approved, Signal::RequestCancelled, P::Approved, false),
        ];
        for (cur, sig, want, applied) in cases {
            let out = apply_peer_signal(&rel(M::Approved, false, *cur, false), *sig, 10, true);
            assert_eq!((out.next.peer_signal, out.applied), (*want, *applied), "({cur:?}, {sig:?})");
            assert_eq!(out.next.last_signal_at, 10);
        }
    }

    #[test]
    fn stale_signals_cannot_undo_newer_ones() {
        let mut r = rel(M::Approved, false, P::None, false);
        // History replayed newest first: block at 200, then the accept from 100.
        r = apply_peer_signal(&r, Signal::Block, 200, false).next;
        let out = apply_peer_signal(&r, Signal::Accept, 100, false);
        assert!(!out.applied);
        assert_eq!(out.next.peer_signal, P::Blocked);
        assert_eq!(out.next.last_signal_at, 200);
        // Same time is accepted (two signals in one second keep arrival order).
        assert!(apply_peer_signal(&r, Signal::Unblock, 200, false).applied);
    }

    #[test]
    fn confirm_back_only_live_only_once_only_for_contacts() {
        let mine = rel(M::Approved, false, P::None, false);
        let live = apply_peer_signal(&mine, Signal::Accept, 10, true);
        assert!(live.confirm_back);
        assert!(live.next.was_ever_mutual);
        assert_eq!(live.system_line, Some(SystemLine::RequestAccepted));

        // The peer's confirm-back arrives at the side that accepted first:
        // already approved, so no answer and no second system line.
        let echo = apply_peer_signal(&live.next, Signal::Accept, 11, true);
        assert!(echo.applied && !echo.confirm_back);
        assert_eq!(echo.system_line, None);

        // Historical accept: applied, never answered.
        let hist = apply_peer_signal(&mine, Signal::Accept, 10, false);
        assert!(hist.applied && !hist.confirm_back);

        // A stranger's accept is a request, not something to confirm.
        let stranger = apply_peer_signal(&rel(M::None, false, P::None, false), Signal::Accept, 10, true);
        assert!(!stranger.confirm_back);
        assert!(!stranger.next.was_ever_mutual);
        assert_eq!(stranger.system_line, Some(SystemLine::RequestReceived));

        // While I block them nothing is answered or shown.
        let blocked = apply_peer_signal(&rel(M::Approved, true, P::None, false), Signal::Accept, 10, true);
        assert!(blocked.applied && !blocked.confirm_back);
        assert_eq!(blocked.system_line, None);
    }

    #[test]
    fn my_actions() {
        // First message to a stranger: request with accept.
        let o = apply_my_action(&Relationship::default(), Action::Request, false, true);
        assert_eq!(o.next.my_contact, M::Approved);
        assert_eq!(o.signals, vec![Signal::Accept]);
        assert_eq!(o.system_line, Some(SystemLine::RequestSent));
        assert_eq!(screen_mode(&o.next), S::RequestSent);

        // Adding a contact without a message is silent.
        let o = apply_my_action(&Relationship::default(), Action::Request, false, false);
        assert!(o.signals.is_empty() && o.system_line.is_none());

        // Re-adding after my decline, or after a mutual past, tells the peer.
        let o = apply_my_action(&rel(M::Declined, false, P::None, false), Action::Request, true, false);
        assert_eq!(o.signals, vec![Signal::Accept]);
        let o = apply_my_action(&rel(M::None, false, P::Left, true), Action::Request, true, false);
        assert_eq!(o.signals, vec![Signal::Accept]);
        assert_eq!(o.next.peer_signal, P::None, "my new request supersedes their old no");

        // Accept.
        let o = apply_my_action(&rel(M::None, false, P::Approved, false), Action::Accept, true, false);
        assert_eq!(screen_mode(&o.next), S::FullChat);
        assert!(o.next.was_ever_mutual);
        assert_eq!(o.signals, vec![Signal::Accept]);
        assert_eq!(o.system_line, Some(SystemLine::RequestAccepted));

        // Decline clears their signal so a new accept reads as a new request.
        let o = apply_my_action(&rel(M::None, false, P::Approved, false), Action::Decline, true, false);
        assert_eq!((o.next.my_contact, o.next.peer_signal), (M::Declined, P::None));
        assert_eq!(o.signals, vec![Signal::Decline]);
        assert_eq!(screen_mode(&o.next), S::RequestDeclinedByMe);
        let again = apply_peer_signal(&o.next, Signal::Accept, 50, true);
        assert_eq!(screen_mode(&again.next), S::RequestReceived);

        // Block keeps contact and signal; unblock leaves the peer defined.
        let full = rel(M::Approved, false, P::Approved, true);
        let b = apply_my_action(&full, Action::Block, true, false);
        assert!(b.next.blocked);
        assert_eq!((b.next.my_contact, b.next.peer_signal), (M::Approved, P::Approved));
        assert_eq!(b.signals, vec![Signal::Block]);
        assert!(apply_my_action(&b.next, Action::Block, true, false).signals.is_empty(), "idempotent");
        let u = apply_my_action(&b.next, Action::Unblock, true, false);
        assert_eq!(u.signals, vec![Signal::Unblock, Signal::Accept]);
        assert_eq!(screen_mode(&u.next), S::FullChat);
        let stranger_blocked = rel(M::None, true, P::None, false);
        let u = apply_my_action(&stranger_blocked, Action::Unblock, false, false);
        assert_eq!(u.signals, vec![Signal::Unblock, Signal::ContactRemoved]);

        // Remove.
        let r = apply_my_action(&full, Action::Remove, true, false);
        assert_eq!(r.signals, vec![Signal::ContactRemoved]);
        assert_eq!((r.next.my_contact, r.next.peer_signal), (M::None, P::None));
        assert_eq!(screen_mode(&r.next), S::MutualReconnect);
        let pending = rel(M::Approved, false, P::None, false);
        assert_eq!(apply_my_action(&pending, Action::Remove, true, false).signals, vec![Signal::RequestCancelled]);
        assert!(apply_my_action(&pending, Action::Remove, false, false).signals.is_empty(), "never reveal a silent add");
        assert!(apply_my_action(&rel(M::Approved, true, P::Approved, true), Action::Remove, true, false).signals.is_empty(), "blocked: no signal");
    }

    #[test]
    fn block_is_symmetric() {
        // A blocks B. A drops B's messages; B (after the signal) cannot send
        // and drops A's messages too.
        let a = apply_my_action(&rel(M::Approved, false, P::Approved, true), Action::Block, true, false).next;
        let b = apply_peer_signal(&rel(M::Approved, false, P::Approved, true), Signal::Block, 10, true).next;
        assert!(matches!(inbound_decision(&a, true, 3), InboundDecision::Drop(DropReason::BlockedByMe)));
        assert!(matches!(outbound_permission(&a, 3, true), OutboundPermission::Deny(DenyReason::Blocked)));
        assert!(matches!(inbound_decision(&b, true, 3), InboundDecision::Drop(DropReason::BlockedByPeer)));
        assert!(matches!(outbound_permission(&b, 3, true), OutboundPermission::Deny(DenyReason::BlockedByPeer)));
        assert_eq!(screen_mode(&a), S::Blocked);
        assert_eq!(screen_mode(&b), S::BlockedByPeer);
    }

    #[test]
    fn own_signals_from_another_device() {
        let r = rel(M::None, false, P::Approved, false);
        let n = apply_own_signal(&r, Signal::Accept, 10).unwrap();
        assert_eq!(n.my_contact, M::Approved);
        assert!(n.was_ever_mutual);
        assert!(apply_own_signal(&n, Signal::Block, 10).is_none(), "not newer");
        let n = apply_own_signal(&n, Signal::Block, 11).unwrap();
        assert!(n.blocked);
        assert!(!apply_own_signal(&n, Signal::Unblock, 12).unwrap().blocked);
    }

    #[test]
    fn names_round_trip() {
        for s in [Signal::Accept, Signal::Decline, Signal::Block, Signal::Unblock, Signal::ContactRemoved, Signal::RequestCancelled] {
            assert_eq!(Signal::parse(s.as_str()), Some(s));
        }
        assert_eq!(Signal::parse("nope"), None);
        assert_eq!(ScreenMode::RequestDeclinedByMe.as_str(), "request_declined_by_me");
        assert_eq!(DenyReason::WaitingApproval.as_str(), "dm_waiting_approval");
    }
}

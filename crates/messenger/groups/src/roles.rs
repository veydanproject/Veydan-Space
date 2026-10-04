// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Roles and the permission table. One function decides everything; the
//! state machine, the commands and the UI all ask it.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Member,
    Moderator,
    Admin,
    Owner,
}

impl Role {
    pub fn rank(self) -> u8 {
        match self {
            Self::Member => 1,
            Self::Moderator => 2,
            Self::Admin => 3,
            Self::Owner => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "member" => Self::Member,
            "moderator" => Self::Moderator,
            "admin" => Self::Admin,
            "owner" => Self::Owner,
            _ => return None,
        })
    }

    /// Owner and admins change membership and settings.
    pub fn is_manager(self) -> bool {
        self >= Self::Admin
    }
}

/// What someone wants to do. Actions on a person carry that person's
/// current role (`None`: not a member).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Post,
    /// Remove a message written by someone with this role.
    DeleteMessageOf(Role),
    Mute(Role),
    /// Approve a request or accept an invitation: add a new member.
    Admit,
    Invite,
    Remove(Role),
    /// Ban a member (with their role) or an outsider.
    Ban(Option<Role>),
    Unban,
    EditSettings,
    /// Give `to` to someone who is `from` now.
    SetRole { from: Role, to: Role },
    TransferOwnership,
    Disband,
    RotateKey,
    RotateLink,
    Leave,
}

/// May someone with `actor` role do `action`?
///
/// Two rules behind the table: managers act only on roles below their
/// own, and nobody grants a role equal to or above their own.
pub fn permits(actor: Role, action: Action) -> bool {
    use Action as A;
    use Role as R;
    match action {
        A::Post => true,
        A::DeleteMessageOf(target) | A::Mute(target) => actor >= R::Moderator && target < actor,
        A::Admit | A::Invite | A::Unban | A::EditSettings | A::RotateKey | A::RotateLink => actor.is_manager(),
        A::Remove(target) => actor.is_manager() && target < actor,
        A::Ban(target) => actor.is_manager() && target.is_none_or(|t| t < actor),
        A::SetRole { from, to } => {
            actor.is_manager() && from < actor && to < actor && to != R::Owner && from != to
        }
        A::TransferOwnership | A::Disband => actor == R::Owner,
        // The owner hands the group over or disbands it; walking away
        // would leave it without anyone who can appoint admins.
        A::Leave => actor != R::Owner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Role::{Admin as A, Member as U, Moderator as M, Owner as O};

    /// Rows of the table in docs/messenger-spec.md, stage 8:
    /// owner, admin, moderator, member.
    #[test]
    fn permission_table() {
        let rows: &[(&str, Action, [bool; 4])] = &[
            ("post", Action::Post, [true, true, true, true]),
            ("delete a member's message", Action::DeleteMessageOf(U), [true, true, true, false]),
            ("mute a member", Action::Mute(U), [true, true, true, false]),
            ("admit", Action::Admit, [true, true, false, false]),
            ("invite", Action::Invite, [true, true, false, false]),
            ("remove a member", Action::Remove(U), [true, true, false, false]),
            ("ban a member", Action::Ban(Some(U)), [true, true, false, false]),
            ("ban an outsider", Action::Ban(None), [true, true, false, false]),
            ("unban", Action::Unban, [true, true, false, false]),
            ("edit settings", Action::EditSettings, [true, true, false, false]),
            ("rotate key", Action::RotateKey, [true, true, false, false]),
            ("rotate link", Action::RotateLink, [true, true, false, false]),
            ("appoint a moderator", Action::SetRole { from: U, to: M }, [true, true, false, false]),
            ("appoint an admin", Action::SetRole { from: U, to: A }, [true, false, false, false]),
            ("transfer ownership", Action::TransferOwnership, [true, false, false, false]),
            ("disband", Action::Disband, [true, false, false, false]),
            ("leave", Action::Leave, [false, true, true, true]),
        ];
        for (what, action, want) in rows {
            for (role, expected) in [O, A, M, U].into_iter().zip(want) {
                assert_eq!(permits(role, *action), *expected, "{what} as {role:?}");
            }
        }
    }

    #[test]
    fn nobody_acts_on_equals_or_superiors() {
        // An admin cannot touch another admin or the owner.
        assert!(!permits(A, Action::Remove(A)));
        assert!(!permits(A, Action::Remove(O)));
        assert!(!permits(A, Action::Ban(Some(A))));
        assert!(!permits(A, Action::Mute(A)));
        assert!(!permits(A, Action::SetRole { from: A, to: U }), "demoting a fellow admin");
        assert!(permits(O, Action::Remove(A)));
        assert!(permits(O, Action::SetRole { from: A, to: U }));
        // A moderator moderates members only.
        assert!(permits(M, Action::DeleteMessageOf(U)));
        assert!(!permits(M, Action::DeleteMessageOf(M)));
        assert!(!permits(M, Action::Mute(A)));
        // Nobody makes a second owner through a role change.
        assert!(!permits(O, Action::SetRole { from: A, to: O }));
        assert!(!permits(O, Action::SetRole { from: M, to: M }), "no-op role change");
        // The owner is out of everyone's reach.
        for r in [O, A, M, U] {
            assert!(!permits(r, Action::Remove(O)), "{r:?}");
            assert!(!permits(r, Action::Mute(O)), "{r:?}");
        }
    }

    #[test]
    fn names_and_order() {
        assert!(O > A && A > M && M > U);
        for r in [O, A, M, U] {
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
        assert_eq!(Role::parse("viewer"), None);
        assert!(O.is_manager() && A.is_manager() && !M.is_manager() && !U.is_manager());
    }
}

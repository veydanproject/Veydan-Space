// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/roles.rs at 1f2ef31d, then changed for channels.

//! Roles and the permission table (internal/communities-wire.md §3). One
//! function decides everything; the state machine, the commands and the
//! UI all ask it.
//!
//! A channel adds the author between the moderator and the member: an
//! author writes posts when posting is for authors. A member is a reader:
//! the journal does not list readers at all (the keeper does), so a
//! member by the journal is somebody it says something about (muted,
//! trusted, a former author).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Member,
    Author,
    Moderator,
    Admin,
    Owner,
}

impl Role {
    pub fn rank(self) -> u8 {
        match self {
            Self::Member => 1,
            Self::Author => 2,
            Self::Moderator => 3,
            Self::Admin => 4,
            Self::Owner => 5,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Author => "author",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "member" => Self::Member,
            "author" => Self::Author,
            "moderator" => Self::Moderator,
            "admin" => Self::Admin,
            "owner" => Self::Owner,
            _ => return None,
        })
    }

    /// Owner and admins change the staff and the settings.
    pub fn is_manager(self) -> bool {
        self >= Self::Admin
    }

    /// Moderators and above: bans, mutes, pins, the link, the keeper's
    /// urgent epoch.
    pub fn is_moderator(self) -> bool {
        self >= Self::Moderator
    }
}

/// What someone wants to do. Actions on a person carry that person's
/// current role (`Member` for a reader the journal does not list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Write a post: authors and above, always (stage 1 fixes `post_mode`
    /// to `authors`, wire §3, 2026-10-22); readers comment.
    Post,
    /// Write a comment under a post (while `comments: on`).
    Comment,
    /// Remove (hide) a message written by someone with this role.
    DeleteMessageOf(Role),
    Mute(Role),
    /// Mark someone with this role as trusted (or take it back): their
    /// messages are never held for moderation.
    Trust(Role),
    /// Set the levels of a person (wire §14.1).
    SetLevels,
    /// Make a scope (`scope_add`).
    EditScopes,
    /// Delete a scope (`scope_del`): the owner alone, as a disband of a
    /// part of the channel.
    DeleteScope,
    /// Rename a scope, change its look, hide it (`scope_set`, a hidden
    /// `scope_add`): the owner alone (2026-10-23).
    SetScope,
    /// Say whose quota a member of the staff uploads into (`set_quota`,
    /// wire §9.2): the owner alone, whose plan pays (2026-10-23).
    SetQuota,
    /// Make, change or delete a level (`level_set`, `level_del`).
    EditLevels,
    /// Let a reader in: an answer to a request (`channel.admit`) or an
    /// invitation (`channel.invite`).
    Admit,
    Invite,
    Remove(Role),
    /// Ban a person with this role (`Member` for a reader or an outsider).
    Ban(Role),
    Unban,
    EditSettings,
    /// Give `to` to someone who is `from` now.
    SetRole { from: Role, to: Role },
    /// Make someone an author or take it back: the row `set_role` of the
    /// table with `author` on either side.
    SetAuthor(Role),
    /// Pin or unpin a post: open in the journal, so never its author's
    /// right (it would tell who wrote the post).
    Pin,
    /// Delegate the keeper to a key (`keeper`).
    Keeper,
    /// Ask the keeper for an epoch now (`rotate_now`).
    RotateNow,
    RotateLink,
    TransferOwnership,
    Disband,
    Leave,
}

/// May someone with `actor` role do `action`?
///
/// Two rules behind the table, as in groups: one acts only on roles below
/// one's own, and nobody grants a role equal to or above their own.
pub fn permits(actor: Role, action: Action) -> bool {
    use Action as A;
    use Role as R;
    match action {
        A::Post => actor >= R::Author,
        A::Comment => true,
        A::DeleteMessageOf(target) | A::Mute(target) | A::Trust(target) | A::Remove(target) | A::Ban(target) => {
            actor.is_moderator() && target < actor
        }
        A::SetLevels | A::Admit | A::Invite | A::Unban | A::Pin | A::RotateNow | A::RotateLink => actor.is_moderator(),
        A::EditSettings | A::EditScopes | A::EditLevels => actor.is_manager(),
        A::DeleteScope | A::SetScope | A::SetQuota => actor == R::Owner,
        A::SetRole { from, to } => {
            // An admin is made by the owner alone.
            actor.is_manager() && from < actor && to < actor && to != R::Owner && from != to && (to != R::Admin || actor == R::Owner)
        }
        A::SetAuthor(target) => actor.is_manager() && target < actor,
        A::Keeper | A::TransferOwnership | A::Disband => actor == R::Owner,
        // The owner hands the channel over or disbands it; walking away
        // would leave it without anyone who can appoint admins.
        A::Leave => actor != R::Owner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Role::{Admin as A, Author as W, Member as U, Moderator as M, Owner as O};

    /// Rows of the table of internal/communities-wire.md §3: owner, admin,
    /// moderator, author, member.
    #[test]
    fn permission_table() {
        let rows: &[(&str, Action, [bool; 5])] = &[
            ("post (authors only)", Action::Post, [true, true, true, true, false]),
            ("comment", Action::Comment, [true, true, true, true, true]),
            ("delete a member's message", Action::DeleteMessageOf(U), [true, true, true, false, false]),
            ("delete an author's message", Action::DeleteMessageOf(W), [true, true, true, false, false]),
            ("mute a member", Action::Mute(U), [true, true, true, false, false]),
            ("trust a member", Action::Trust(U), [true, true, true, false, false]),
            ("set levels", Action::SetLevels, [true, true, true, false, false]),
            ("admit", Action::Admit, [true, true, true, false, false]),
            ("invite", Action::Invite, [true, true, true, false, false]),
            ("remove a member", Action::Remove(U), [true, true, true, false, false]),
            ("ban a member", Action::Ban(U), [true, true, true, false, false]),
            ("ban an author", Action::Ban(W), [true, true, true, false, false]),
            ("unban", Action::Unban, [true, true, true, false, false]),
            ("pin", Action::Pin, [true, true, true, false, false]),
            ("rotate now", Action::RotateNow, [true, true, true, false, false]),
            ("rotate link", Action::RotateLink, [true, true, true, false, false]),
            ("edit settings", Action::EditSettings, [true, true, false, false, false]),
            ("make an author", Action::SetAuthor(U), [true, true, false, false, false]),
            ("appoint an author", Action::SetRole { from: U, to: W }, [true, true, false, false, false]),
            ("appoint a moderator", Action::SetRole { from: U, to: M }, [true, true, false, false, false]),
            ("appoint an admin", Action::SetRole { from: U, to: A }, [true, false, false, false, false]),
            ("delegate the keeper", Action::Keeper, [true, false, false, false, false]),
            ("transfer ownership", Action::TransferOwnership, [true, false, false, false, false]),
            ("disband", Action::Disband, [true, false, false, false, false]),
            ("leave", Action::Leave, [false, true, true, true, true]),
        ];
        for (what, action, want) in rows {
            for (role, expected) in [O, A, M, W, U].into_iter().zip(want) {
                assert_eq!(permits(role, *action), *expected, "{what} as {role:?}");
            }
        }
    }

    #[test]
    fn nobody_acts_on_equals_or_superiors() {
        // An admin cannot touch another admin or the owner.
        assert!(!permits(A, Action::Remove(A)));
        assert!(!permits(A, Action::Remove(O)));
        assert!(!permits(A, Action::Ban(A)));
        assert!(!permits(A, Action::Mute(A)));
        assert!(!permits(A, Action::SetRole { from: A, to: U }), "demoting a fellow admin");
        assert!(!permits(A, Action::SetRole { from: M, to: A }), "an admin is the owner's to make");
        assert!(permits(O, Action::Remove(A)));
        assert!(permits(O, Action::SetRole { from: A, to: U }));
        // A moderator moderates authors and members only.
        assert!(permits(M, Action::DeleteMessageOf(W)));
        assert!(permits(M, Action::Ban(W)));
        assert!(!permits(M, Action::DeleteMessageOf(M)));
        assert!(!permits(M, Action::Mute(A)));
        assert!(!permits(M, Action::Ban(M)));
        assert!(!permits(M, Action::Trust(M)));
        // An author moderates nobody.
        assert!(!permits(W, Action::Mute(U)));
        assert!(!permits(W, Action::Pin));
        // Nobody makes a second owner through a role change.
        assert!(!permits(O, Action::SetRole { from: A, to: O }));
        assert!(!permits(O, Action::SetRole { from: M, to: M }), "no-op role change");
        // The owner is out of everyone's reach.
        for r in [O, A, M, W, U] {
            assert!(!permits(r, Action::Remove(O)), "{r:?}");
            assert!(!permits(r, Action::Mute(O)), "{r:?}");
            assert!(!permits(r, Action::Ban(O)), "{r:?}");
        }
    }

    #[test]
    fn names_and_order() {
        assert!(O > A && A > M && M > W && W > U);
        for r in [O, A, M, W, U] {
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
        assert_eq!(Role::parse("viewer"), None);
        assert!(O.is_manager() && A.is_manager() && !M.is_manager() && !W.is_manager() && !U.is_manager());
        assert!(M.is_moderator() && !W.is_moderator());
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/marks.rs at 1f2ef31d, then changed for channels.

//! Marks of moderation: what a moderator decided of a message
//! (`cmod.hide`, `cmod.hide_author`, `cmod.unhide`) or of one that waits
//! for them (`cmod.approve`, `cmod.reject`), and what the marks about one
//! message add up to (internal/communities-wire.md §4, `t:"mod"`).
//!
//! A mark is a note sealed into the feed and signed by its author, kept as
//! it came. Whether it is in force is not written into it: the journal as
//! it stands now says who is a moderator and whom they are above, and
//! that is read again whenever the journal changes (`active`). So a
//! demoted moderator loses the force of every mark they made, on every
//! device, and the same marks and the same journal give the same answer
//! whatever order they came in. The author of a post hides and shows the
//! comments under it (the note is sealed: that right tells the relay
//! nothing). Among the marks in force about one message the newest by
//! its author's clock wins (`resolve`); the clock is the author's word,
//! taken only inside a window, never cut.
//!
//! Everything here is pure: the service stores and applies.

use crate::roles::Role;
use crate::state::ChannelState;
use messenger_core::{Envelope, PubKey};

pub const T_HIDE: &str = "cmod.hide";
pub const T_HIDE_AUTHOR: &str = "cmod.hide_author";
pub const T_UNHIDE: &str = "cmod.unhide";
pub const T_APPROVE: &str = "cmod.approve";
pub const T_REJECT: &str = "cmod.reject";
/// The prefix of every note of moderation.
pub const T_PREFIX: &str = "cmod.";

/// The kinds of marks as they are kept (`cm_marks.kind`).
pub const KIND_HIDE: &str = "hide";
pub const KIND_UNHIDE: &str = "unhide";
pub const KIND_APPROVE: &str = "approve";
pub const KIND_REJECT: &str = "reject";

/// The review of a message as it is kept (`cm_posts.review`,
/// `cm_comments.review`).
pub const REVIEW_OK: &str = "ok";
pub const REVIEW_PENDING: &str = "pending";
pub const REVIEW_APPROVED: &str = "approved";
pub const REVIEW_REJECTED: &str = "rejected";
pub const REVIEW_RELEASED: &str = "released";

/// Messages one note may name.
pub const MAX_TARGETS: usize = 64;
/// How far ahead of my clock a note may be dated and still count.
pub const SKEW_SECS: i64 = 300;
/// The one reason a hide (and a ban) may carry.
pub const REASON_SPAM: &str = "spam";

/// One mark as it is kept: one row per message a note names, or one for
/// a span of an author's messages (`cm_marks`).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MarkRow {
    /// The inner event of the note (signed by `by`).
    pub note_id: String,
    pub community_id: String,
    /// `hide` | `unhide` | `approve` | `reject`
    pub kind: String,
    /// The message the mark is about; empty for a mark about a span of an
    /// author's messages.
    pub target_id: String,
    /// Of a span: whose messages, between `from_at` and `to_at` (their
    /// `created_at`, inclusive).
    pub author: Option<String>,
    pub from_at: Option<i64>,
    pub to_at: Option<i64>,
    /// Of an approval: the version of the message it approves.
    pub version: Option<String>,
    pub reason: String,
    /// Hex pubkey of the moderator (or of the author of the post).
    pub by: String,
    /// The moderator's clock, as signed.
    pub at: i64,
    /// In force by the journal as it stands now.
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hide,
    Unhide,
    Approve,
    Reject,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Hide => KIND_HIDE,
            Kind::Unhide => KIND_UNHIDE,
            Kind::Approve => KIND_APPROVE,
            Kind::Reject => KIND_REJECT,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            KIND_HIDE => Kind::Hide,
            KIND_UNHIDE => Kind::Unhide,
            KIND_APPROVE => Kind::Approve,
            KIND_REJECT => Kind::Reject,
            _ => return None,
        })
    }

    /// Hide and unhide are about what the channel sees; approve and reject
    /// about what waited for a moderator.
    fn of_visibility(self) -> bool {
        matches!(self, Kind::Hide | Kind::Unhide)
    }

    /// Of two marks dated alike, the stricter one stands.
    fn weight(self) -> u8 {
        match self {
            Kind::Hide | Kind::Reject => 1,
            Kind::Unhide | Kind::Approve => 0,
        }
    }
}

/// A note of moderation as it is sent and read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModNote {
    Hide { targets: Vec<String>, reason: String },
    /// Everything `author` wrote dated `from..=to`.
    HideAuthor { author: PubKey, from: i64, to: i64, reason: String },
    Unhide { targets: Vec<String> },
    Approve { target: String, version: String },
    Reject { target: String },
}

fn is_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn targets_of(e: &Envelope) -> Result<Vec<String>, &'static str> {
    let list = e.fields.get("targets").and_then(|v| v.as_array()).ok_or("targets")?;
    if list.is_empty() || list.len() > MAX_TARGETS {
        return Err("targets");
    }
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let id = v.as_str().filter(|s| is_id(s)).ok_or("targets")?;
        if !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
    }
    Ok(out)
}

fn reason_of(e: &Envelope) -> Result<String, &'static str> {
    match e.str_field("reason").unwrap_or("") {
        "" => Ok(String::new()),
        REASON_SPAM => Ok(REASON_SPAM.into()),
        _ => Err("reason"),
    }
}

impl ModNote {
    pub fn kind(&self) -> Kind {
        match self {
            ModNote::Hide { .. } | ModNote::HideAuthor { .. } => Kind::Hide,
            ModNote::Unhide { .. } => Kind::Unhide,
            ModNote::Approve { .. } => Kind::Approve,
            ModNote::Reject { .. } => Kind::Reject,
        }
    }

    /// The messages the note names by id; none for a span.
    pub fn targets(&self) -> &[String] {
        match self {
            ModNote::Hide { targets, .. } | ModNote::Unhide { targets } => targets,
            ModNote::Approve { target, .. } | ModNote::Reject { target } => std::slice::from_ref(target),
            ModNote::HideAuthor { .. } => &[],
        }
    }

    /// The envelope of the note, as it is sealed into the feed.
    pub fn envelope(&self) -> Envelope {
        match self {
            ModNote::Hide { targets, reason } => {
                let e = Envelope::new(T_HIDE).with("targets", targets.clone());
                if reason.is_empty() { e } else { e.with("reason", reason.as_str()) }
            }
            ModNote::HideAuthor { author, from, to, reason } => {
                let e = Envelope::new(T_HIDE_AUTHOR).with("author", author.as_hex()).with("from", *from).with("to", *to);
                if reason.is_empty() { e } else { e.with("reason", reason.as_str()) }
            }
            ModNote::Unhide { targets } => Envelope::new(T_UNHIDE).with("targets", targets.clone()),
            ModNote::Approve { target, version } => Envelope::new(T_APPROVE).with("target", target.as_str()).with("version", version.as_str()),
            ModNote::Reject { target } => Envelope::new(T_REJECT).with("target", target.as_str()),
        }
    }

    /// Read a note; `Err` names the field that is not right. `None` when
    /// the type is one of moderation this build does not know: dropped
    /// quietly, like every unknown note.
    pub fn parse(e: &Envelope) -> Result<Option<Self>, &'static str> {
        Ok(Some(match e.t.as_str() {
            T_HIDE => ModNote::Hide { targets: targets_of(e)?, reason: reason_of(e)? },
            T_HIDE_AUTHOR => {
                let reason = reason_of(e)?;
                let author = e.str_field("author").and_then(PubKey::parse).ok_or("author")?;
                let from = e.fields.get("from").and_then(|v| v.as_i64()).ok_or("from")?;
                let to = e.fields.get("to").and_then(|v| v.as_i64()).ok_or("to")?;
                if from < 0 || from > to {
                    return Err("range");
                }
                ModNote::HideAuthor { author, from, to, reason }
            }
            T_UNHIDE => ModNote::Unhide { targets: targets_of(e)? },
            T_APPROVE => {
                let target = e.str_field("target").filter(|s| is_id(s)).ok_or("target")?;
                let version = e.str_field("version").filter(|s| is_id(s)).ok_or("version")?;
                ModNote::Approve { target: target.into(), version: version.into() }
            }
            T_REJECT => ModNote::Reject { target: e.str_field("target").filter(|s| is_id(s)).ok_or("target")?.into() },
            _ => return Ok(None),
        }))
    }

    /// The rows the note is kept as: one per message it names, or one
    /// for the span. `active` is left for the service to say.
    pub fn rows(&self, community_id: &str, note_id: &str, by: &PubKey, at: i64) -> Vec<MarkRow> {
        let row = |target: &str, author: Option<&PubKey>, range: Option<(i64, i64)>, version: Option<&str>, reason: &str| MarkRow {
            note_id: note_id.to_string(),
            community_id: community_id.to_string(),
            kind: self.kind().as_str().into(),
            target_id: target.to_string(),
            author: author.map(|a| a.as_hex().to_string()),
            from_at: range.map(|r| r.0),
            to_at: range.map(|r| r.1),
            version: version.map(String::from),
            reason: reason.to_string(),
            by: by.as_hex().to_string(),
            at,
            active: false,
        };
        match self {
            ModNote::Hide { targets, reason } => targets.iter().map(|t| row(t, None, None, None, reason)).collect(),
            ModNote::HideAuthor { author, from, to, reason } => vec![row("", Some(author), Some((*from, *to)), None, reason)],
            ModNote::Unhide { targets } => targets.iter().map(|t| row(t, None, None, None, "")).collect(),
            ModNote::Approve { target, version } => vec![row(target, None, None, Some(version), "")],
            ModNote::Reject { target } => vec![row(target, None, None, None, "")],
        }
    }
}

/// Whether a mark is in force by the state of the channel now: its author
/// is a moderator or above, and, for a hide or an unhide, above the one
/// whose message it is (`target_author`: of the message when it is here,
/// of the span otherwise; somebody the journal does not name counts as a
/// reader, an unknown message as a reader's). An approval or a rejection
/// takes the role of its author alone. The author of a post
/// (`thread_author`, of a comment's post) hides and shows by id the
/// comments under it of anybody below a moderator, not their own.
pub fn active(mark: &MarkRow, state: &ChannelState, target_author: Option<&str>, thread_author: Option<&str>) -> bool {
    if state.disbanded {
        return false;
    }
    let Some(by_key) = PubKey::parse(&mark.by) else { return false };
    if state.is_banned(&by_key) {
        return false;
    }
    let Some(kind) = Kind::parse(&mark.kind) else { return false };
    let author = target_author.or(mark.author.as_deref());
    let target = author.and_then(PubKey::parse).and_then(|p| state.role_of(&p)).unwrap_or(Role::Member);
    match state.role_of(&by_key) {
        Some(by) if by >= Role::Moderator => !kind.of_visibility() || target < by,
        _ => {
            kind.of_visibility()
                && !mark.target_id.is_empty()
                && thread_author == Some(mark.by.as_str())
                && author.is_some_and(|a| a != mark.by)
                && target < Role::Moderator
        }
    }
}

/// Whether a message waits for a moderator and what came of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Review {
    /// Never held.
    Ok,
    Pending,
    Approved,
    Rejected,
    /// Held once, shown now: the journal no longer holds its author and
    /// nobody decided (`REVIEW_RELEASED`).
    Released,
}

impl Review {
    pub fn as_str(self) -> &'static str {
        match self {
            Review::Ok => REVIEW_OK,
            Review::Pending => REVIEW_PENDING,
            Review::Approved => REVIEW_APPROVED,
            Review::Rejected => REVIEW_REJECTED,
            Review::Released => REVIEW_RELEASED,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub hidden: bool,
    pub review: Review,
}

/// What the marks about one message add up to. `marks`: those that name
/// it and those about a span its date falls into, in force or not (only
/// those in force count). A mark about the message by id dated before the
/// message does not count: its author did not know the message then.
/// `version` is the message's current version (`db::content::MessageRow::version`):
/// an approval counts only for the version it names. `held`: whether
/// the message was held for a moderator when it came; otherwise
/// approvals and rejections say nothing of it. `subject`: whether the
/// journal holds its author now (`ChannelState::subject_to_premod`); one
/// held that nobody decided waits while it does and is released when it
/// does not, and waits again should it again. So whether a message is
/// seen is a function of the journal as it stands and the marks, the
/// same on every device, whatever came first.
///
/// Among what counts, the newest by `at` wins; dated alike, a hide beats
/// an unhide and a rejection an approval; still alike, the smaller note
/// id. A rejected message is hidden too.
pub fn resolve(marks: &[MarkRow], created_at: i64, version: &str, held: bool, subject: bool) -> Resolution {
    let counts = |m: &&MarkRow| m.active && (m.target_id.is_empty() || m.at >= created_at);
    let key = |m: &MarkRow, kind: Kind| (m.at, kind.weight(), std::cmp::Reverse(m.note_id.clone()));
    let winner = |of_visibility: bool| {
        marks
            .iter()
            .filter(counts)
            .filter_map(|m| Kind::parse(&m.kind).map(|k| (m, k)))
            .filter(|(_, k)| k.of_visibility() == of_visibility)
            .filter(|(m, k)| *k != Kind::Approve || m.version.as_deref() == Some(version))
            .max_by_key(|(m, k)| key(m, *k))
            .map(|(_, k)| k)
    };
    let review = if !held {
        Review::Ok
    } else {
        match winner(false) {
            Some(Kind::Approve) => Review::Approved,
            Some(Kind::Reject) => Review::Rejected,
            _ if subject => Review::Pending,
            _ => Review::Released,
        }
    };
    let hidden = winner(true) == Some(Kind::Hide) || review == Review::Rejected;
    Resolution { hidden, review }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::pk;

    fn mark(note: &str, kind: Kind, at: i64, active: bool) -> MarkRow {
        MarkRow {
            note_id: note.into(),
            community_id: "g".into(),
            kind: kind.as_str().into(),
            target_id: "t".into(),
            author: None,
            from_at: None,
            to_at: None,
            version: None,
            reason: String::new(),
            by: pk("a").as_hex().into(),
            at,
            active,
        }
    }

    #[test]
    fn the_newest_mark_in_force_wins_and_ties_go_to_the_stricter_then_the_smaller_id() {
        let none = resolve(&[], 0, "v", false, false);
        assert_eq!(none, Resolution { hidden: false, review: Review::Ok });
        let hide = mark("n2", Kind::Hide, 10, true);
        let unhide = mark("n1", Kind::Unhide, 10, true);
        assert!(resolve(&[hide.clone(), unhide.clone()], 0, "v", false, false).hidden, "dated alike: the hide");
        let later = mark("n3", Kind::Unhide, 11, true);
        assert!(!resolve(&[hide.clone(), unhide.clone(), later], 0, "v", false, false).hidden, "the newest");
        let inactive = mark("n0", Kind::Hide, 50, false);
        assert!(!resolve(&[unhide.clone(), inactive], 0, "v", false, false).hidden, "a mark out of force says nothing");
        // Dated alike, the same kind: the smaller note id, whatever the order given.
        let a = mark("a", Kind::Unhide, 10, true);
        let b = mark("b", Kind::Hide, 10, true);
        assert!(resolve(&[a.clone(), b.clone()], 0, "v", false, false).hidden);
        assert!(resolve(&[b, a], 0, "v", false, false).hidden);
        // A mark by id dated before the message does not count; a span does.
        assert!(!resolve(std::slice::from_ref(&hide), 20, "v", false, false).hidden);
        let span = MarkRow { target_id: String::new(), author: Some(pk("c").as_hex().into()), from_at: Some(0), to_at: Some(100), ..hide };
        assert!(resolve(&[span], 20, "v", false, false).hidden);
    }

    #[test]
    fn approval_counts_for_its_version_only_and_a_rejection_hides() {
        let approve = MarkRow { version: Some("v1".into()), ..mark("n1", Kind::Approve, 10, true) };
        let r = resolve(std::slice::from_ref(&approve), 0, "v1", true, true);
        assert_eq!(r, Resolution { hidden: false, review: Review::Approved });
        assert_eq!(resolve(std::slice::from_ref(&approve), 0, "v2", true, true).review, Review::Pending, "edited since: waits again");
        assert_eq!(resolve(std::slice::from_ref(&approve), 0, "v1", false, false).review, Review::Ok, "never held: nothing to approve");
        let reject = mark("n2", Kind::Reject, 11, true);
        assert_eq!(resolve(&[approve.clone(), reject.clone()], 0, "v1", true, true), Resolution { hidden: true, review: Review::Rejected });
        let again = MarkRow { version: Some("v1".into()), ..mark("n3", Kind::Approve, 12, true) };
        assert_eq!(resolve(&[approve, reject, again], 0, "v1", true, true).review, Review::Approved);
        // Nobody decided: waits. A hide on a waiting message hides it.
        assert_eq!(resolve(&[], 0, "v1", true, true), Resolution { hidden: false, review: Review::Pending });
        assert_eq!(resolve(&[mark("n4", Kind::Hide, 5, true)], 0, "v1", true, true), Resolution { hidden: true, review: Review::Pending });
    }

    #[test]
    fn what_was_held_is_released_while_the_journal_does_not_hold_its_author_and_a_decision_stands_either_way() {
        // Held, nobody decided, the author not held any more: shown.
        assert_eq!(resolve(&[], 0, "v1", true, false), Resolution { hidden: false, review: Review::Released });
        // Never held: nothing to release.
        assert_eq!(resolve(&[], 0, "v1", false, false).review, Review::Ok);
        // A decision counts whether or not the journal still holds the author.
        let approve = MarkRow { version: Some("v1".into()), ..mark("n1", Kind::Approve, 10, true) };
        assert_eq!(resolve(std::slice::from_ref(&approve), 0, "v1", true, false).review, Review::Approved);
        let reject = mark("n2", Kind::Reject, 11, true);
        assert_eq!(resolve(std::slice::from_ref(&reject), 0, "v1", true, false), Resolution { hidden: true, review: Review::Rejected });
        // Edited since the approval, the author released: shown, not waiting.
        assert_eq!(resolve(std::slice::from_ref(&approve), 0, "v2", true, false).review, Review::Released);
        // A hide stands over a release.
        assert_eq!(resolve(&[mark("n4", Kind::Hide, 5, true)], 0, "v1", true, false), Resolution { hidden: true, review: Review::Released });
    }

    #[test]
    fn notes_round_trip_through_the_envelope_and_bad_ones_are_refused() {
        let id = "ab".repeat(32);
        let notes = [
            ModNote::Hide { targets: vec![id.clone()], reason: REASON_SPAM.into() },
            ModNote::HideAuthor { author: pk("c"), from: 0, to: 100, reason: String::new() },
            ModNote::Unhide { targets: vec![id.clone(), "cd".repeat(32)] },
            ModNote::Approve { target: id.clone(), version: "ef".repeat(32) },
            ModNote::Reject { target: id.clone() },
        ];
        for n in notes {
            let e = Envelope::parse(&n.envelope().encode()).unwrap();
            assert!(e.t.starts_with(T_PREFIX));
            assert_eq!(ModNote::parse(&e).unwrap(), Some(n.clone()), "{n:?}");
            let rows = n.rows("g", "note", &pk("b"), 7);
            assert_eq!(rows.len(), if let ModNote::Unhide { targets } = &n { targets.len() } else { 1 });
            assert!(rows.iter().all(|r| !r.active && r.at == 7 && r.kind == n.kind().as_str()));
        }
        assert_eq!(ModNote::parse(&Envelope::new("cmod.later")).unwrap(), None, "unknown: quietly nothing");
        assert_eq!(ModNote::parse(&Envelope::new("mod.hide").with("targets", vec![id.clone()])).unwrap(), None, "a note of groups is no note of a channel");
        assert_eq!(ModNote::parse(&Envelope::new(T_HIDE).with("targets", Vec::<String>::new())), Err("targets"));
        let many: Vec<String> = (0..65).map(|i| format!("{i:064x}")).collect();
        assert_eq!(ModNote::parse(&Envelope::new(T_HIDE).with("targets", many)), Err("targets"));
        assert_eq!(ModNote::parse(&Envelope::new(T_HIDE).with("targets", vec!["short"])), Err("targets"));
        assert_eq!(ModNote::parse(&Envelope::new(T_HIDE).with("targets", vec![id.clone()]).with("reason", "rude")), Err("reason"));
        assert_eq!(ModNote::parse(&Envelope::new(T_HIDE_AUTHOR).with("author", pk("c").as_hex()).with("from", 10).with("to", 5)), Err("range"));
        assert_eq!(ModNote::parse(&Envelope::new(T_HIDE).with("author", pk("c").as_hex()).with("from", 0).with("to", 5)), Err("targets"), "a span is its own type");
        assert_eq!(ModNote::parse(&Envelope::new(T_APPROVE).with("target", id.clone())), Err("version"));
        // The same id twice is one target.
        let twice = ModNote::parse(&Envelope::new(T_UNHIDE).with("targets", vec![id.clone(), id.clone()])).unwrap().unwrap();
        assert_eq!(twice, ModNote::Unhide { targets: vec![id] });
    }

    #[test]
    fn a_mark_is_in_force_by_the_roles_now() {
        use crate::op::OpBody;
        use crate::state::tests::{create, op};
        let (o, m, u, x) = (pk("0"), pk("b"), pk("c"), pk("d"));
        let mut s = ChannelState::genesis(&create(&o)).unwrap();
        let hide = |by: &PubKey| MarkRow { by: by.as_hex().into(), ..mark("n", Kind::Hide, 1, false) };
        let approve = |by: &PubKey| MarkRow { by: by.as_hex().into(), ..mark("n", Kind::Approve, 1, false) };
        assert!(!active(&hide(&m), &s, Some(u.as_hex()), None), "a reader moderates nobody");
        s.apply(&op(&o, OpBody::SetRole { who: m.clone(), role: crate::roles::Role::Moderator })).unwrap();
        assert!(active(&hide(&m), &s, Some(u.as_hex()), None));
        assert!(active(&hide(&m), &s, Some(x.as_hex()), None), "an outsider's message counts as a reader's");
        assert!(active(&hide(&m), &s, None, None), "a message not here yet counts as a reader's");
        assert!(!active(&hide(&m), &s, Some(o.as_hex()), None), "not above the owner");
        assert!(!active(&hide(&m), &s, Some(m.as_hex()), None), "not above oneself");
        assert!(active(&hide(&o), &s, Some(m.as_hex()), None));
        assert!(active(&approve(&m), &s, None, None) && !active(&approve(&u), &s, None, None));
        let span = MarkRow { target_id: String::new(), author: Some(u.as_hex().into()), ..hide(&m) };
        assert!(active(&span, &s, None, None), "a span takes the author it names");
        s.apply(&op(&o, OpBody::SetRole { who: m.clone(), role: crate::roles::Role::Member })).unwrap();
        assert!(!active(&hide(&m), &s, Some(u.as_hex()), None), "demoted: out of force");
        s.apply(&op(&o, OpBody::SetRole { who: m.clone(), role: crate::roles::Role::Moderator })).unwrap();
        s.apply(&op(&o, OpBody::Remove { who: m.clone() })).unwrap();
        assert!(!active(&hide(&m), &s, Some(u.as_hex()), None), "gone: out of force");
    }

    #[test]
    fn the_author_of_a_post_moderates_the_comments_under_it() {
        use crate::op::OpBody;
        use crate::state::tests::{create, op};
        let (o, w, u, m) = (pk("0"), pk("e"), pk("c"), pk("b"));
        let mut s = ChannelState::genesis(&create(&o)).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: w.clone(), role: crate::roles::Role::Author })).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: m.clone(), role: crate::roles::Role::Moderator })).unwrap();
        let hide = MarkRow { by: w.as_hex().into(), ..mark("n", Kind::Hide, 1, false) };
        let unhide = MarkRow { by: w.as_hex().into(), ..mark("n", Kind::Unhide, 1, false) };
        assert!(active(&hide, &s, Some(u.as_hex()), Some(w.as_hex())), "a reader's comment under the author's post");
        assert!(active(&unhide, &s, Some(u.as_hex()), Some(w.as_hex())));
        assert!(!active(&hide, &s, Some(u.as_hex()), Some(o.as_hex())), "under somebody else's post");
        assert!(!active(&hide, &s, Some(u.as_hex()), None), "not a comment");
        assert!(!active(&hide, &s, Some(m.as_hex()), Some(w.as_hex())), "a moderator's comment");
        assert!(!active(&hide, &s, Some(w.as_hex()), Some(w.as_hex())), "not one's own");
        let approve = MarkRow { by: w.as_hex().into(), ..mark("n", Kind::Approve, 1, false) };
        assert!(!active(&approve, &s, Some(u.as_hex()), Some(w.as_hex())), "the queue is the moderators'");
        let span = MarkRow { target_id: String::new(), author: Some(u.as_hex().into()), ..hide.clone() };
        assert!(!active(&span, &s, None, Some(w.as_hex())), "a span reaches past the post");
        s.apply(&op(&m, OpBody::Ban { who: w.clone(), reason: None })).unwrap();
        assert!(!active(&hide, &s, Some(u.as_hex()), Some(w.as_hex())), "banned: out of force");
    }
}

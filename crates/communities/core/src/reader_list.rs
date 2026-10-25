// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The list of the readers of a channel, for its team only (the owner's
//! decision, 2026-10-25): moderators and above see who reads, readers
//! never see each other. The journal does not list readers (wire §3, §8):
//! the device that keeps the channel reads its keeper's rows, every other
//! device of the team the shards of the newest snapshot, the missing ones
//! wanted of the hosts by a job (`Member::known_readers`). Nothing new on
//! the wire.
//!
//! The list leaves out the staff (authors and above, in the team list of
//! the page), the banned and me; a reader the journal names for a mute, a
//! trust or levels stays in it, as a reader. Newest join first; `query`
//! is a start of the hex key (names are the page's).

use crate::roles::Role;
use crate::service::ChannelService;
use crate::state::ChannelState;
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;
use std::collections::HashSet;

/// Readers one page holds at most.
pub const MAX_PAGE: usize = 500;

/// One reader as the team sees them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderLine {
    /// Hex key.
    pub pubkey: String,
    /// The time of their join at the keeper.
    pub since: i64,
    /// The levels the journal gives them, the base level (bit 0) with them.
    pub levels: u64,
}

/// A page of the readers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadersPage {
    pub readers: Vec<ReaderLine>,
    /// How many readers match, on every page.
    pub total: usize,
    /// Some shards of the snapshot are not here yet (asked of the hosts):
    /// the list may lack readers; asked again, it may hold more.
    pub partial: bool,
}

/// The query as a start of a hex key: lower case, trimmed; `None` for one
/// that cannot start any key (then nothing matches).
fn hex_prefix(query: &str) -> Option<String> {
    let q = query.trim().to_ascii_lowercase();
    (q.len() <= 64 && q.bytes().all(|b| b.is_ascii_hexdigit())).then_some(q)
}

/// The readers of `known` that the list shows, newest join first (then by
/// key), the page of `offset` and `limit`, and how many match.
pub fn page_of(state: &ChannelState, me: &PubKey, known: Vec<(String, i64)>, query: Option<&str>, offset: usize, limit: usize) -> (Vec<ReaderLine>, usize) {
    let prefix = match query.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => match hex_prefix(q) {
            Some(p) => p,
            None => return (Vec::new(), 0),
        },
        None => String::new(),
    };
    let mut seen = HashSet::new();
    let mut lines: Vec<ReaderLine> = known
        .into_iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .filter_map(|(k, since)| PubKey::parse(&k).map(|pk| (pk, since)))
        .filter(|(pk, _)| pk != me && !state.is_banned(pk) && !state.role_of(pk).is_some_and(|r| r >= Role::Author))
        .filter(|(pk, _)| seen.insert(pk.as_hex().to_string()))
        .map(|(pk, since)| ReaderLine { levels: state.levels_of(&pk), pubkey: pk.as_hex().to_string(), since })
        .collect();
    lines.sort_by(|a, b| b.since.cmp(&a.since).then_with(|| a.pubkey.cmp(&b.pubkey)));
    let total = lines.len();
    let page = lines.into_iter().skip(offset).take(limit.clamp(1, MAX_PAGE)).collect();
    (page, total)
}

impl ChannelService {
    /// A page of the readers of a channel (module doc). For moderators and
    /// above, refused to anybody else.
    pub async fn readers(&self, keys: &Keys, community_id: &str, query: Option<&str>, offset: usize, limit: usize) -> Result<ReadersPage> {
        let log = self.log(community_id).await?.ok_or_else(|| MessengerError::Invalid("channel: unknown channel".into()))?;
        let me = Self::me(keys)?;
        let state = log.state();
        if state.is_banned(&me) || !state.role_of(&me).is_some_and(Role::is_moderator) {
            return Err(MessengerError::Invalid("channel: not permitted".into()));
        }
        let known = self.member(keys).known_readers(&log).await?;
        let (readers, total) = page_of(state, &me, known.readers, query, offset, limit);
        Ok(ReadersPage { readers, total, partial: known.partial })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::{LevelCan, OpBody};
    use crate::state::tests::{op, pk, team};

    fn hex(c: &str) -> String {
        pk(c).as_hex().to_string()
    }

    /// Owner `0`, admin `a`, moderator `b`, author `e` (`team`); `c`
    /// banned; readers `1`, `2` (with level 2), `3`.
    fn channel() -> ChannelState {
        let mut s = team();
        let o = pk("0");
        s.apply(&op(&o, OpBody::Ban { who: pk("c"), reason: None })).unwrap();
        s.apply(&op(&o, OpBody::LevelSet { id: 2, scopes: vec![], can: LevelCan::BASE })).unwrap();
        s.apply(&op(&o, OpBody::SetLevels { who: pk("2"), levels: 0b101 })).unwrap();
        s
    }

    fn known() -> Vec<(String, i64)> {
        vec![
            (hex("0"), 100),
            (hex("a"), 110),
            (hex("b"), 115),
            (hex("e"), 120),
            (hex("c"), 130),
            (hex("1"), 140),
            (hex("2"), 150),
            (hex("3"), 160),
            (hex("3"), 160),
            ("not a key".to_string(), 170),
        ]
    }

    fn keys(page: &[ReaderLine]) -> Vec<String> {
        page.iter().map(|l| l.pubkey.clone()).collect()
    }

    #[test]
    fn the_list_leaves_out_the_staff_the_banned_and_me_newest_first() {
        let s = channel();
        let (page, total) = page_of(&s, &pk("b"), known(), None, 0, 100);
        assert_eq!(total, 3);
        assert_eq!(keys(&page), [hex("3"), hex("2"), hex("1")]);
        assert_eq!(page[1].levels, 0b101, "the journal's levels, the base one with them");
        assert_eq!((page[0].levels, page[0].since), (1, 160));
    }

    #[test]
    fn the_list_pages_and_finds_a_start_of_a_key() {
        let s = channel();
        let me = pk("a");
        let (page, total) = page_of(&s, &me, known(), None, 1, 1);
        assert_eq!((keys(&page), total), (vec![hex("2")], 3));
        assert!(page_of(&s, &me, known(), None, 3, 10).0.is_empty());
        let (page, total) = page_of(&s, &me, known(), Some(" 111111 "), 0, 100);
        assert_eq!((keys(&page), total), (vec![hex("1")], 1));
        assert_eq!(page_of(&s, &me, known(), Some("33333"), 0, 100).1, 1);
        assert_eq!(page_of(&s, &me, known(), Some("ABCDEF"), 0, 100).1, 0, "upper case reads as lower: no such key");
        assert_eq!(page_of(&s, &me, known(), Some("alice"), 0, 100).1, 0, "words match nothing");
        assert_eq!(page_of(&s, &me, known(), Some("  "), 0, 100).1, 3, "a blank query is none");
    }

    #[test]
    fn the_same_second_is_ordered_by_key() {
        let s = channel();
        let (page, _) = page_of(&s, &pk("0"), vec![(hex("5"), 5), (hex("4"), 5)], None, 0, 10);
        assert_eq!(keys(&page), [hex("4"), hex("5")]);
    }
}

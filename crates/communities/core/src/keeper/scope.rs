// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper of one scope, whatever its scheme (internal/communities-wire.md
//! §14.2): keydist's `TreeKeeper<SdScheme>` for `sd-v1` (`everyone` and
//! the sections of readers), its `FlatKeeper` for `flat-v1` (small closed
//! sections, the staff). The cycle asks both the same things; where their
//! states differ (a tree saved row by row, a flat state saved whole) this
//! is the one place that knows.

use keydist::{Change, FlatKeeper, KeyDistribution, Policy, SchemeId, SdKeeper};

/// keydist's keeper of one scope. At most 16 a channel, held for the life
/// of the keeper and never moved around: boxing the larger would buy
/// nothing.
#[allow(clippy::large_enum_variant)]
pub enum ScopeKeeper {
    Sd(SdKeeper),
    Flat(FlatKeeper),
}

impl ScopeKeeper {
    pub fn kd(&self) -> &dyn KeyDistribution {
        match self {
            Self::Sd(k) => k,
            Self::Flat(k) => k,
        }
    }

    pub fn kd_mut(&mut self) -> &mut dyn KeyDistribution {
        match self {
            Self::Sd(k) => k,
            Self::Flat(k) => k,
        }
    }

    pub fn scheme(&self) -> SchemeId {
        self.kd().scheme()
    }

    /// The tree keeper, for what only a tree has (generations, rows).
    pub fn sd(&self) -> Option<&SdKeeper> {
        match self {
            Self::Sd(k) => Some(k),
            Self::Flat(_) => None,
        }
    }

    pub fn members_len(&self) -> usize {
        match self {
            Self::Sd(k) => k.state().members.len(),
            Self::Flat(k) => k.state().members.len(),
        }
    }

    /// The members, by the 32 bytes of their keys.
    pub fn members(&self) -> Vec<keydist::MemberId> {
        match self {
            Self::Sd(k) => k.state().members.keys().copied().collect(),
            Self::Flat(k) => k.state().members.keys().copied().collect(),
        }
    }

    /// When the scope wants the next turn. keydist's `next_wake` says
    /// "now" (the last epoch's time) while a planned migration is due by
    /// the policy's thresholds, and the engine starts none yet: that "now"
    /// would come back after every turn, and the loop of the module would
    /// turn at full speed. Then only the times of its epochs count.
    pub fn next_wake(&self) -> Option<u64> {
        match self {
            Self::Sd(k) if k.migration_due(&k.state().policy.thresholds).is_some() => {
                let st = k.state();
                let due = st.schedule.due(st.last_epoch_at, st.policy.min_epoch_interval_secs);
                // The first epoch of a scope with members, as keydist wakes for it.
                let first = (st.head.is_none() && !st.members.is_empty()).then(|| st.last_epoch_at.unwrap_or(k.generation().started_at));
                match (due, first) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            }
            _ => self.kd().next_wake(),
        }
    }

    /// When the queued changes want an epoch.
    pub fn due_at(&self) -> Option<u64> {
        match self {
            Self::Sd(k) => k.state().schedule.due_at,
            Self::Flat(k) => k.state().schedule.due_at,
        }
    }

    pub fn published_head(&self) -> Option<&keydist::EpochKey> {
        match self {
            Self::Sd(k) => k.state().published_head.as_ref(),
            Self::Flat(k) => k.state().published_head.as_ref(),
        }
    }

    pub fn last_epoch_at(&self) -> Option<u64> {
        match self {
            Self::Sd(k) => k.state().last_epoch_at,
            Self::Flat(k) => k.state().last_epoch_at,
        }
    }

    pub fn pending_epoch(&self) -> Option<u32> {
        self.kd().pending().map(|p| p.key.epoch)
    }

    /// The blobs of the newest published epoch (a flat scope has none).
    pub fn published_blobs(&self) -> Vec<[u8; 32]> {
        match self {
            Self::Sd(k) => k.state().published_blobs.clone(),
            Self::Flat(_) => Vec::new(),
        }
    }

    pub fn generations(&self) -> usize {
        match self {
            Self::Sd(k) => k.state().gens.len(),
            Self::Flat(_) => 1,
        }
    }

    pub fn policy(&self) -> &Policy {
        match self {
            Self::Sd(k) => &k.state().policy,
            Self::Flat(k) => &k.state().policy,
        }
    }

    /// The tree's change log since the last call (a flat state is saved
    /// whole, `flat_json`).
    pub fn take_changes(&mut self) -> Vec<Change> {
        match self {
            Self::Sd(k) => k.take_changes(),
            Self::Flat(_) => Vec::new(),
        }
    }

    /// The whole state of a flat scope, to save.
    pub fn flat_json(&self) -> Option<serde_json::Result<String>> {
        match self {
            Self::Sd(_) => None,
            Self::Flat(k) => Some(serde_json::to_string(k.state())),
        }
    }

    /// The same keeper numbering its next epoch from `from` at least: a
    /// keeper of a new series publishes only from the epoch its `keeper`
    /// operation named (wire §15), whatever the former one reached.
    pub fn with_next_epoch(self, from: u32) -> Result<Self, keydist::Error> {
        match self {
            Self::Sd(k) => {
                let mut st = k.into_state();
                st.next_epoch = st.next_epoch.max(from);
                SdKeeper::from_state(st).map(Self::Sd)
            }
            Self::Flat(k) => {
                let mut st = k.state().clone();
                st.next_epoch = st.next_epoch.max(from);
                FlatKeeper::from_state(st).map(Self::Flat)
            }
        }
    }

    /// The same keeper under another policy (`open_scope`, `history`):
    /// keydist keeps its policy in its state, so it is put together again.
    pub fn with_policy(self, want: &Policy) -> Result<Self, keydist::Error> {
        match self {
            Self::Sd(k) => {
                let mut st = k.into_state();
                st.policy.open_scope = want.open_scope;
                st.policy.history = want.history;
                SdKeeper::from_state(st).map(Self::Sd)
            }
            Self::Flat(k) => {
                let mut st = k.state().clone();
                st.policy.open_scope = want.open_scope;
                st.policy.history = want.history;
                FlatKeeper::from_state(st).map(Self::Flat)
            }
        }
    }
}

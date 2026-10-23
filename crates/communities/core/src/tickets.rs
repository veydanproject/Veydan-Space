// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The tickets of the moderators on the moderator's and the newcomer's
//! side (internal/communities-wire.md §17, internal/communities-spec.md
//! §8.6): a moderator asks the keeper for a batch, keeps them, and when he
//! admits somebody while the keeper is away hands one over with the
//! admission; the newcomer reads at once by it.

use crate::db::{self, tickets::HeldRow};
use crate::keeper::KEEPER_AWAY_SECS;
use crate::notes::Opened;
use crate::op::JoinMode;
use crate::roles::{permits, Action, Role};
use crate::service::{ChannelService, Outcome, MEMBERSHIP_JOINED, UI_EVENT_CHANNEL_UPDATED};
use crate::state::ChannelState;
use crate::wire::{self, Admit, Issue, TicketRequest, TicketUse, Tickets, MAX_TICKETS};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel: {what}"))
}

/// What a moderator holds of the tickets of a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicketsHeld {
    /// Tickets good for at least a while yet.
    pub held: u32,
    pub max: u32,
}

impl ChannelService {
    /// A moderator asks the keeper for `n` tickets (`request` and `invite`
    /// channels: in an open one no moderator lets people in).
    pub async fn tickets_request(&self, keys: &Keys, community_id: &str, n: u32) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let state = self.state(community_id).await?;
        if !state.role_of(&me).is_some_and(|r| permits(r, Action::Admit)) || state.is_banned(&me) {
            return Err(invalid("not permitted"));
        }
        if state.join == JoinMode::Open {
            return Err(invalid("no tickets in an open channel"));
        }
        let body = TicketRequest { community_id: community_id.to_string(), n: n.clamp(1, MAX_TICKETS) };
        let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?.inbox().clone();
        self.send_note(Some(&state), community_id, &keeper, wire::T_TICKET_REQUEST, &body, None).await?;
        Ok(Outcome::default())
    }

    /// The tickets I hold of a channel.
    pub async fn tickets_held(&self, community_id: &str) -> Result<TicketsHeld> {
        let now = self.now();
        db::tickets::drop_expired_held(&self.env.db, community_id, now).await?;
        let held = db::tickets::held_count(&self.env.db, community_id, now + wire::SKEW_SECS).await?;
        Ok(TicketsHeld { held: u32::try_from(held).unwrap_or(u32::MAX), max: MAX_TICKETS })
    }

    /// `channel.ticket`: from the keeper of the current series, to me as a
    /// moderator.
    pub(crate) async fn on_tickets(&self, keys: &Keys, n: &Opened, body: &Tickets) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(log) = self.log(&body.community_id).await? else { return Ok(out) };
        let state = log.state();
        let me = Self::me(keys)?;
        let ok = body.check().is_ok()
            && state.keeper().is_some_and(|k| k.pubkey == n.sender)
            && state.role_of(&me).is_some_and(|r| permits(r, Action::Admit));
        if !ok {
            out.note("tickets not from the keeper, or not for a moderator");
            return Ok(out);
        }
        let now = self.now();
        for t in &body.tickets {
            let row = HeldRow {
                community_id: body.community_id.clone(),
                ticket_id: t.ticket_id.clone(),
                issued: t.issued.clone(),
                expires_at: t.expires_at.min(now + wire::TICKET_TTL_SECS),
                status: db::tickets::HELD.into(),
                used_by: None,
                received_at: now,
            };
            db::tickets::put_held(&self.env.db, &row).await?;
        }
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": body.community_id, "tickets": true }));
        Ok(out)
    }

    /// When the keeper of a channel was last heard of here: its newest
    /// epoch, snapshot or sign of life (wire §18): a keeper of a quiet
    /// channel makes no epoch for hours and would look away, and the
    /// tickets would be handed almost always.
    pub(crate) async fn keeper_heard_at(&self, community_id: &str) -> Result<Option<i64>> {
        let epoch = db::keys::last_epoch_at(&self.env.db, community_id).await?;
        let snapshot = db::snapshots::latest(&self.env.db, community_id).await?.map(|(at, _)| at);
        let alive = db::channels::keeper_seen(&self.env.db, community_id).await?;
        Ok(epoch.max(snapshot).max(alive))
    }

    /// A ticket for the newcomer of an admission, when the keeper is away
    /// (not kept here, not heard of for `KEEPER_AWAY_SECS`) and I hold one:
    /// handed to him at once, its id named in the admission.
    pub(crate) async fn ticket_for(&self, state: &ChannelState, admit: &mut Admit) -> Result<()> {
        let cid = &state.community_id;
        if state.join == JoinMode::Open || self.keeper(cid).await?.is_some() {
            return Ok(());
        }
        let now = self.now();
        if self.keeper_heard_at(cid).await?.is_some_and(|at| now - at < KEEPER_AWAY_SECS) {
            return Ok(());
        }
        let Some(t) = db::tickets::use_one(&self.env.db, cid, now + wire::SKEW_SECS, admit.who.as_hex()).await? else { return Ok(()) };
        let body = TicketUse { community_id: cid.clone(), ticket_id: t.ticket_id.clone(), issued: t.issued };
        self.send_note(None, cid, &admit.who, wire::T_TICKET_USE, &body, None).await?;
        admit.ticket_id = Some(t.ticket_id);
        tracing::info!(community = %cid, who = %admit.who.short(), "a ticket handed to a newcomer: the keeper is away");
        Ok(())
    }

    /// `channel.ticket_use`: a moderator by the journal handed me a ticket;
    /// I read at once, the keeper's own issue comes later.
    pub(crate) async fn on_ticket_use(&self, keys: &Keys, n: &Opened, body: &TicketUse) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(log) = self.log(&body.community_id).await? else {
            out.note("a ticket before the journal");
            return Ok(out);
        };
        let state = log.state();
        let moderator = state.role_of(&n.sender).is_some_and(Role::is_moderator) && !state.is_banned(&n.sender);
        let me: PubKey = Self::me(keys)?;
        if !moderator || state.is_banned(&me) {
            out.note("a ticket not from a moderator");
            return Ok(out);
        }
        let issued = Issue { community_id: body.community_id.clone(), relays: vec![], storage: vec![], heads: vec![], link: None, issued: body.issued.clone() }
            .issued()
            .map_err(|e| invalid(&format!("ticket: {e}")))?;
        let m = self.member(keys).take_ticket(state, &issued, self.now()).await?;
        self.refresh_row(state, &me, Some(MEMBERSHIP_JOINED)).await?;
        self.read_pending(keys, m.pending, &mut out).await?;
        out.resubscribe = true;
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": body.community_id, "membership": MEMBERSHIP_JOINED }));
        Ok(out)
    }
}

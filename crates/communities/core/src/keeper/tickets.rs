// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The tickets of the moderators on the keeper's side (see
//! internal/communities-wire.md §17, internal/communities-spec.md §8.6).
//!
//! A ticket is a leaf of `everyone` the keeper lets in ahead of time for a
//! newcomer not named yet: keydist's `join` of a member id made from the
//! ticket's id, its `Issued` sealed to the moderator who asked. While the
//! keeper is away the moderator hands one to the person he admits, who
//! reads at once; the moderator's `channel.admit` names the ticket, and the
//! keeper, when it is back, lets the person in on a leaf of his own and puts
//! the ticket's leaf out. A ticket not used in a day is put out too, and so
//! are those of a moderator who is one no longer.

use super::{kd, secs, Keeper};
use crate::db::tickets as rows;
use crate::log::OpLog;
use crate::op::JoinMode;
use crate::roles::{permits, Action};
use crate::state::SCOPE_EVERYONE;
use crate::wire::{self, Admit, Issue, Ticket, TicketRequest, Tickets, MAX_TICKETS, TICKETS_PER_NOTE, TICKET_TTL_SECS};
use keydist::{MemberId, Revoke};
use messenger_core::{PubKey, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// The keydist member a ticket holds its leaf under: no person's key (the
/// hash of the ticket's id within its channel), so nobody can claim it.
pub fn ticket_member(community_id: &str, ticket_id: &str) -> MemberId {
    let mut h = Sha256::new();
    h.update(b"veydan channel ticket");
    h.update(community_id.as_bytes());
    h.update(ticket_id.as_bytes());
    MemberId(h.finalize().into())
}

impl Keeper {
    /// The leaves the open tickets hold: not readers, left out of the
    /// snapshot and of every bundle handed round.
    pub(crate) async fn ticket_members(&mut self) -> Result<BTreeSet<MemberId>> {
        let open = rows::open(&self.env.db, &self.community_id, None).await?;
        Ok(open.iter().map(|t| ticket_member(&self.community_id, &t.ticket_id)).collect())
    }

    /// `channel.ticket_request`: tickets for a moderator by the journal, in a
    /// channel a moderator lets people into (`request`, `invite`), up to
    /// `MAX_TICKETS` open at once. How many were minted.
    pub async fn on_ticket_request(&mut self, log: &OpLog, from: &PubKey, req: &TicketRequest, now: i64) -> Result<u32> {
        let state = log.state();
        if req.community_id != self.community_id || req.check().is_err() {
            return Ok(0);
        }
        let moderator = !state.is_banned(from) && state.role_of(from).is_some_and(|r| permits(r, Action::Admit));
        if !moderator || state.join == JoinMode::Open || state.disbanded {
            tracing::info!(community = %self.community_id, by = %from.short(), "keeper refused tickets: no moderator, or an open channel");
            return Ok(0);
        }
        let open = rows::open(&self.env.db, &self.community_id, Some(from.as_hex())).await?;
        let room = MAX_TICKETS.saturating_sub(u32::try_from(open.len()).unwrap_or(u32::MAX));
        let n = req.n.min(room);
        let mut minted = Vec::new();
        for _ in 0..n {
            let mut raw = [0u8; 16];
            getrandom::fill(&mut raw).map_err(|e| messenger_core::MessengerError::Crypto(e.to_string()))?;
            let ticket_id = hex::encode(raw);
            let member = ticket_member(&self.community_id, &ticket_id);
            let issued = self.sk(SCOPE_EVERYONE)?.kd_mut().join(member, secs(now)).map_err(kd)?;
            self.save(SCOPE_EVERYONE).await?;
            let expires_at = now + TICKET_TTL_SECS;
            rows::put_minted(
                &self.env.db,
                &rows::MintedRow {
                    community_id: self.community_id.clone(),
                    ticket_id: ticket_id.clone(),
                    moderator: from.as_hex().to_string(),
                    member: hex::encode(member.0),
                    expires_at,
                    status: rows::MINTED.into(),
                    used_by: None,
                    created_at: now,
                },
            )
            .await?;
            minted.push(Ticket { ticket_id, issued: Issue::encode_issued(&issued), expires_at });
        }
        for chunk in minted.chunks(TICKETS_PER_NOTE) {
            let body = Tickets { community_id: self.community_id.clone(), tickets: chunk.to_vec() };
            self.send_note(Some(state), from, wire::T_TICKET, &body, now).await?;
        }
        tracing::info!(community = %self.community_id, by = %from.short(), n, "keeper minted tickets");
        Ok(n)
    }

    /// An admission that names a ticket (§17): the ticket used by its
    /// moderator for this newcomer, its leaf out; the newcomer comes in on a
    /// leaf of his own by his join.
    pub(crate) async fn ticket_admitted(&mut self, admit: &Admit, from: &PubKey, now: i64) -> Result<()> {
        let Some(ticket_id) = &admit.ticket_id else { return Ok(()) };
        let Some(t) = rows::minted(&self.env.db, &self.community_id, ticket_id).await? else { return Ok(()) };
        if t.status != rows::MINTED || t.moderator != from.as_hex() {
            tracing::info!(community = %self.community_id, "an admission names a ticket not open, or of another moderator");
            return Ok(());
        }
        rows::set_minted_status(&self.env.db, &self.community_id, ticket_id, rows::USED, Some(admit.who.as_hex())).await?;
        self.ticket_out(&t.ticket_id, Revoke::Leave, now).await
    }

    /// Puts the leaf of a ticket out of `everyone`.
    async fn ticket_out(&mut self, ticket_id: &str, why: Revoke, now: i64) -> Result<()> {
        let m = ticket_member(&self.community_id, ticket_id);
        let k = self.sk(SCOPE_EVERYONE)?;
        if k.kd().is_member(&m) {
            k.kd_mut().revoke(&[m], why, secs(now));
            self.save(SCOPE_EVERYONE).await?;
        }
        Ok(())
    }

    /// The tickets past their day, and those of somebody who is no
    /// moderator any more: their leaves out at the next epoch.
    pub(crate) async fn retire_tickets(&mut self, log: &OpLog, now: i64) -> Result<()> {
        let state = log.state();
        for t in rows::open(&self.env.db, &self.community_id, None).await? {
            let moderator = PubKey::parse(&t.moderator).is_some_and(|m| !state.is_banned(&m) && state.role_of(&m).is_some_and(|r| permits(r, Action::Admit)));
            if t.expires_at > now && moderator {
                continue;
            }
            rows::set_minted_status(&self.env.db, &self.community_id, &t.ticket_id, rows::REVOKED, None).await?;
            self.ticket_out(&t.ticket_id, Revoke::Ban, now).await?;
            tracing::info!(community = %self.community_id, expired = t.expires_at <= now, "keeper put a ticket out");
        }
        Ok(())
    }
}

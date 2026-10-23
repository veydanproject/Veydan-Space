// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A channel handed to a new owner (internal/communities-wire.md §16,
//! internal/communities-spec.md §7.8, the gate of wire §3).
//!
//! The owner names the new one by `transfer_ownership` in the journal and
//! offers the channel by `channel.transfer_offer`; nothing changes until the
//! new owner accepts: a new root of his own, a keeper key, the `keeper`
//! operation of the next series — the one operation of somebody who is not
//! the owner yet, which puts the handover in force (the former owner is an
//! admin from it on). The former owner's keeper hands its state over to the
//! new one's inbox (`hand_over`, wire §15); the new keeper starts a planned
//! generation per scope from the new root with a grace of 7 days and hands
//! every reader a bundle of it; the card is the new owner's. Declined: a
//! note back, the journal untouched.

use crate::db::{self, handover::TakeRow, transfers::TransferRow};
use crate::notes::Opened;
use crate::op::OpBody;
use crate::secrets;
use crate::service::{ChannelService, Outcome, UI_EVENT_CHANNEL_UPDATED};
use crate::wire::{self, TransferDecline, TransferOffer};
use keydist::RootSecret;
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;

/// The grace of the old owner's generation after a handover (spec §7.8,
/// decision of the owner): a week for every reader to take a bundle of the
/// new one.
pub const TRANSFER_GRACE_SECS: u64 = 7 * 24 * 3600;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel: {what}"))
}

/// The handover of a channel as a page shows it: mine to answer (`in`) or
/// made by me (`out`), and where it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub direction: String,
    pub peer: String,
    pub status: String,
    pub created_at: i64,
}

impl ChannelService {
    /// «Передать канал» (the owner): `transfer_ownership` in the journal and
    /// the offer to the new owner.
    pub async fn transfer(&self, keys: &Keys, community_id: &str, to: &PubKey) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let state = self.state(community_id).await?;
        if state.owner != me {
            return Err(invalid("not permitted"));
        }
        if to == &me {
            return Err(invalid("transfer to oneself"));
        }
        let mut out = self.act(keys, community_id, OpBody::TransferOwnership { to: to.clone() }).await?;
        let now = self.now();
        let offer = TransferOffer {
            community_id: community_id.to_string(),
            name: state.name.clone(),
            about: state.about.clone(),
            picture: state.picture.clone(),
            relay: state.relays.first().cloned(),
            created_at: now,
            expires_at: now + wire::INVITE_TTL_SECS,
        };
        db::transfers::put(
            &self.env.db,
            &TransferRow {
                community_id: community_id.to_string(),
                direction: db::transfers::OUT.into(),
                peer: to.as_hex().to_string(),
                status: db::transfers::OFFERED.into(),
                payload_json: serde_json::to_string(&offer)?,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
        // To the person, not the channel's relays: he may follow none of them.
        self.send_note(None, community_id, to, wire::T_TRANSFER_OFFER, &offer, None).await?;
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id, "transfer": true }));
        Ok(out)
    }

    /// `channel.transfer_offer`: kept when it is the owner's by the journal
    /// (or no journal is here yet: the answer checks it again).
    pub(crate) async fn on_transfer_offer(&self, keys: &Keys, n: &Opened, body: &TransferOffer) -> Result<Outcome> {
        let mut out = Outcome::default();
        let now = self.now();
        if body.check(now).is_err() {
            out.note("a transfer offer out of shape");
            return Ok(out);
        }
        let me = Self::me(keys)?;
        if let Some(log) = self.log(&body.community_id).await? {
            let s = log.state();
            if s.owner != n.sender || s.owner == me {
                out.note("a transfer offer not from the owner");
                return Ok(out);
            }
        }
        let row = TransferRow {
            community_id: body.community_id.clone(),
            direction: db::transfers::IN.into(),
            peer: n.sender.as_hex().to_string(),
            status: db::transfers::OFFERED.into(),
            payload_json: serde_json::to_string(body)?,
            created_at: n.created_at,
            updated_at: now,
        };
        db::transfers::put(&self.env.db, &row).await?;
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": body.community_id, "transfer": true }));
        Ok(out)
    }

    /// `channel.transfer_decline`: from the one I offered the channel to.
    pub(crate) async fn on_transfer_decline(&self, n: &Opened, body: &TransferDecline) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(row) = db::transfers::get(&self.env.db, &body.community_id, db::transfers::OUT).await? else { return Ok(out) };
        if row.peer != n.sender.as_hex() || row.status != db::transfers::OFFERED {
            return Ok(out);
        }
        db::transfers::set_status(&self.env.db, &body.community_id, db::transfers::OUT, db::transfers::DECLINED, self.now()).await?;
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": body.community_id, "transfer": true }));
        Ok(out)
    }

    /// The answer to an offer: accepted — a new root, a keeper key, the
    /// `keeper` operation of the next series (in force by the gate of wire
    /// §3), the card mine; the state of the old keeper comes after.
    /// Declined — a note back, nothing else.
    pub async fn transfer_answer(&self, keys: &Keys, community_id: &str, accept: bool) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let row = db::transfers::get(&self.env.db, community_id, db::transfers::IN).await?.ok_or_else(|| invalid("no transfer offer"))?;
        if row.status != db::transfers::OFFERED {
            return Err(invalid("no transfer offer"));
        }
        let old = PubKey::parse(&row.peer).ok_or_else(|| invalid("no transfer offer"))?;
        let now = self.now();
        let mut out = Outcome::default();
        if !accept {
            self.send_note(None, community_id, &old, wire::T_TRANSFER_DECLINE, &TransferDecline { community_id: community_id.to_string() }, None).await?;
            db::transfers::set_status(&self.env.db, community_id, db::transfers::IN, db::transfers::DECLINED, now).await?;
            out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id, "transfer": true }));
            return Ok(out);
        }
        let state = self.state(community_id).await?;
        if state.pending_owner.as_ref() != Some(&me) || state.owner != old {
            return Err(invalid("the journal of the channel is not here yet"));
        }
        let last = state.keeper().ok_or_else(|| invalid("no keeper"))?;
        let serial = last.serial.checked_add(1).ok_or_else(|| invalid("keeper"))?;
        let newest = db::keys::newest_epoch(&self.env.db, community_id).await?;
        let from_epoch = newest.saturating_add(2).max(last.from_epoch.saturating_add(1));
        let kk = Keys::generate();
        let pubkey = Self::me(&kk)?;
        secrets::put_keeper_keys(self.env.secrets.as_ref(), community_id, &kk).await?;
        // A root of my own: the old owner's never leaves his devices.
        secrets::put_root_of(self.env.secrets.as_ref(), community_id, &RootSecret::generate(), serial).await?;
        db::handover::put_take(
            &self.env.db,
            &TakeRow {
                community_id: community_id.to_string(),
                keeper_pubkey: pubkey.as_hex().to_string(),
                serial: i64::from(serial),
                from_epoch: i64::from(from_epoch),
                mode: db::handover::MODE_TRANSFER.into(),
                started_at: now,
            },
        )
        .await?;
        match self.act(keys, community_id, OpBody::Keeper { pubkey, serial, from_epoch, inbox: Some(me.clone()) }).await {
            Ok(o) => out.merge(o),
            Err(e) => {
                db::handover::drop_take(&self.env.db, community_id).await?;
                secrets::forget_keeper_keys(self.env.secrets.as_ref(), community_id).await?;
                secrets::forget_root(self.env.secrets.as_ref(), community_id).await?;
                return Err(e);
            }
        }
        db::transfers::set_status(&self.env.db, community_id, db::transfers::IN, db::transfers::ACCEPTED, now).await?;
        // The card is the owner's: mine from now on.
        let state = self.state(community_id).await?;
        let card = self.card_of(&state, now);
        self.enqueue(community_id, db::outbox::LANE_REPLACEABLE, wire::KIND_CARD, &wire::sign_card(keys, &card)?, None).await?;
        if let Err(e) = self.member(keys).send_own_keys(&state, now).await {
            tracing::info!(community = %community_id, error = %e, "the root did not reach my other devices yet");
        }
        tracing::info!(community = %community_id, serial, from_epoch, "the channel is mine: its keeper comes from the former owner");
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id, "transfer": true }));
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id, "keeper": "taking" }));
        Ok(out)
    }

    /// The handover of a channel as this device knows it: an offer to me
    /// not answered, or mine (accepted once the journal says so).
    pub async fn transfer_of(&self, community_id: &str) -> Result<Option<Transfer>> {
        let owner = self.log(community_id).await?.map(|l| l.state().owner.as_hex().to_string());
        for dir in [db::transfers::IN, db::transfers::OUT] {
            let Some(row) = db::transfers::get(&self.env.db, community_id, dir).await? else { continue };
            let status = if owner.as_deref() == Some(row.peer.as_str()) && dir == db::transfers::OUT { db::transfers::ACCEPTED.to_string() } else { row.status.clone() };
            // An offer to me answered, or one of mine taken: nothing to show.
            if status == db::transfers::ACCEPTED || (dir == db::transfers::IN && status != db::transfers::OFFERED) {
                continue;
            }
            return Ok(Some(Transfer { direction: dir.to_string(), peer: row.peer, status, created_at: row.created_at }));
        }
        Ok(None)
    }
}

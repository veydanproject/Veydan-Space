// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The sender of what the commands and the keepers left in the outbox
//! (`db::outbox`): a command stores and queues, answers at once, and this
//! sends off every lock, in the order the wire needs, again after a
//! failure (живой запуск 2026-10-23: a channel, a section, a post «hung»
//! on the relay's answer and on a keeper's turn, under the channel's lock).
//!
//! `flush_outbox` is one pass over the rows due by the service's clock:
//! the channel lane of a channel strictly in order, stopped at the first
//! row the relays refuse (the content after an operation relies on it,
//! wire §4); the replaceable rows on their own; a note once the channel
//! rows it follows are gone (`after_seq`), wrapped at this moment with the
//! keys of its signer. Whoever runs it (the module's outbox task, the
//! testkit's pump) waits on `ChannelService::outbox_notify` and for
//! `Flushed::wake` between passes.

use crate::db::{self, outbox::OutboxRow};
use crate::keeper::{OWED_FIRST_WAIT_SECS, OWED_MAX_WAIT_SECS};
use crate::secrets;
use crate::service::{sent_line, short, ChannelService};
use crate::wire;
use messenger_core::outbound::WireEvent;
use messenger_core::traits::UiEvent;
use messenger_core::{EventId, Outbound, PubKey, RelayUrl, Result, Scope};
use nostr::key::Keys;
use std::collections::{HashMap, HashSet};

/// A row of the outbox changed: `{community_id, ref_id, state, error?}`,
/// `state` one of `queued` | `failed` | `sent`.
pub const UI_EVENT_CHANNEL_OUTBOX: &str = "channel.outbox";

/// From this many attempts a row is labelled `failed` for the page; it is
/// still sent again.
pub const FAILED_AFTER_ATTEMPTS: i64 = 6;

/// What a pass of the worker did.
#[derive(Debug, Default)]
pub struct Flushed {
    /// When a row that waits is due (unix seconds); none: nothing waits.
    pub wake: Option<u64>,
    /// Rows that went out.
    pub sent: usize,
    pub ui: Vec<UiEvent>,
    /// Channels whose journal operation, epoch or snapshot went out: the
    /// keeper's turn is due (its cycle waits for the operations to be on
    /// the relay, `tick_one`).
    pub woke: Vec<String>,
}

fn state_event(row: &OutboxRow, state: &str, error: Option<&str>) -> Option<UiEvent> {
    let ref_id = row.ref_id.as_ref()?;
    let mut payload = serde_json::json!({ "community_id": row.community_id, "ref_id": ref_id, "state": state });
    if let Some(e) = error {
        payload["error"] = serde_json::Value::String(e.to_string());
    }
    Some(UiEvent { name: UI_EVENT_CHANNEL_OUTBOX.into(), payload })
}

/// Why no relay took an event.
fn refusal(ack: &messenger_core::Ack) -> String {
    if ack.rejected_by.is_empty() {
        return "no relay answered".into();
    }
    ack.rejected_by.iter().map(|(r, why)| format!("{}: {why}", r.as_str())).collect::<Vec<_>>().join(", ")
}

impl ChannelService {
    /// One pass over the rows due now. Never two at once.
    pub async fn flush_outbox(&self) -> Result<Flushed> {
        let _one = self.flushing.lock().await;
        let now = self.now();
        let db = &self.env.db;
        let mut out = Flushed::default();
        let dropped = db::outbox::drop_notes_before(db, now - wire::NOTE_MAX_AGE_SECS).await?;
        if dropped > 0 {
            self.say(&format!("outbox: {dropped} note(s) older than a week given up"));
        }
        // Channels whose channel lane stopped in this pass (a row refused,
        // or an earlier one that waits for its time).
        let mut held: HashSet<String> = HashSet::new();
        let mut keeper_keys: HashMap<String, Option<Keys>> = HashMap::new();
        for row in db::outbox::due(db, now).await? {
            let cid = row.community_id.clone();
            let sent = match row.lane.as_str() {
                db::outbox::LANE_CHANNEL => {
                    if held.contains(&cid) {
                        continue;
                    }
                    if db::outbox::channel_head(db, &cid).await? != Some(row.seq) {
                        held.insert(cid);
                        continue;
                    }
                    let sent = self.send_scoped(&row).await;
                    if sent.is_err() {
                        held.insert(cid);
                    }
                    sent
                }
                db::outbox::LANE_REPLACEABLE | db::outbox::LANE_KEEPER => self.send_scoped(&row).await,
                db::outbox::LANE_INBOX => {
                    if db::outbox::channel_rows_up_to(db, &cid, row.after_seq).await? > 0 {
                        continue;
                    }
                    let keys = match row.signer.as_str() {
                        db::outbox::SIGNER_KEEPER => match keeper_keys.get(&cid) {
                            Some(k) => k.clone(),
                            None => {
                                let k = secrets::keeper_keys(self.env.secrets.as_ref(), &cid).await.ok().flatten();
                                keeper_keys.insert(cid.clone(), k.clone());
                                k
                            }
                        },
                        _ => self.signer(),
                    };
                    match keys {
                        Some(keys) => self.send_note_row(&keys, &row, now).await,
                        // The keeper's keys are gone from the store: the note
                        // is nobody's to wrap. Mine: no identity at work yet,
                        // the row waits for it.
                        None if row.signer == db::outbox::SIGNER_KEEPER => Err("the keeper's keys are not in the secret store".into()),
                        None => continue,
                    }
                }
                other => Err(format!("unknown lane {other}")),
            };
            match sent {
                Ok(()) => self.row_sent(&row, &mut out).await?,
                Err(why) => self.row_failed(&row, &why, now, &mut out).await?,
            }
        }
        out.wake = self.next_due(now).await?;
        Ok(out)
    }

    /// A signed event of a channel to its relays.
    async fn send_scoped(&self, row: &OutboxRow) -> std::result::Result<(), String> {
        let json: serde_json::Value = serde_json::from_str(&row.payload).map_err(|e| format!("a row that does not read: {e}"))?;
        let id = EventId::parse(&row.event_id).ok_or_else(|| "a row without an event id".to_string())?;
        let out = Outbound::PublishScoped { scope: Scope::Channel { id: row.community_id.clone() }, event: WireEvent { id, json } };
        match self.env.net.send(out).await {
            Ok(ack) if ack.is_delivered() => Ok(()),
            Ok(ack) => Err(refusal(&ack)),
            Err(e) => Err(e.to_string()),
        }
    }

    /// A note wrapped anew (a new date) with `keys`: to its recipient's
    /// inbox, with the relays hinted; without a recipient, to my own.
    async fn send_note_row(&self, keys: &Keys, row: &OutboxRow, now: i64) -> std::result::Result<(), String> {
        let hint_relays: Vec<RelayUrl> = row.hint_json.as_deref().and_then(|h| serde_json::from_str::<Vec<String>>(h).ok()).unwrap_or_default().iter().filter_map(|r| RelayUrl::parse(r)).collect();
        let (to, out) = match row.recipient.as_deref().and_then(PubKey::parse) {
            Some(to) => {
                let event = crate::notes::wrap_note(keys, &to, &row.payload, now).map_err(|e| e.to_string())?;
                (Some(to.clone()), Outbound::PublishToInbox { recipient: to, event, hint_relays })
            }
            None => {
                let event = crate::notes::wrap_own(keys, &row.payload, now).map_err(|e| e.to_string())?;
                (None, Outbound::PublishOwn { event })
            }
        };
        let t = serde_json::from_str::<serde_json::Value>(&row.payload).ok().and_then(|v| v["t"].as_str().map(str::to_string)).unwrap_or_else(|| "a note".into());
        match self.env.net.send(out).await {
            Ok(ack) if ack.is_delivered() => {
                if let Some(to) = &to {
                    self.say(&sent_line(&t, &row.community_id, to, &ack));
                }
                Ok(())
            }
            Ok(ack) => Err(refusal(&ack)),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn row_sent(&self, row: &OutboxRow, out: &mut Flushed) -> Result<()> {
        db::outbox::sent(&self.env.db, row.seq).await?;
        out.sent += 1;
        let kind = row.kind.and_then(|k| u16::try_from(k).ok());
        if matches!(kind, Some(wire::KIND_JOURNAL | wire::KIND_EPOCH | wire::KIND_SNAPSHOT)) && !out.woke.contains(&row.community_id) {
            out.woke.push(row.community_id.clone());
        }
        // The keeper's sign of life counts from when a relay took it: the
        // owner's own page reads it as every member does (wire §18).
        if kind == Some(wire::KIND_HEARTBEAT) {
            let hb = serde_json::from_str::<serde_json::Value>(&row.payload).ok().and_then(|v| serde_json::from_str::<crate::heartbeat::Heartbeat>(v["content"].as_str()?).ok());
            if let Some(hb) = hb {
                if let Err(e) = db::channels::put_keeper_seen(&self.env.db, &row.community_id, hb.at, hb.serial).await {
                    tracing::debug!(community = %row.community_id, error = %e, "the sign of life was not kept here");
                }
            }
        }
        if let Some(e) = state_event(row, "sent", None) {
            out.ui.push(e);
        }
        if row.attempts > 0 {
            self.say(&format!("outbox {}: a {} went out after {} refusal(s)", short(&row.community_id), what(row), row.attempts));
        }
        Ok(())
    }

    /// A row did not go out: its next time, twice as far each time, and
    /// what the page is told.
    async fn row_failed(&self, row: &OutboxRow, why: &str, now: i64, out: &mut Flushed) -> Result<()> {
        let attempts = row.attempts + 1;
        let shift = u32::try_from(row.attempts.clamp(0, 16)).unwrap_or(16);
        let next_at = now + OWED_FIRST_WAIT_SECS.saturating_mul(1i64 << shift).min(OWED_MAX_WAIT_SECS);
        let state = if attempts >= FAILED_AFTER_ATTEMPTS { db::outbox::STATE_FAILED } else { db::outbox::STATE_QUEUED };
        db::outbox::retry_later(&self.env.db, row.seq, attempts, next_at, state, Some(why)).await?;
        if let Some(e) = state_event(row, state, Some(why)) {
            out.ui.push(e);
        }
        self.say(&format!("outbox {}: a {} did not go out ({why}), again in {} s", short(&row.community_id), what(row), next_at - now));
        Ok(())
    }

    /// When the next row that can go is due: the head of each channel lane,
    /// the replaceable rows, the notes whose channel rows are gone (and
    /// mine only with an identity at work).
    async fn next_due(&self, now: i64) -> Result<Option<u64>> {
        let rows = db::outbox::all(&self.env.db).await?;
        let mut heads: HashMap<&str, i64> = HashMap::new();
        for r in rows.iter().filter(|r| r.lane == db::outbox::LANE_CHANNEL) {
            heads.entry(r.community_id.as_str()).or_insert(r.seq);
        }
        let signed_in = self.signer().is_some();
        let mut wake: Option<i64> = None;
        for r in &rows {
            let can_go = match r.lane.as_str() {
                db::outbox::LANE_CHANNEL => heads.get(r.community_id.as_str()) == Some(&r.seq),
                db::outbox::LANE_INBOX => {
                    let behind = rows.iter().any(|c| c.lane == db::outbox::LANE_CHANNEL && c.community_id == r.community_id && c.seq <= r.after_seq);
                    !behind && (r.signer != db::outbox::SIGNER_ME || signed_in)
                }
                _ => true,
            };
            if can_go {
                wake = Some(wake.map_or(r.next_at, |w| w.min(r.next_at)));
            }
        }
        Ok(wake.map(|w| u64::try_from(w.max(now)).unwrap_or(0)))
    }

    /// Every row of a channel tried again now (the user asked).
    pub async fn outbox_retry(&self, community_id: &str) -> Result<u64> {
        let n = db::outbox::retry_now(&self.env.db, community_id, self.now()).await?;
        self.env.outbox.notify_one();
        Ok(n)
    }

    /// What a channel has in the outbox, for its page.
    pub async fn outbox_summary(&self, community_id: &str) -> Result<db::outbox::Summary> {
        db::outbox::summary(&self.env.db, community_id).await
    }

    /// The ids the page shows as pending in a channel.
    pub async fn outbox_pending(&self, community_id: &str) -> Result<HashSet<String>> {
        db::outbox::pending_refs(&self.env.db, community_id).await
    }

    /// Whether operations of the journal of a channel still wait for the
    /// relays: the keeper's cycle holds its epoch until they are out
    /// (the `keeper` operation before the first epoch of its series,
    /// wire §15). Content, an epoch or a snapshot that waits holds the
    /// cycle back for nothing (a snapshot goes under the published epoch
    /// while the next waits for its relays).
    pub(crate) async fn outbox_holds_channel(&self, community_id: &str) -> Result<bool> {
        Ok(db::outbox::count_kind(&self.env.db, community_id, db::outbox::LANE_CHANNEL, wire::KIND_JOURNAL).await? > 0)
    }

    /// Queues a signed event of a channel for its relays and wakes the
    /// worker; `ref_id`: what the page shows pending.
    pub(crate) async fn enqueue(&self, community_id: &str, lane: &str, kind: u16, event: &nostr::prelude::Event, ref_id: Option<&str>) -> Result<i64> {
        let json = serde_json::to_string(event)?;
        let seq = db::outbox::enqueue_event(&self.env.db, community_id, lane, kind, &event.id.to_hex(), ref_id, &json, self.now()).await?;
        self.env.outbox.notify_one();
        Ok(seq)
    }
}

/// A row for the log: by its kind.
fn what(row: &OutboxRow) -> &'static str {
    match row.kind.and_then(|k| u16::try_from(k).ok()) {
        Some(wire::KIND_JOURNAL) => "journal operation",
        Some(wire::KIND_CONTENT) => "sealed event",
        Some(wire::KIND_CARD) => "card",
        Some(wire::KIND_EPOCH) => "epoch event",
        Some(wire::KIND_SNAPSHOT) => "snapshot",
        Some(wire::KIND_HEARTBEAT) => "sign of life",
        Some(crate::notes::KIND_WRAP) => "note",
        _ => "row",
    }
}

/// A note of mine queued for the inbox lane (`db::outbox`): to `to` with
/// the channel's relays as the hint (a keeper, a member who follows the
/// channel), to a person's own inbox relays without one (`hint` empty),
/// or to my own devices (`to` none). Wrapped with the signer's keys at
/// every send. `after_seq`: the channel rows up to it go first.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn queue_note(env: &crate::keeper::Env, community_id: &str, to: Option<&PubKey>, hint: &[String], t: &str, body: &impl serde::Serialize, after_seq: i64, ref_id: Option<&str>, now: i64) -> Result<i64> {
    let content = wire::note_envelope(t, body)?;
    let recipient = to.map_or_else(String::new, |p| p.as_hex().to_string());
    let hint: Vec<&str> = hint.iter().map(String::as_str).filter(|r| RelayUrl::parse(r).is_some()).collect();
    let note = db::outbox::NewNote {
        out_id: crate::blobs::sha256_hex(format!("{recipient}:{content}").as_bytes()),
        recipient,
        content,
        hint_json: serde_json::to_string(&hint)?,
        signer: db::outbox::SIGNER_ME.into(),
        after_seq,
        ref_id: ref_id.map(str::to_string),
    };
    let seq = db::outbox::enqueue_note(&env.db, community_id, &note, now).await?;
    env.outbox.notify_one();
    Ok(seq)
}


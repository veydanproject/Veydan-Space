// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A report of a person, or of a whole group, to the moderators of the
//! project (internal/messenger-spec.md, «Жалоба»): the reason, the
//! reporter's own words and, when asked, the last messages of the
//! conversation with the proof that their authors wrote them
//! (`messenger-evidence`). A report from a group carries the name of the
//! group and the message it was opened on; one on a group, its owner as
//! the accused and its admins, as the journal of the group here has them.
//!
//! The messages are gathered before anything else is done: a block of a
//! stranger takes the chat with it, and what was not gathered by then is
//! gone. The report is checked here by the code the server checks it with
//! (`messenger_evidence::verify`), so that nothing malformed leaves and
//! the page learns how many messages were proved.
//!
//! The way to the server is the host's ([`ReportSender`]: the HTTP of the
//! project, signed by the identity, `POST /cloud/v1/report`). A report that
//! does not get there waits in `msg_reports_outbox` and is tried again
//! ([`MessengerRuntime::reports_retry`], called by the host's timer) with a
//! growing pause, for seven days; its delivery is told to the page by
//! `moderation.report`.

use crate::MessengerRuntime;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use messenger_core::traits::{SystemClock, UiEvent};
use messenger_core::{Clock, MessengerError, Result};
use messenger_dm::{Action, DmPeer};
use messenger_evidence::{
    Attached, Context, Proof, Reason, Report, MAX_COMMENT_CHARS, MAX_MESSAGES, REPORT_VERSION,
};
use messenger_store::messages::{self as msgs, MessageRow};
use messenger_store::{events_raw, groups as group_rows, reports as rows};
use nostr::event::{Event, Kind, UnsignedEvent};
use nostr::key::Keys;
use nostr::nips::nip44::{self, v2::ConversationKey};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use ts_rs::TS;

/// The event that tells the page what became of a report that waited:
/// `{id?, status}` — `sent` (with the server's `id`), `refused` (with the
/// server's word in `error`) or `expired` (seven days without getting
/// there).
pub const UI_EVENT_MODERATION_REPORT: &str = "moderation.report";

/// Reports a device sends in a minute at most: a person reports one at a
/// time, and a bug in a loop must not flood the moderators.
pub const REPORTS_PER_MINUTE: usize = 3;
/// How long a report that did not get there is tried again.
pub const RETRY_FOR_SECS: i64 = 7 * 86_400;
/// The pause after the first failure; it doubles with each next one, up to
/// [`LAST_RETRY_SECS`].
pub const FIRST_RETRY_SECS: i64 = 60;
pub const LAST_RETRY_SECS: i64 = 6 * 3600;
/// A report done (sent, refused, given up) is remembered this long.
const KEEP_DONE_SECS: i64 = 30 * 86_400;
/// Reports tried in one round of [`MessengerRuntime::reports_retry`].
const RETRY_BATCH: i64 = 20;
/// The note of a message whose proof is not on this device any more.
pub const NOTE_WRAP_GONE: &str = "wrap_gone";
/// The note of a message whose event is here but proves nothing.
pub const NOTE_UNPROVABLE: &str = "unprovable";
/// The copies of mine looked through for the messages of mine a report
/// attaches, at most: see [`own_copies`].
const OWN_COPY_SCAN_MAX: i64 = 20_000;

// ─── The contract with the page ─────────────────────────────────────────────

/// Where the conduct reported happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ReportContext {
    /// The chat of the two.
    Dm,
    /// A group. On a person, only the messages of the person reported and
    /// of the reporter are attached, never those of the other members; on
    /// the group (`ReportTarget::Group`), those of any member.
    Group { group_id: String },
    /// A request: a stranger wrote first.
    Request,
    /// A channel of a community (the module Communities): its id, and the
    /// post or comment the report was opened on, when it was. Nothing of a
    /// chat goes with it: its messages are not attached, the chat of the
    /// two is not blocked, and `message_id` is not one of a chat.
    Channel {
        community_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        post_id: Option<String>,
    },
}

/// What a report is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    ChildHarm,
    Spam,
    Harassment,
    Illegal,
    Other,
}

/// The kind of harm to a child (only with `child_harm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReportSubReason {
    Sexual,
    Grooming,
    ViolenceIllegal,
    SuicideSelfharm,
    SectRecruitment,
}

impl ReportSubReason {
    /// The word of the wire: the same as the page's.
    fn word(self) -> &'static str {
        match self {
            Self::Sexual => "sexual",
            Self::Grooming => "grooming",
            Self::ViolenceIllegal => "violence_illegal",
            Self::SuicideSelfharm => "suicide_selfharm",
            Self::SectRecruitment => "sect_recruitment",
        }
    }
}

/// Whom a report is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReportTarget {
    /// The person of `accused`.
    #[default]
    Person,
    /// The whole group of the context (`context` must be a group): the
    /// messages of any member go, and the accused is the group's owner.
    Group,
}

/// A report as `messenger_report_send` takes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReportInput {
    /// The person reported: hex (an npub is taken too), or the address of
    /// a private chat (`<hex>@<server id>`, or its chat id) for a report
    /// from it; a bare key with a `message_id` of a private chat with that
    /// person is that chat too. Ignored for a report on a group, whose
    /// owner the journal of the group names; the page may pass an empty
    /// string then.
    pub accused: String,
    /// A person (when absent) or the whole group of the context;
    /// `report_target` when a group is reported outside of one.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub target: Option<ReportTarget>,
    pub context: ReportContext,
    pub reason: ReportReason,
    /// Only with `child_harm`.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub sub_reason: Option<ReportSubReason>,
    /// The reporter's own words, at most 2000 characters.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub comment: Option<String>,
    /// Attach the last messages (up to ten) with their proofs.
    pub attach: bool,
    /// The message the report starts from: the attached run ends with it;
    /// with the newest when none.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub message_id: Option<String>,
    /// Block the person once the report is made (a chat of two or a
    /// request; ignored for a group).
    pub block: bool,
}

/// Whether the report reached the server now or waits on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Sent,
    Queued,
}

/// What `messenger_report_send` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReportResult {
    pub status: ReportStatus,
    /// Messages attached, with a proof or without one.
    pub attached: u32,
    /// Of those, the ones whose author's key is proved (the check the
    /// server makes).
    pub proved: u32,
    /// The person was blocked.
    pub blocked: bool,
}

// ─── The way to the server ──────────────────────────────────────────────────

/// What became of one try to send a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The server took it (`202`), under its `id`.
    Accepted { id: Option<String> },
    /// The server refused it (`4xx`): its word. Sending it again changes
    /// nothing.
    Refused { error: String },
    /// It did not get there, or the server could not take it now (the
    /// network, `5xx`, `429`): worth another try later.
    Failed { why: String },
    /// There is no way to the server at all (the servers in use are the
    /// user's own, no host is known, the messenger stopped): a fresh report
    /// is refused with this word rather than kept; a waiting one waits on.
    NoWay { why: String },
}

/// The host's way to the moderators of the project: one try of one report,
/// `body` the report's JSON, signed by `keys` (NIP-98).
#[async_trait]
pub trait ReportSender: Send + Sync {
    async fn send(&self, keys: &Keys, body: &[u8]) -> Delivery;
}

/// What the runtime keeps of reports in memory: the rate of this device,
/// and one round of retries at a time.
#[derive(Default)]
pub(crate) struct Reports {
    recent: std::sync::Mutex<VecDeque<Instant>>,
    retrying: tokio::sync::Mutex<()>,
}

impl Reports {
    /// One more report now, or `rate_limited` past [`REPORTS_PER_MINUTE`].
    fn admit(&self, now: Instant) -> Result<()> {
        let mut recent = self
            .recent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while recent
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            recent.pop_front();
        }
        if recent.len() >= REPORTS_PER_MINUTE {
            return Err(MessengerError::Invalid("rate_limited".into()));
        }
        recent.push_back(now);
        Ok(())
    }
}

/// The pause before the next try after `attempts` tries.
fn backoff(attempts: i64) -> i64 {
    let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 20)).unwrap_or(20);
    FIRST_RETRY_SECS
        .saturating_mul(1 << doublings)
        .min(LAST_RETRY_SECS)
}

fn invalid(word: &str) -> MessengerError {
    MessengerError::Invalid(word.to_string())
}

fn local_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(hex::encode(bytes))
}

impl MessengerRuntime {
    /// Report a person: gather the messages, make and check the report,
    /// send it (or keep it to send later), then block when asked. A refusal
    /// of the server comes back as its word; nothing is blocked then, so
    /// the page may try again with what was typed.
    pub async fn report_send(
        &self,
        input: ReportInput,
        sender: &dyn ReportSender,
    ) -> Result<ReportResult> {
        let keys = self.session_keys().await?;
        let (report, chat) = self.report_make(&keys, &input).await?;
        let now = report.created_at;
        let verified = messenger_evidence::verify(&report, now);
        if let Some(why) = verified.refused {
            return Err(MessengerError::Invalid(format!("report_{why}")));
        }
        let (attached, proved) = (
            report.messages.len() as u32,
            verified.proved().count() as u32,
        );
        let body = serde_json::to_vec(&report)?;
        let status = match sender.send(&keys, &body).await {
            Delivery::Accepted { .. } => ReportStatus::Sent,
            Delivery::Failed { why } => {
                let json =
                    String::from_utf8(body).map_err(|e| MessengerError::Invalid(e.to_string()))?;
                rows::queue(
                    self.store(),
                    &local_id()?,
                    &json,
                    now,
                    1,
                    now + backoff(1),
                    Some(&why),
                )
                .await?;
                ReportStatus::Queued
            }
            Delivery::Refused { error } => return Err(MessengerError::Invalid(error)),
            Delivery::NoWay { why } => return Err(MessengerError::Invalid(why)),
        };
        let blocked = input.block && !matches!(input.context, ReportContext::Group { .. } | ReportContext::Channel { .. });
        // The block of the chat reported: a private chat's is that chat's
        // alone, as from its banner, and nothing of it goes home.
        if blocked {
            self.dm_act(&chat, Action::Block).await?;
        }
        Ok(ReportResult {
            status,
            attached,
            proved,
            blocked,
        })
    }

    /// The report the input asks for, its messages gathered, not sent;
    /// beside it, the chat of the two (its id), which a block takes.
    async fn report_make(&self, keys: &Keys, input: &ReportInput) -> Result<(Report, String)> {
        let me = keys.public_key().to_hex();
        let target = input.target.unwrap_or_default();
        let group_id = match &input.context {
            ReportContext::Group { group_id } => Some(group_id.as_str()),
            ReportContext::Dm | ReportContext::Request | ReportContext::Channel { .. } => None,
        };
        // A channel's posts are not messages of a chat here: nothing of a
        // chat is looked up, attached or blocked for it.
        let channel = matches!(input.context, ReportContext::Channel { .. });
        // The group of the context, as this device holds it: its row, and
        // for a report on it, its owner and admins by its journal.
        let group = match group_id {
            Some(id) => Some(group_rows::get(self.store(), id).await?.ok_or_else(|| invalid("report_group"))?),
            None => None,
        };
        let (accused, group_admins, addressed) = match (target, group_id) {
            (ReportTarget::Person, _) => {
                let addressed =
                    crate::addr::parse(&input.accused).map_err(|_| invalid("report_accused"))?;
                (addressed.key.as_hex().to_string(), Vec::new(), Some(addressed))
            }
            (ReportTarget::Group, None) => return Err(invalid("report_target")),
            // The journal decides who answers for the group, not the page:
            // the owner now (after a transfer, the new one), the admins
            // beside them, as many as a report carries.
            (ReportTarget::Group, Some(id)) => {
                let (owner, leaders) = self.groups().leaders(id).await?.ok_or_else(|| invalid("report_group"))?;
                let admins = leaders
                    .iter()
                    .take(messenger_evidence::MAX_GROUP_ADMINS)
                    .map(|k| k.as_hex().to_string())
                    .collect();
                (owner.as_hex().to_string(), admins, None)
            }
        };
        // Its owner may report a channel of their own (its content, no post
        // named; Google Play, 2026-10-22): `messenger_evidence::verify`
        // lets that one through; anything else on oneself is refused.
        let own_channel = matches!(input.context, ReportContext::Channel { post_id: None, .. });
        if accused == me && !own_channel {
            return Err(invalid("report_self"));
        }
        // The name this device shows, cut to what a report carries: it
        // tells the moderator which group, it is nobody's word to refuse.
        let group_name = group
            .as_ref()
            .map(|g| g.name.trim().chars().take(messenger_evidence::MAX_GROUP_NAME_CHARS).collect::<String>())
            .filter(|n| !n.is_empty());
        let chat_id = match (group_id, addressed) {
            (Some(id), _) => group_rows::group_chat_id(id),
            (None, Some(addressed)) => self
                .report_chat(addressed, input.message_id.as_deref())
                .await?
                .chat_id(),
            (None, None) => return Err(invalid("report_target")),
        };
        // The message the report was opened on is one of this chat, or the
        // report is malformed, attached messages or not.
        let start = match input.message_id.as_deref().filter(|_| !channel) {
            None => None,
            Some(id) => {
                let row = msgs::get(self.store(), id)
                    .await?
                    .filter(|r| r.chat_id == chat_id)
                    .ok_or_else(|| invalid("report_message"))?;
                Some((row.created_at, row.id))
            }
        };
        // The wire takes the id of an event only; a row of another shape
        // (none is known) is still a start, not a claim.
        let message_id = start
            .as_ref()
            .map(|(_, id)| id.clone())
            .filter(|id| id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        if input.sub_reason.is_some() && input.reason != ReportReason::ChildHarm {
            return Err(invalid("report_sub_reason"));
        }
        let comment = input
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(String::from);
        if comment
            .as_deref()
            .is_some_and(|c| c.chars().count() > MAX_COMMENT_CHARS)
        {
            return Err(invalid("report_comment"));
        }
        let context = match &input.context {
            ReportContext::Dm => Context::Dm,
            ReportContext::Request => Context::Request,
            ReportContext::Group { group_id } => Context::Group {
                group_id: group_id.clone(),
            },
            ReportContext::Channel {
                community_id,
                post_id,
            } => Context::Channel {
                community_id: community_id.clone(),
                post_id: post_id.clone(),
            },
        };
        // Counted before the work, so that a loop is stopped before it
        // reads the chats.
        self.reports.admit(Instant::now())?;
        let messages = if input.attach && !channel {
            // On a group, the conduct is the group's: any member's words.
            let authors = match target {
                ReportTarget::Person => Some((accused.as_str(), me.as_str())),
                ReportTarget::Group => None,
            };
            self.report_evidence(keys, &chat_id, group_id, authors, start.as_ref())
                .await?
        } else {
            Vec::new()
        };
        let report = Report {
            target: match target {
                ReportTarget::Person => messenger_evidence::Target::Person,
                ReportTarget::Group => messenger_evidence::Target::Group,
            },
            group_name,
            group_admins,
            message_id,
            v: REPORT_VERSION,
            reporter: me,
            accused,
            reason: match input.reason {
                ReportReason::ChildHarm => Reason::ChildHarm,
                ReportReason::Spam => Reason::Spam,
                ReportReason::Harassment => Reason::Harassment,
                ReportReason::Illegal => Reason::Illegal,
                ReportReason::Other => Reason::Other,
            },
            sub_reason: input.sub_reason.map(|s| s.word().to_string()),
            comment,
            context,
            messages,
            created_at: SystemClock.now().secs(),
        };
        Ok((report, chat_id))
    }

    /// The chat of two a report on a person is about: the one the address
    /// of the accused names (a private chat's `<key>@<server id>` or its
    /// chat id, on a server of mine), else the chat of the message the
    /// report was opened on when that is a chat with the accused (the page
    /// may pass the bare key from a private chat), else the home chat.
    async fn report_chat(&self, addressed: DmPeer, message_id: Option<&str>) -> Result<DmPeer> {
        if addressed.is_private() {
            return self.peer_addr(&addressed.chat_id()).await;
        }
        let Some(id) = message_id else {
            return Ok(addressed);
        };
        let of_message = msgs::get(self.store(), id)
            .await?
            .and_then(|row| DmPeer::of_chat(&row.chat_id))
            .filter(|chat| chat.key == addressed.key);
        Ok(of_message.unwrap_or(addressed))
    }

    /// The last messages of `chat_id` written by one of `authors` (the
    /// accused and me), or by anybody on a report on the group (`None`),
    /// up to `start`, each with its proof or the word why there is none,
    /// the oldest first. `group` is the group of the chat, whose key opens
    /// the proofs of its messages.
    async fn report_evidence(
        &self,
        keys: &Keys,
        chat_id: &str,
        group: Option<&str>,
        authors: Option<(&str, &str)>,
        start: Option<&(i64, String)>,
    ) -> Result<Vec<Attached>> {
        let run = msgs::evidence_run(
            self.store(),
            chat_id,
            authors,
            start.map(|(at, id)| (*at, id.as_str())),
            MAX_MESSAGES as i64,
        )
        .await?;
        let proofs = match group {
            None => self.dm_proofs(keys, &run).await?,
            Some(group_id) => self.group_proofs(group_id, &run).await?,
        };
        Ok(proofs
            .into_iter()
            .map(|p| match p {
                Ok(proof) => Attached {
                    proof: Some(proof),
                    note: None,
                },
                Err(note) => Attached {
                    proof: None,
                    note: Some(note.to_string()),
                },
            })
            .collect())
    }

    /// The proofs of DMs. A message of the peer is proved by the wrap it
    /// came in (its row names it). A message of mine is proved by my own
    /// copy, which its row does not name when it was written here (the row
    /// keeps the id of the peer's copy, which this device cannot open):
    /// that one is looked for among the wraps kept ([`own_copies`]).
    async fn dm_proofs(
        &self,
        keys: &Keys,
        run: &[MessageRow],
    ) -> Result<Vec<std::result::Result<Proof, &'static str>>> {
        let mut out = Vec::with_capacity(run.len());
        let mut mine: HashMap<String, usize> = HashMap::new();
        for (i, row) in run.iter().enumerate() {
            let wrap = match row.wire_id.as_deref() {
                Some(id) => events_raw::raw_json(self.store(), id).await?,
                None => None,
            };
            let proved = wrap
                .as_deref()
                .and_then(|json| serde_json::from_str::<Event>(json).ok())
                .map(|wrap| messenger_evidence::dm_proof(keys, &wrap));
            out.push(match proved {
                Some(Ok(proof)) => Ok(proof),
                // Mine, with the peer's copy here or none: my own copy may be.
                _ if row.direction == msgs::DIR_OUT => {
                    mine.insert(row.id.clone(), i);
                    Err(NOTE_WRAP_GONE)
                }
                Some(Err(_)) => Err(NOTE_UNPROVABLE),
                None => Err(NOTE_WRAP_GONE),
            });
        }
        if !mine.is_empty() {
            let bounds = mine.values().map(|&i| run[i].created_at);
            let (first, last) = (bounds.clone().min().unwrap_or(0), bounds.max().unwrap_or(0));
            for (id, proof) in own_copies(self.store(), keys, &mine, first, last).await? {
                if let Some(&i) = mine.get(&id) {
                    out[i] = Ok(proof);
                }
            }
        }
        Ok(out)
    }

    /// The proofs of group messages: the sealed event the row names,
    /// opened with the group's key, gives the inner event its author
    /// signed.
    async fn group_proofs(
        &self,
        group_id: &str,
        run: &[MessageRow],
    ) -> Result<Vec<std::result::Result<Proof, &'static str>>> {
        let mut out = Vec::with_capacity(run.len());
        for row in run {
            let outer = match row.wire_id.as_deref() {
                Some(id) => events_raw::raw_json(self.store(), id).await?,
                None => None,
            };
            let Some(outer) =
                outer.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            else {
                out.push(Err(NOTE_WRAP_GONE));
                continue;
            };
            out.push(match self.groups().inner_message(group_id, &outer).await? {
                Some(inner) => messenger_evidence::group_proof(&inner).map_err(|_| NOTE_UNPROVABLE),
                // The key that opens it is gone from this device.
                None => Err(NOTE_WRAP_GONE),
            });
        }
        Ok(out)
    }

    /// Try the reports that wait and are due by `now`; the host calls it on
    /// a timer. Each one sent, refused or given up is told to the page
    /// (`moderation.report`). Answers how many were sent.
    pub async fn reports_retry(&self, sender: &dyn ReportSender, now: i64) -> Result<u32> {
        let _one_round = self.reports.retrying.lock().await;
        rows::purge_done(self.store(), now - KEEP_DONE_SECS).await?;
        let due = rows::due(self.store(), now, RETRY_BATCH).await?;
        if due.is_empty() {
            return Ok(0);
        }
        // Without a session nothing is signed: they wait for the next one.
        let Ok(keys) = self.session_keys().await else {
            return Ok(0);
        };
        let mut sent = 0;
        for row in due {
            if now - row.created_at > RETRY_FOR_SECS {
                rows::finish(self.store(), &row.id, rows::STATE_EXPIRED, None, now).await?;
                self.tell_report(serde_json::json!({ "status": rows::STATE_EXPIRED }));
                continue;
            }
            match sender.send(&keys, row.report_json.as_bytes()).await {
                Delivery::Accepted { id } => {
                    rows::finish(self.store(), &row.id, rows::STATE_SENT, None, now).await?;
                    let mut told = serde_json::json!({ "status": rows::STATE_SENT });
                    if let Some(id) = id {
                        told["id"] = serde_json::Value::String(id);
                    }
                    self.tell_report(told);
                    sent += 1;
                }
                Delivery::Refused { error } => {
                    rows::finish(
                        self.store(),
                        &row.id,
                        rows::STATE_REFUSED,
                        Some(&error),
                        now,
                    )
                    .await?;
                    self.tell_report(
                        serde_json::json!({ "status": rows::STATE_REFUSED, "error": error }),
                    );
                }
                Delivery::Failed { why } | Delivery::NoWay { why } => {
                    let attempts = row.attempts + 1;
                    rows::retry_later(
                        self.store(),
                        &row.id,
                        attempts,
                        now + backoff(attempts),
                        &why,
                        now,
                    )
                    .await?;
                }
            }
        }
        Ok(sent)
    }

    fn tell_report(&self, payload: serde_json::Value) {
        let _ = self.ui.send(UiEvent {
            name: UI_EVENT_MODERATION_REPORT.into(),
            payload,
        });
    }
}

/// My own copies of the messages `wanted` (rumor id → anything), among the
/// gift wraps kept here: `(rumor id, proof)` of each one found.
///
/// A wrap is dated by NIP-59 up to two days before it was made, and a
/// message up to a few minutes after it, so the copy of a message dated
/// `t` lies in `[t − 2 days − slack, t + slack]`. Each wrap of that window
/// is opened as far as its seal; one sealed by somebody else is a message
/// to me and is left there; one sealed by me is opened with the one key a
/// copy of mine is sealed with (mine to mine), and its rumor's id is
/// compared. At most [`OWN_COPY_SCAN_MAX`] wraps, the newest first.
async fn own_copies(
    store: &messenger_store::Store,
    keys: &Keys,
    wanted: &HashMap<String, usize>,
    first: i64,
    last: i64,
) -> Result<Vec<(String, Proof)>> {
    let slack =
        messenger_evidence::SEAL_TIME_SLACK_SECS + messenger_dm::relationship::FLOOR_SKEW_SECS;
    let since = first.saturating_sub(messenger_evidence::SEAL_TWEAK_SECS + slack);
    let until = last.saturating_add(slack);
    let candidates = events_raw::raw_in_window(
        store,
        messenger_evidence::KIND_GIFT_WRAP,
        since,
        until,
        OWN_COPY_SCAN_MAX,
    )
    .await?;
    let me = keys.public_key();
    let to_myself = ConversationKey::derive(keys.secret_key(), &me)
        .map_err(|e| MessengerError::Crypto(e.to_string()))?;
    let mut found = Vec::new();
    for (_, json) in candidates {
        if found.len() == wanted.len() {
            break;
        }
        let Ok(wrap) = serde_json::from_str::<Event>(&json) else {
            continue;
        };
        let Ok(seal) = nip44::decrypt(keys.secret_key(), &wrap.pubkey, &wrap.content) else {
            continue;
        };
        let Ok(seal) = serde_json::from_str::<Event>(&seal) else {
            continue;
        };
        if seal.pubkey != me || seal.kind != Kind::Seal {
            continue;
        }
        let Ok(payload) = STANDARD.decode(seal.content.as_bytes()) else {
            continue;
        };
        let Ok(plain) = nip44::v2::decrypt_to_bytes(&to_myself, &payload) else {
            continue;
        };
        let Ok(rumor) = serde_json::from_slice::<UnsignedEvent>(&plain) else {
            continue;
        };
        let id = rumor.compute_id().to_hex();
        if !wanted.contains_key(&id) || found.iter().any(|(f, _)| *f == id) {
            continue;
        }
        if let Ok(proof) = messenger_evidence::dm_proof(keys, &wrap) {
            found.push((id, proof));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ctx_of, pk_of};
    use messenger_core::outbound::WireEvent;
    use messenger_core::{
        Envelope, EventSource, MessengerConfig, PubKey, RawEvent, RelayUrl, Timestamp,
    };
    use messenger_store::chats;
    use messenger_store::messages::NewMessage;
    use messenger_testkit::{open_dm, MemorySecretStore};
    use std::sync::{Arc, Mutex};

    /// A runtime with a session, offline; `gate`: a stranger's first word
    /// is a request.
    async fn started(gate: bool) -> (tempfile::TempDir, MessengerRuntime, Keys) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked()))
            .await
            .unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        rt.dm().set_gate(gate);
        let keys = rt.session_keys().await.unwrap();
        (dir, rt, keys)
    }

    fn now() -> i64 {
        SystemClock.now().secs()
    }

    /// A sender that answers what it is told to and keeps what it got.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<VecDeque<Delivery>>,
        got: Mutex<Vec<Report>>,
    }

    impl Fake {
        fn answering(answers: impl IntoIterator<Item = Delivery>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                got: Mutex::default(),
            }
        }

        fn last(&self) -> Report {
            self.got
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("a report was sent")
        }
    }

    #[async_trait]
    impl ReportSender for Fake {
        async fn send(&self, keys: &Keys, body: &[u8]) -> Delivery {
            let report: Report = serde_json::from_slice(body).expect("the body is a report");
            assert_eq!(
                report.reporter,
                keys.public_key().to_hex(),
                "signed by whoever reports"
            );
            self.got.lock().unwrap().push(report);
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Delivery::Accepted {
                    id: Some("r".into()),
                })
        }
    }

    fn input(accused: &Keys, context: ReportContext) -> ReportInput {
        ReportInput {
            accused: accused.public_key().to_hex(),
            target: None,
            context,
            reason: ReportReason::Harassment,
            sub_reason: None,
            comment: Some("again and again".into()),
            attach: true,
            message_id: None,
            block: false,
        }
    }

    /// A gift wrap as ingress takes it: kept in `msg_events_raw`, then
    /// handed to the DM service.
    async fn receive(rt: &MessengerRuntime, me: &Keys, wrap: &WireEvent) {
        let ev: Event = serde_json::from_value(wrap.json.clone()).unwrap();
        let raw = RawEvent {
            id: wrap.id.clone(),
            kind: 1059,
            pubkey: PubKey::parse(&ev.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(ev.created_at.as_secs() as i64),
            json: wrap.json.clone(),
            source: EventSource::Relay {
                url: RelayUrl::parse("wss://r.example").unwrap(),
            },
        };
        assert!(events_raw::insert_if_new(rt.store(), &raw).await.unwrap());
        let msg = open_dm(me, wrap, false).expect("addressed to me");
        rt.dm()
            .apply_inbound(msg, &ctx_of(&pk_of(me), now()))
            .await
            .unwrap();
    }

    /// `text` of `peer` to me, dated after everything in the chat.
    async fn from_peer(rt: &MessengerRuntime, me: &Keys, peer: &Keys, text: &str) -> String {
        let chat = chats::dm_chat_id(&peer.public_key().to_hex(), None);
        let at = msgs::last_created_at(rt.store(), &chat)
            .await
            .unwrap()
            .map_or(now() - 600, |l| l + 1);
        let w =
            messenger_dm::wrap::wrap(peer, pk_of(me), &Envelope::text(text).encode(), at, None)
                .unwrap();
        receive(rt, me, &w.to_peer).await;
        w.rumor_id.as_hex().to_string()
    }

    /// `text` of mine to `peer`, and my own copy of it coming back from my
    /// relay, as it does: the row keeps the id of the peer's copy.
    async fn to_peer(rt: &MessengerRuntime, me: &Keys, peer: &Keys, text: &str) -> String {
        let p = rt
            .dm()
            .prepare_text(me, &pk_of(peer), text, None)
            .await
            .unwrap();
        let row = msgs::get(rt.store(), &p.message.id).await.unwrap().unwrap();
        let messenger_core::Outbound::PublishToInbox { event: peers, .. } = &p.to_peer else {
            panic!("a message goes to the peer's inbox")
        };
        assert_eq!(
            row.wire_id.as_deref(),
            Some(peers.id.as_hex()),
            "the row names the peer's copy"
        );
        let Some(messenger_core::Outbound::PublishOwn { event }) = p.to_self else {
            panic!("a message has a copy of mine")
        };
        receive(rt, me, &event).await;
        p.message.id
    }

    fn ids(report: &Report) -> Vec<String> {
        messenger_evidence::verify(report, now())
            .messages
            .iter()
            .map(|o| match o {
                messenger_evidence::Outcome::Verified(m) => m.id.clone(),
                messenger_evidence::Outcome::Unverified(why) => format!("unverified:{why}"),
            })
            .collect()
    }

    /// Twelve messages, both ways: the last ten go, each proved — the
    /// peer's by the wrap it came in, mine by my own copy — and a report
    /// that starts from a message ends with it.
    #[tokio::test]
    async fn the_last_ten_messages_go_each_with_its_proof() {
        let (_dir, rt, me) = started(false).await;
        let bob = Keys::generate();
        let mut sent = Vec::new();
        for i in 0..12 {
            sent.push(if i % 3 == 2 {
                to_peer(&rt, &me, &bob, &format!("mine {i}")).await
            } else {
                from_peer(&rt, &me, &bob, &format!("his {i}")).await
            });
        }
        let fake = Fake::default();
        let r = rt
            .report_send(input(&bob, ReportContext::Dm), &fake)
            .await
            .unwrap();
        assert_eq!(
            r,
            ReportResult {
                status: ReportStatus::Sent,
                attached: 10,
                proved: 10,
                blocked: false
            }
        );
        let report = fake.last();
        assert_eq!(
            ids(&report),
            sent[2..].to_vec(),
            "the last ten, oldest first, all proved"
        );
        assert_eq!(
            (report.context.clone(), report.comment.as_deref()),
            (Context::Dm, Some("again and again"))
        );

        let r = rt
            .report_send(
                ReportInput {
                    message_id: Some(sent[4].clone()),
                    ..input(&bob, ReportContext::Dm)
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!((r.attached, r.proved), (5, 5));
        assert_eq!(
            ids(&fake.last()),
            sent[..5].to_vec(),
            "up to the message it starts from"
        );
        assert_eq!(
            fake.last().message_id.as_deref(),
            Some(sent[4].as_str()),
            "and it names that message"
        );
        assert_eq!(fake.last().group_name, None, "no group in a chat of two");

        let r = rt
            .report_send(
                ReportInput {
                    attach: false,
                    ..input(&bob, ReportContext::Dm)
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!((r.attached, r.proved), (0, 0));
        assert!(fake.last().messages.is_empty());
        rt.shutdown().await;
    }

    /// A message whose wrap this device no longer keeps goes with the word
    /// why, and proves nothing; the others still do.
    #[tokio::test]
    async fn a_message_without_its_wrap_goes_with_the_word_why() {
        let (_dir, rt, me) = started(false).await;
        let bob = Keys::generate();
        let first = from_peer(&rt, &me, &bob, "one").await;
        to_peer(&rt, &me, &bob, "two").await;
        from_peer(&rt, &me, &bob, "three").await;
        let wire = msgs::get(rt.store(), &first)
            .await
            .unwrap()
            .unwrap()
            .wire_id
            .unwrap();
        sqlx::query("DELETE FROM msg_events_raw WHERE event_id = ?")
            .bind(&wire)
            .execute(rt.store().pool())
            .await
            .unwrap();

        let fake = Fake::default();
        let r = rt
            .report_send(input(&bob, ReportContext::Dm), &fake)
            .await
            .unwrap();
        assert_eq!((r.attached, r.proved), (3, 2));
        let report = fake.last();
        assert_eq!(
            report.messages[0],
            Attached {
                proof: None,
                note: Some(NOTE_WRAP_GONE.into())
            }
        );
        assert!(report.messages[1..].iter().all(|m| m.proof.is_some()));
        rt.shutdown().await;
    }

    /// A group: the messages of the person reported and mine go, those of
    /// the other members never; each one is the inner event its author
    /// signed, opened with the key of the group. A block is not a group's.
    #[tokio::test]
    async fn a_group_report_carries_the_two_parties_only() {
        let (_dir, rt, me) = started(false).await;
        let g = rt
            .group_create(messenger_groups::GroupKind::Private, "Team", "", true, None)
            .await
            .unwrap();
        let (_, key) = rt
            .groups()
            .export_keys()
            .await
            .unwrap()
            .into_iter()
            .find(|(id, _)| *id == g.id)
            .expect("the key of my group");
        let (bob, carl) = (Keys::generate(), Keys::generate());
        let mut theirs = Vec::new();
        let base = now() - 300;
        for (i, (who, text)) in [
            (&bob, "b1"),
            (&carl, "c1"),
            (&me, "m1"),
            (&bob, "b2"),
            (&carl, "c2"),
            (&me, "m2"),
        ]
        .into_iter()
        .enumerate()
        {
            let at = base + i as i64;
            let signed = messenger_groups::wire::sign_message(
                who,
                &g.id,
                &Envelope::text(text).encode(),
                at,
                None,
            )
            .unwrap();
            let sealed =
                messenger_groups::wire::seal_message(&g.id, &key, None, &signed, who).unwrap();
            let raw = RawEvent {
                id: sealed.id.clone(),
                kind: 9,
                pubkey: PubKey::parse(&"ab".repeat(32)).unwrap(),
                created_at: Timestamp(at),
                json: sealed.json.clone(),
                source: EventSource::Relay {
                    url: RelayUrl::parse("wss://r.example").unwrap(),
                },
            };
            events_raw::insert_if_new(rt.store(), &raw).await.unwrap();
            let mine = who.public_key() == me.public_key();
            msgs::insert(
                rt.store(),
                &NewMessage {
                    id: signed.id.to_hex(),
                    chat_id: g.chat_id.clone(),
                    wire_id: Some(sealed.id.as_hex().to_string()),
                    direction: if mine {
                        msgs::DIR_OUT.into()
                    } else {
                        msgs::DIR_IN.into()
                    },
                    status: if mine {
                        msgs::STATUS_SENT.into()
                    } else {
                        msgs::STATUS_RECEIVED.into()
                    },
                    content_type: msgs::CT_TEXT.into(),
                    text: Some(text.into()),
                    envelope_json: Envelope::text(text).encode(),
                    sender_pubkey: who.public_key().to_hex(),
                    reply_to_id: None,
                    target_id: None,
                    created_at: at,
                    is_hidden: false,
                    outbox_local_id: None,
                    media_json: None,
                },
            )
            .await
            .unwrap();
            if who.public_key() != carl.public_key() {
                theirs.push(signed.id.to_hex());
            }
        }
        let fake = Fake::default();
        let context = ReportContext::Group {
            group_id: g.id.clone(),
        };
        let r = rt
            .report_send(
                ReportInput {
                    block: true,
                    ..input(&bob, context)
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!(
            r,
            ReportResult {
                status: ReportStatus::Sent,
                attached: 4,
                proved: 4,
                blocked: false
            }
        );
        assert_eq!(
            ids(&fake.last()),
            theirs,
            "Carl's words are not the report's"
        );
        assert!(
            !rt.dm().relation(&pk_of(&bob)).await.unwrap().blocked,
            "a group report blocks nobody"
        );

        let unknown = ReportContext::Group {
            group_id: "f".repeat(64),
        };
        assert!(
            matches!(rt.report_send(input(&bob, unknown), &fake).await, Err(MessengerError::Invalid(ref w)) if w == "report_group")
        );
        rt.shutdown().await;
    }

    /// `text` of `who` in the group, as it arrives: the sealed event kept in
    /// `msg_events_raw`, the row of the chat naming it. The id of the inner
    /// event its author signed, the message's id.
    async fn say_in_group(
        rt: &MessengerRuntime,
        g: &messenger_groups::GroupView,
        key: &messenger_groups::GroupKey,
        who: &Keys,
        me: &Keys,
        text: &str,
        at: i64,
    ) -> String {
        let signed =
            messenger_groups::wire::sign_message(who, &g.id, &Envelope::text(text).encode(), at, None)
                .unwrap();
        let sealed = messenger_groups::wire::seal_message(&g.id, key, None, &signed, who).unwrap();
        let raw = RawEvent {
            id: sealed.id.clone(),
            kind: 9,
            pubkey: PubKey::parse(&"ab".repeat(32)).unwrap(),
            created_at: Timestamp(at),
            json: sealed.json.clone(),
            source: EventSource::Relay {
                url: RelayUrl::parse("wss://r.example").unwrap(),
            },
        };
        events_raw::insert_if_new(rt.store(), &raw).await.unwrap();
        let mine = who.public_key() == me.public_key();
        msgs::insert(
            rt.store(),
            &NewMessage {
                id: signed.id.to_hex(),
                chat_id: g.chat_id.clone(),
                wire_id: Some(sealed.id.as_hex().to_string()),
                direction: if mine { msgs::DIR_OUT.into() } else { msgs::DIR_IN.into() },
                status: if mine { msgs::STATUS_SENT.into() } else { msgs::STATUS_RECEIVED.into() },
                content_type: msgs::CT_TEXT.into(),
                text: Some(text.into()),
                envelope_json: Envelope::text(text).encode(),
                sender_pubkey: who.public_key().to_hex(),
                reply_to_id: None,
                target_id: None,
                created_at: at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: None,
            },
        )
        .await
        .unwrap();
        signed.id.to_hex()
    }

    /// A group made by me, then handed over: Bob and Carl let in, Carl an
    /// admin, Bob the owner, I an admin as the transfer leaves me. With the
    /// key its messages are sealed with now.
    async fn handed_over(
        rt: &MessengerRuntime,
        bob: &Keys,
        carl: &Keys,
    ) -> (messenger_groups::GroupView, messenger_groups::GroupKey) {
        let g = rt
            .group_create(messenger_groups::GroupKind::Private, "Team", "", true, None)
            .await
            .unwrap();
        for who in [bob, carl] {
            group_rows::put_request(
                rt.store(),
                &group_rows::RequestRow {
                    group_id: g.id.clone(),
                    requester: who.public_key().to_hex(),
                    direction: "in".into(),
                    status: "pending".into(),
                    payload_json: "{}".into(),
                    created_at: now(),
                    updated_at: 0,
                },
            )
            .await
            .unwrap();
            rt.group_answer_request(&g.id, &who.public_key().to_hex(), true)
                .await
                .unwrap();
        }
        rt.group_act(
            &g.id,
            messenger_groups::OpBody::SetRole {
                who: pk_of(carl),
                role: messenger_groups::Role::Admin,
            },
        )
        .await
        .unwrap();
        let g = rt
            .group_act(&g.id, messenger_groups::OpBody::TransferOwnership { to: pk_of(bob) })
            .await
            .unwrap();
        assert_eq!(g.owner, bob.public_key().to_hex(), "the journal names Bob");
        let current = g.key.clone().expect("a member sees the key").id;
        let (_, key) = rt
            .groups()
            .export_keys()
            .await
            .unwrap()
            .into_iter()
            .find(|(id, k)| *id == g.id && k.id().0 == current)
            .expect("the key in use");
        (g, key)
    }

    /// A report on the group: the last messages of any member, each under
    /// its own author's proof; the owner by the journal is the accused
    /// whatever the page said, the admins beside, the name of the group as
    /// this device shows it, and nobody blocked.
    #[tokio::test]
    async fn a_report_on_a_group_carries_any_member_and_names_its_owner() {
        let (_dir, rt, me) = started(false).await;
        let (bob, carl) = (Keys::generate(), Keys::generate());
        let (g, key) = handed_over(&rt, &bob, &carl).await;
        let base = now() - 300;
        let mut said = Vec::new();
        for (i, who) in [&bob, &carl, &me, &carl, &bob].into_iter().enumerate() {
            said.push(say_in_group(&rt, &g, &key, who, &me, &format!("m{i}"), base + i as i64).await);
        }
        let fake = Fake::default();
        let r = rt
            .report_send(
                ReportInput {
                    accused: String::new(),
                    target: Some(ReportTarget::Group),
                    block: true,
                    ..input(&carl, ReportContext::Group { group_id: g.id.clone() })
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!(
            r,
            ReportResult { status: ReportStatus::Sent, attached: 5, proved: 5, blocked: false }
        );
        let report = fake.last();
        assert_eq!(ids(&report), said, "every member's, all proved");
        let authors: std::collections::BTreeSet<String> = messenger_evidence::verify(&report, now())
            .proved()
            .map(|m| m.author.clone())
            .collect();
        assert_eq!(authors.len(), 3, "three members, each under their own key");
        assert_eq!(report.target, messenger_evidence::Target::Group);
        assert_eq!(report.accused, bob.public_key().to_hex(), "the owner, not the page's word");
        let mut admins = vec![carl.public_key().to_hex(), me.public_key().to_hex()];
        admins.sort();
        assert_eq!(report.group_admins, [vec![bob.public_key().to_hex()], admins].concat(), "the owner first");
        assert_eq!(report.group_name.as_deref(), Some("Team"));
        assert_eq!(report.message_id, None, "opened on no message");

        // Opened on a message: the run ends with it, and the report says so.
        rt.report_send(
            ReportInput {
                accused: String::new(),
                target: Some(ReportTarget::Group),
                message_id: Some(said[2].clone()),
                ..input(&carl, ReportContext::Group { group_id: g.id.clone() })
            },
            &fake,
        )
        .await
        .unwrap();
        assert_eq!(ids(&fake.last()), said[..3].to_vec());
        assert_eq!(fake.last().message_id.as_deref(), Some(said[2].as_str()));
        rt.shutdown().await;
    }

    /// A report on a person from a group: the two parties' messages only,
    /// with the name of the group and the message it was opened on; the
    /// group's admins are no claim of it.
    #[tokio::test]
    async fn a_report_on_a_person_from_a_group_names_the_group_and_the_message() {
        let (_dir, rt, me) = started(false).await;
        let (bob, carl) = (Keys::generate(), Keys::generate());
        let (g, key) = handed_over(&rt, &bob, &carl).await;
        let base = now() - 300;
        let c1 = say_in_group(&rt, &g, &key, &carl, &me, "c1", base).await;
        say_in_group(&rt, &g, &key, &bob, &me, "b1", base + 1).await;
        let m1 = say_in_group(&rt, &g, &key, &me, &me, "m1", base + 2).await;
        let c2 = say_in_group(&rt, &g, &key, &carl, &me, "c2", base + 3).await;
        say_in_group(&rt, &g, &key, &carl, &me, "c3", base + 4).await;
        let fake = Fake::default();
        let r = rt
            .report_send(
                ReportInput {
                    message_id: Some(c2.clone()),
                    ..input(&carl, ReportContext::Group { group_id: g.id.clone() })
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!((r.attached, r.proved), (3, 3));
        let report = fake.last();
        assert_eq!(ids(&report), vec![c1, m1, c2.clone()], "Bob's word is not the report's");
        assert_eq!(report.target, messenger_evidence::Target::Person);
        assert_eq!(report.accused, carl.public_key().to_hex());
        assert_eq!(report.group_name.as_deref(), Some("Team"));
        assert_eq!(report.message_id.as_deref(), Some(c2.as_str()));
        assert!(report.group_admins.is_empty());

        // Without the messages, the message it was opened on is still told.
        rt.report_send(
            ReportInput {
                attach: false,
                message_id: Some(c2.clone()),
                ..input(&carl, ReportContext::Group { group_id: g.id.clone() })
            },
            &fake,
        )
        .await
        .unwrap();
        assert!(fake.last().messages.is_empty());
        assert_eq!(fake.last().message_id.as_deref(), Some(c2.as_str()));
        rt.shutdown().await;
    }

    /// A report from a channel names the channel and its post, and takes
    /// nothing of a chat: no messages, no block, no message of a chat.
    #[tokio::test]
    async fn a_report_from_a_channel_carries_the_channel_and_its_post() {
        let (_dir, rt, me) = started(false).await;
        let bob = Keys::generate();
        let fake = Fake::default();
        let (community_id, post_id) = ("c".repeat(64), "d".repeat(64));
        let context = ReportContext::Channel { community_id: community_id.clone(), post_id: Some(post_id.clone()) };
        let r = rt
            .report_send(ReportInput { block: true, message_id: Some(post_id.clone()), ..input(&bob, context) }, &fake)
            .await
            .unwrap();
        assert_eq!(r, ReportResult { status: ReportStatus::Sent, attached: 0, proved: 0, blocked: false });
        let sent = fake.last();
        assert_eq!(sent.context, Context::Channel { community_id: community_id.clone(), post_id: Some(post_id) });
        assert_eq!((sent.message_id, sent.accused), (None, bob.public_key().to_hex()));
        assert!(!rt.dm().relation(&pk_of(&bob)).await.unwrap().blocked);
        // A channel that is no id of a channel: the check of the report refuses it.
        let bad = ReportContext::Channel { community_id: "x".into(), post_id: None };
        let r = rt.report_send(input(&bob, bad), &fake).await;
        assert!(matches!(r, Err(MessengerError::Invalid(ref w)) if w == "report_channel_id"), "{r:?}");
        // Its owner reports a channel of their own (Google Play, 2026-10-22);
        // on a post of their own, never.
        let own = ReportContext::Channel { community_id: community_id.clone(), post_id: None };
        let r = rt.report_send(ReportInput { accused: me.public_key().to_hex(), ..input(&bob, own) }, &fake).await.unwrap();
        assert_eq!(r.status, ReportStatus::Sent);
        assert_eq!(fake.last().accused, me.public_key().to_hex());
        let mine = ReportContext::Channel { community_id: community_id.clone(), post_id: Some("d".repeat(64)) };
        let r = rt.report_send(ReportInput { accused: me.public_key().to_hex(), ..input(&bob, mine) }, &fake).await;
        assert!(matches!(r, Err(MessengerError::Invalid(ref w)) if w == "report_self"), "{r:?}");
        rt.shutdown().await;
    }

    /// A group is reported in a group only.
    #[tokio::test]
    async fn a_group_is_reported_in_a_group_only() {
        let (_dir, rt, _me) = started(false).await;
        let bob = Keys::generate();
        let fake = Fake::default();
        for context in [ReportContext::Dm, ReportContext::Request] {
            let r = rt
                .report_send(
                    ReportInput { target: Some(ReportTarget::Group), ..input(&bob, context) },
                    &fake,
                )
                .await;
            assert!(matches!(r, Err(MessengerError::Invalid(ref w)) if w == "report_target"), "{r:?}");
        }
        assert!(fake.got.lock().unwrap().is_empty());
        rt.shutdown().await;
    }

    /// A report from a private chat is of that chat: by the message it was
    /// opened on (the page may pass the bare key) or by the address of the
    /// chat; its messages go with their proofs, and the block is the
    /// chat's alone — the home chat with the same person stays open.
    #[tokio::test]
    async fn a_report_from_a_private_chat_is_of_that_chat() {
        let (_dir, rt, me) = started(false).await;
        let s1 = rt
            .server_put_private(&crate::private::PrivateServerInput {
                relay_url: "wss://s1.example".into(),
                name: Some("S1".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let bob = Keys::generate();
        let bob_hex = bob.public_key().to_hex();
        let on_s1 = messenger_dm::DmPeer::on(pk_of(&me), Some(&s1));
        let mut sent = Vec::new();
        for (i, text) in ["first on s1", "second on s1"].into_iter().enumerate() {
            let w = messenger_dm::wrap::wrap(
                &bob,
                on_s1.clone(),
                &Envelope::text(text).encode(),
                now() - 60 + i as i64,
                None,
            )
            .unwrap();
            receive(&rt, &me, &w.to_peer).await;
            sent.push(w.rumor_id.as_hex().to_string());
        }
        let private_chat = format!("dm:{bob_hex}@{s1}");
        assert_eq!(
            msgs::count_in_chat(rt.store(), &private_chat).await.unwrap(),
            2
        );
        let home_chat = chats::dm_chat_id(&bob_hex, None);
        assert_eq!(msgs::count_in_chat(rt.store(), &home_chat).await.unwrap(), 0);

        // The bare key and a message of the private chat.
        let fake = Fake::default();
        let r = rt
            .report_send(
                ReportInput {
                    message_id: Some(sent[1].clone()),
                    ..input(&bob, ReportContext::Dm)
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!((r.attached, r.proved), (2, 2));
        assert_eq!(ids(&fake.last()), sent);
        assert_eq!(fake.last().accused, bob_hex, "the accused is the key");

        // The address of the chat, with a block: the block is the chat's.
        let r = rt
            .report_send(
                ReportInput {
                    accused: format!("{bob_hex}@{s1}"),
                    block: true,
                    ..input(&bob, ReportContext::Dm)
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!((r.attached, r.blocked), (2, true));
        assert!(
            rt.dm_relation(&format!("{bob_hex}@{s1}"))
                .await
                .unwrap()
                .blocked
        );
        assert!(
            !rt.dm_relation(&bob_hex).await.unwrap().blocked,
            "the home chat with him is not blocked"
        );
        assert!(rt.dm_blocked().await.unwrap().is_empty());

        // A server this device has not: refused.
        let err = rt
            .report_send(
                ReportInput {
                    accused: format!("{bob_hex}@{}", "f".repeat(16)),
                    ..input(&bob, ReportContext::Dm)
                },
                &fake,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("server_unknown"), "{err}");
        rt.shutdown().await;
    }

    /// A stranger blocked with the report: what he wrote is in the report,
    /// gathered before the block took the chat away.
    #[tokio::test]
    async fn a_block_comes_after_the_evidence_is_gathered() {
        let (_dir, rt, me) = started(true).await;
        let spammer = Keys::generate();
        let first = from_peer(&rt, &me, &spammer, "cheap coins here").await;
        let chat = chats::dm_chat_id(&spammer.public_key().to_hex(), None);
        assert!(
            msgs::count_in_chat(rt.store(), &chat).await.unwrap() >= 1,
            "a request"
        );

        let fake = Fake::default();
        let r = rt
            .report_send(
                ReportInput {
                    reason: ReportReason::Spam,
                    block: true,
                    ..input(&spammer, ReportContext::Request)
                },
                &fake,
            )
            .await
            .unwrap();
        assert_eq!(
            r,
            ReportResult {
                status: ReportStatus::Sent,
                attached: 1,
                proved: 1,
                blocked: true
            }
        );
        assert_eq!(ids(&fake.last()), vec![first]);
        assert_eq!(fake.last().context, Context::Request);
        assert!(rt.dm().relation(&pk_of(&spammer)).await.unwrap().blocked);
        assert_eq!(
            msgs::count_in_chat(rt.store(), &chat).await.unwrap(),
            0,
            "the chat went with the block"
        );
        rt.shutdown().await;
    }

    /// Three a minute, then `rate_limited`; what is refused before the
    /// count (a malformed input) does not count.
    #[tokio::test]
    async fn more_than_three_a_minute_are_refused() {
        let (_dir, rt, _me) = started(false).await;
        let bob = Keys::generate();
        let fake = Fake::default();
        let quiet = || ReportInput {
            attach: false,
            ..input(&bob, ReportContext::Dm)
        };
        let bad = ReportInput {
            sub_reason: Some(ReportSubReason::Grooming),
            ..quiet()
        };
        assert!(
            matches!(rt.report_send(bad, &fake).await, Err(MessengerError::Invalid(ref w)) if w == "report_sub_reason")
        );
        for _ in 0..REPORTS_PER_MINUTE {
            rt.report_send(quiet(), &fake).await.unwrap();
        }
        assert!(
            matches!(rt.report_send(quiet(), &fake).await, Err(MessengerError::Invalid(ref w)) if w == "rate_limited")
        );
        assert_eq!(
            fake.got.lock().unwrap().len(),
            REPORTS_PER_MINUTE,
            "the fourth never left"
        );

        let reports = Reports::default();
        let t = Instant::now();
        for _ in 0..REPORTS_PER_MINUTE {
            reports.admit(t).unwrap();
        }
        assert!(reports.admit(t + Duration::from_secs(59)).is_err());
        reports.admit(t + Duration::from_secs(60)).unwrap();
        rt.shutdown().await;
    }

    /// The input is checked before anything is gathered or sent.
    #[tokio::test]
    async fn a_malformed_report_never_leaves() {
        let (_dir, rt, me) = started(false).await;
        let bob = Keys::generate();
        let fake = Fake::default();
        let quiet = || ReportInput {
            attach: false,
            ..input(&bob, ReportContext::Dm)
        };
        for (bad, word) in [
            (
                ReportInput {
                    accused: "nobody".into(),
                    ..quiet()
                },
                "report_accused",
            ),
            (
                ReportInput {
                    accused: me.public_key().to_hex(),
                    ..quiet()
                },
                "report_self",
            ),
            (
                ReportInput {
                    comment: Some("x".repeat(MAX_COMMENT_CHARS + 1)),
                    ..quiet()
                },
                "report_comment",
            ),
            (
                ReportInput {
                    attach: true,
                    message_id: Some("ab".repeat(32)),
                    ..quiet()
                },
                "report_message",
            ),
        ] {
            assert!(
                matches!(rt.report_send(bad, &fake).await, Err(MessengerError::Invalid(ref w)) if w == word),
                "{word}"
            );
        }
        assert!(fake.got.lock().unwrap().is_empty());
        let child = ReportInput {
            reason: ReportReason::ChildHarm,
            sub_reason: Some(ReportSubReason::ViolenceIllegal),
            ..quiet()
        };
        rt.report_send(child, &fake).await.unwrap();
        assert_eq!(fake.last().sub_reason.as_deref(), Some("violence_illegal"));
        rt.shutdown().await;
    }

    /// A refusal of the server is its word, and nothing is kept or
    /// blocked; a failure on the way keeps the report, which goes at the
    /// next due try and is told to the page.
    #[tokio::test]
    async fn a_report_that_did_not_get_there_waits_and_goes_later() {
        let (_dir, rt, _me) = started(false).await;
        let bob = Keys::generate();
        let quiet = || ReportInput {
            attach: false,
            block: true,
            ..input(&bob, ReportContext::Dm)
        };
        let refused = Fake::answering([Delivery::Refused {
            error: "too_many_messages".into(),
        }]);
        assert!(
            matches!(rt.report_send(quiet(), &refused).await, Err(MessengerError::Invalid(ref w)) if w == "too_many_messages")
        );
        assert!(
            !rt.dm().relation(&pk_of(&bob)).await.unwrap().blocked,
            "refused: not blocked"
        );
        let no_way = Fake::answering([Delivery::NoWay {
            why: "no_host".into(),
        }]);
        assert!(
            matches!(rt.report_send(quiet(), &no_way).await, Err(MessengerError::Invalid(ref w)) if w == "no_host")
        );
        assert_eq!(
            rows::count_queued(rt.store()).await.unwrap(),
            0,
            "neither is kept"
        );

        let down = Fake::answering([Delivery::Failed {
            why: "transport".into(),
        }]);
        let r = rt.report_send(quiet(), &down).await.unwrap();
        assert_eq!(
            (r.status, r.blocked),
            (ReportStatus::Queued, true),
            "kept, and the block goes on"
        );
        assert_eq!(rows::count_queued(rt.store()).await.unwrap(), 1);

        let mut events = rt.ui_events();
        let t = now();
        let later = Fake::answering([
            Delivery::Failed {
                why: "status 503".into(),
            },
            Delivery::Accepted {
                id: Some("srv-1".into()),
            },
        ]);
        assert_eq!(rt.reports_retry(&later, t).await.unwrap(), 0, "not due yet");
        assert!(later.got.lock().unwrap().is_empty());
        assert_eq!(
            rt.reports_retry(&later, t + FIRST_RETRY_SECS)
                .await
                .unwrap(),
            0,
            "the server is down still"
        );
        let row = rows::due(rt.store(), i64::MAX, 10).await.unwrap().remove(0);
        assert_eq!(
            (row.attempts, row.next_retry_at - (t + FIRST_RETRY_SECS)),
            (2, 2 * FIRST_RETRY_SECS),
            "the pause doubles"
        );
        assert_eq!(
            rt.reports_retry(&later, row.next_retry_at).await.unwrap(),
            1
        );
        assert_eq!(later.got.lock().unwrap().len(), 2);
        assert_eq!(
            later.got.lock().unwrap()[1],
            down.last(),
            "the same report, as it was made"
        );
        let told = loop {
            let ev = events.try_recv().expect("the page was told");
            if ev.name == UI_EVENT_MODERATION_REPORT {
                break ev.payload;
            }
        };
        assert_eq!(told, serde_json::json!({ "id": "srv-1", "status": "sent" }));
        assert_eq!(rows::count_queued(rt.store()).await.unwrap(), 0);
        assert_eq!(
            rows::get(rt.store(), &row.id)
                .await
                .unwrap()
                .unwrap()
                .report_json,
            "",
            "nothing of it stays"
        );
        rt.shutdown().await;
    }

    /// Seven days without getting there: given up, and the page told.
    #[tokio::test]
    async fn a_report_is_given_up_after_seven_days() {
        let (_dir, rt, _me) = started(false).await;
        let t = now();
        rows::queue(
            rt.store(),
            "old",
            "{}",
            t - RETRY_FOR_SECS - 1,
            30,
            t,
            Some("transport"),
        )
        .await
        .unwrap();
        let mut events = rt.ui_events();
        let fake = Fake::default();
        assert_eq!(rt.reports_retry(&fake, t).await.unwrap(), 0);
        assert!(fake.got.lock().unwrap().is_empty(), "not tried");
        assert_eq!(
            rows::get(rt.store(), "old").await.unwrap().unwrap().state,
            rows::STATE_EXPIRED
        );
        let ev = events.try_recv().unwrap();
        assert_eq!(
            (ev.name.as_str(), ev.payload),
            (
                UI_EVENT_MODERATION_REPORT,
                serde_json::json!({ "status": "expired" })
            )
        );
        rt.reports_retry(&fake, t + KEEP_DONE_SECS + 1)
            .await
            .unwrap();
        assert!(
            rows::get(rt.store(), "old").await.unwrap().is_none(),
            "forgotten a month later"
        );
        rt.shutdown().await;
    }

    #[test]
    fn the_pause_grows_to_six_hours() {
        assert_eq!((backoff(1), backoff(2), backoff(3)), (60, 120, 240));
        assert_eq!((backoff(9), backoff(10)), (15_360, LAST_RETRY_SECS));
        assert_eq!(backoff(1_000), LAST_RETRY_SECS);
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A report of a bug to the project (internal/messenger-spec.md,
//! «Сообщение об ошибке»): the user's words, the product, its version, the
//! system and the language, the versions of the modules, and, when the
//! user attaches it, the diagnosis of the network (`net::NetDiagnosis`).
//!
//! It goes the way a report of a person goes (`crate::report`): the host's
//! [`ReportSender`] at the path of bugs (`POST /cloud/v1/bug`, signed by the
//! identity, NIP-98), checked here by the limits the server checks it with.
//! One that does not get there waits in `msg_bugs_outbox` as it was made,
//! byte for byte (the server knows a body sent again by its reporter and
//! `created_at`, and answers the same id), and is tried again
//! ([`MessengerRuntime::bugs_retry`], called by the host's timer) with the
//! pauses of the reports, for seven days; its delivery is told to the page
//! by `bug.report`. With the user's own servers there is no way to the
//! project: a fresh report is refused (`no_host`) rather than kept.

use crate::report::{
    self, backoff, local_id, Delivery, ReportSender, KEEP_DONE_SECS, RETRY_BATCH, RETRY_FOR_SECS,
};
use crate::MessengerRuntime;
use messenger_core::traits::{SystemClock, UiEvent};
use messenger_core::{Clock, MessengerError, Result};
use messenger_store::bugs as rows;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use ts_rs::TS;

/// The event that tells the page what became of a report of a bug that
/// waited: `{id?, status}` — `sent` (with the server's `id`), `refused`
/// (with the server's word in `error`) or `expired` (seven days without
/// getting there).
pub const UI_EVENT_BUG_REPORT: &str = "bug.report";

/// The version of the body, its `v`.
pub const BUG_VERSION: u8 = 1;
/// The user's words, trimmed, at most this many characters (`bug_text`).
pub const MAX_TEXT_CHARS: usize = 8000;
/// The limits of the other fields (`bug_field`).
pub const MAX_PRODUCT_CHARS: usize = 32;
pub const MAX_VERSION_CHARS: usize = 32;
pub const MAX_OS_CHARS: usize = 64;
pub const MAX_LOCALE_CHARS: usize = 16;
pub const MAX_MODULES: usize = 16;
pub const MAX_MODULE_CHARS: usize = 200;
/// The whole body, at most: past it the diagnosis is cut, then left out.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

// ─── The contract with the page ─────────────────────────────────────────────

/// A report of a bug as `messenger_bug_send` takes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BugInput {
    /// What happened, in the user's words: 1 to 8000 characters once
    /// trimmed.
    pub text: String,
    /// The product (`space`, `chat`…): 1 to 32 of `a-z`, `0-9`, `-`.
    pub product: String,
    /// Its version, at most 32 characters.
    pub version: String,
    /// The system and the machine (`android/aarch64`), at most 64.
    pub os: String,
    /// The language of the app (`ru`), at most 16.
    pub locale: String,
    /// The versions of the modules (`Camoufox: 135.0`): at most 16 lines
    /// of at most 200 characters.
    #[serde(default)]
    pub modules: Vec<String>,
    /// The diagnosis of the network, as `messenger_net_diagnose` gave it,
    /// when the user attaches it.
    #[serde(default)]
    #[ts(optional = nullable, type = "NetDiagnosis")]
    pub net: Option<serde_json::Value>,
}

/// Whether the report reached the server now or waits on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BugStatus {
    Sent,
    Queued,
}

/// What `messenger_bug_send` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BugResult {
    pub status: BugStatus,
    /// The server's id of the report (12 hex), when it was sent now.
    pub id: Option<String>,
}

// ─── The body ───────────────────────────────────────────────────────────────

/// The body of `POST /cloud/v1/bug`, in the order of the wire.
#[derive(Serialize)]
struct Body<'a> {
    v: u8,
    reporter: &'a str,
    created_at: i64,
    product: &'a str,
    version: &'a str,
    os: &'a str,
    locale: &'a str,
    text: &'a str,
    modules: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    net: Option<&'a serde_json::Value>,
}

fn invalid(word: &str) -> MessengerError {
    MessengerError::Invalid(word.to_string())
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// The input checked by the limits of the server: the text trimmed, or the
/// word of the refusal.
fn checked(input: &BugInput) -> Result<String> {
    let text = input.text.trim();
    if text.is_empty() || chars(text) > MAX_TEXT_CHARS {
        return Err(invalid("bug_text"));
    }
    let product_ok = (1..=MAX_PRODUCT_CHARS).contains(&input.product.len())
        && input
            .product
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
    let fields_ok = product_ok
        && chars(&input.version) <= MAX_VERSION_CHARS
        && chars(&input.os) <= MAX_OS_CHARS
        && chars(&input.locale) <= MAX_LOCALE_CHARS
        && input.modules.len() <= MAX_MODULES
        && input.modules.iter().all(|m| chars(m) <= MAX_MODULE_CHARS)
        && input.net.as_ref().is_none_or(|n| n.is_object());
    if !fields_ok {
        return Err(invalid("bug_field"));
    }
    Ok(text.to_string())
}

/// The body of `input` by `reporter` at `created_at`, at most
/// [`MAX_BODY_BYTES`]: when the diagnosis makes it larger, the `error`
/// words of its lists go first, the longest list first, then the diagnosis
/// itself. The user's words are never cut: a body too large without any
/// diagnosis is refused (`payload_too_large`, the server's word for it).
fn body_of(input: &BugInput, text: &str, reporter: &str, created_at: i64) -> Result<Vec<u8>> {
    let make = |net: Option<&serde_json::Value>| {
        serde_json::to_vec(&Body {
            v: BUG_VERSION,
            reporter,
            created_at,
            product: &input.product,
            version: &input.version,
            os: &input.os,
            locale: &input.locale,
            text,
            modules: &input.modules,
            net,
        })
    };
    let mut net = input.net.clone();
    let body = make(net.as_ref())?;
    if body.len() <= MAX_BODY_BYTES {
        return Ok(body);
    }
    let mut lists: Vec<(usize, String)> = match &net {
        Some(serde_json::Value::Object(diagnosis)) => diagnosis
            .iter()
            .filter_map(|(k, v)| v.as_array().map(|a| (a.len(), k.clone())))
            .collect(),
        _ => Vec::new(),
    };
    // The longest first; of two as long, by name, so that the same
    // diagnosis is cut the same way.
    lists.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (_, key) in lists {
        if let Some(serde_json::Value::Array(items)) = net.as_mut().and_then(|n| n.get_mut(&key)) {
            for item in items.iter_mut() {
                if let Some(fields) = item.as_object_mut() {
                    fields.remove("error");
                }
            }
        }
        let body = make(net.as_ref())?;
        if body.len() <= MAX_BODY_BYTES {
            return Ok(body);
        }
    }
    let body = make(None)?;
    if body.len() <= MAX_BODY_BYTES {
        return Ok(body);
    }
    Err(invalid("payload_too_large"))
}

impl MessengerRuntime {
    /// Report a bug: check it, make the body with the identity, send it now
    /// (or keep it to send later). A refusal of the server comes back as
    /// its word, `no_host` when there is no way to the project (the user's
    /// own servers), `rate_limited` past three a minute, `bug_text` or
    /// `bug_field` of a malformed input; nothing is kept then.
    pub async fn bug_send(&self, input: BugInput, sender: &dyn ReportSender) -> Result<BugResult> {
        let text = checked(&input)?;
        let keys = self.session_keys().await?;
        let now = SystemClock.now().secs();
        let body = body_of(&input, &text, &keys.public_key().to_hex(), now)?;
        // Counted before the way is tried: a loop is stopped before it
        // reaches the server.
        self.bugs.admit(Instant::now())?;
        match sender.send(&keys, &body).await {
            Delivery::Accepted { id } => Ok(BugResult {
                status: BugStatus::Sent,
                id,
            }),
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
                Ok(BugResult {
                    status: BugStatus::Queued,
                    id: None,
                })
            }
            Delivery::Refused { error } => Err(MessengerError::Invalid(error)),
            Delivery::NoWay { why } => Err(MessengerError::Invalid(why)),
        }
    }

    /// Try the reports of a bug that wait and are due by `now`; the host
    /// calls it on a timer. Each one sent, refused or given up is told to
    /// the page (`bug.report`). Answers how many were sent. A report that
    /// finds no way (the user's own servers now) waits on, until its seven
    /// days are over.
    pub async fn bugs_retry(&self, sender: &dyn ReportSender, now: i64) -> Result<u32> {
        let _one_round = self.bugs.one_round().await;
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
                self.tell_bug(serde_json::json!({ "status": rows::STATE_EXPIRED }));
                continue;
            }
            match sender.send(&keys, row.body_json.as_bytes()).await {
                Delivery::Accepted { id } => {
                    rows::finish(self.store(), &row.id, rows::STATE_SENT, None, now).await?;
                    let mut told = serde_json::json!({ "status": rows::STATE_SENT });
                    if let Some(id) = id {
                        told["id"] = serde_json::Value::String(id);
                    }
                    self.tell_bug(told);
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
                    self.tell_bug(
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

    fn tell_bug(&self, payload: serde_json::Value) {
        let _ = self.ui.send(UiEvent {
            name: UI_EVENT_BUG_REPORT.into(),
            payload,
        });
    }
}

/// The rate and the rounds of the reports of a bug: those of the reports
/// of a person, kept apart, so that one kind does not use up the other's.
pub(crate) type Bugs = report::Reports;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{FIRST_RETRY_SECS, REPORTS_PER_MINUTE};
    use async_trait::async_trait;
    use messenger_core::MessengerConfig;
    use messenger_testkit::MemorySecretStore;
    use nostr::key::Keys;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    async fn started() -> (tempfile::TempDir, MessengerRuntime, Keys) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked()))
            .await
            .unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        crate::testing::log_in(&rt).await;
        assert!(rt.refresh_signer().await.unwrap());
        let keys = rt.session_keys().await.unwrap();
        (dir, rt, keys)
    }

    fn now() -> i64 {
        SystemClock.now().secs()
    }

    /// A sender that answers what it is told to and keeps the bodies it got.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<VecDeque<Delivery>>,
        got: Mutex<Vec<Vec<u8>>>,
    }

    impl Fake {
        fn answering(answers: impl IntoIterator<Item = Delivery>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                got: Mutex::default(),
            }
        }

        fn got(&self) -> Vec<Vec<u8>> {
            self.got.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ReportSender for Fake {
        async fn send(&self, keys: &Keys, body: &[u8]) -> Delivery {
            let json: serde_json::Value = serde_json::from_slice(body).expect("the body is JSON");
            assert_eq!(
                json["reporter"].as_str(),
                Some(keys.public_key().to_hex().as_str()),
                "signed by whoever reports"
            );
            self.got.lock().unwrap().push(body.to_vec());
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Delivery::Accepted {
                    id: Some("0123456789ab".into()),
                })
        }
    }

    fn input() -> BugInput {
        BugInput {
            text: "  the list of chats stays empty  ".into(),
            product: "space".into(),
            version: "5.2.0".into(),
            os: "android/aarch64".into(),
            locale: "ru".into(),
            modules: vec!["Camoufox: 135.0".into()],
            net: None,
        }
    }

    fn event(events: &mut tokio::sync::broadcast::Receiver<UiEvent>) -> serde_json::Value {
        loop {
            let ev = events.try_recv().expect("the page was told");
            if ev.name == UI_EVENT_BUG_REPORT {
                break ev.payload;
            }
        }
    }

    /// Sent at once: the server's id comes back, and the body is that of
    /// the wire, in its order, the text trimmed.
    #[tokio::test]
    async fn a_bug_goes_at_once_with_the_body_of_the_wire() {
        let (_dir, rt, me) = started().await;
        let fake = Fake::default();
        let before = now();
        let r = rt.bug_send(input(), &fake).await.unwrap();
        assert_eq!(
            r,
            BugResult {
                status: BugStatus::Sent,
                id: Some("0123456789ab".into())
            }
        );
        let body = String::from_utf8(fake.got().remove(0)).unwrap();
        let at: serde_json::Value = serde_json::from_str(&body).unwrap();
        let created_at = at["created_at"].as_i64().unwrap();
        assert!((before..=now()).contains(&created_at));
        assert_eq!(
            body,
            format!(
                r#"{{"v":1,"reporter":"{}","created_at":{created_at},"product":"space","version":"5.2.0","os":"android/aarch64","locale":"ru","text":"the list of chats stays empty","modules":["Camoufox: 135.0"]}}"#,
                me.public_key().to_hex()
            ),
            "no `net` when none is attached"
        );
        let with_net = BugInput {
            net: Some(serde_json::json!({ "v": 1, "direct": [] })),
            ..input()
        };
        rt.bug_send(with_net, &fake).await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&fake.got()[1]).unwrap();
        assert_eq!(sent["net"], serde_json::json!({ "v": 1, "direct": [] }));
        assert_eq!(rows::count_queued(rt.store()).await.unwrap(), 0);
        rt.shutdown().await;
    }

    /// The limits of the server, checked before anything leaves.
    #[tokio::test]
    async fn a_malformed_bug_never_leaves() {
        let (_dir, rt, _me) = started().await;
        let fake = Fake::default();
        for (bad, word) in [
            (
                BugInput {
                    text: "   ".into(),
                    ..input()
                },
                "bug_text",
            ),
            (
                BugInput {
                    text: "я".repeat(MAX_TEXT_CHARS + 1),
                    ..input()
                },
                "bug_text",
            ),
            (
                BugInput {
                    product: "".into(),
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    product: "Space".into(),
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    product: "x".repeat(MAX_PRODUCT_CHARS + 1),
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    version: "1".repeat(MAX_VERSION_CHARS + 1),
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    os: "o".repeat(MAX_OS_CHARS + 1),
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    locale: "l".repeat(MAX_LOCALE_CHARS + 1),
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    modules: vec!["m".into(); MAX_MODULES + 1],
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    modules: vec!["m".repeat(MAX_MODULE_CHARS + 1)],
                    ..input()
                },
                "bug_field",
            ),
            (
                BugInput {
                    net: Some(serde_json::json!([1, 2])),
                    ..input()
                },
                "bug_field",
            ),
        ] {
            assert!(
                matches!(rt.bug_send(bad, &fake).await, Err(MessengerError::Invalid(ref w)) if w == word),
                "{word}"
            );
        }
        assert!(fake.got().is_empty());
        // At the limits exactly: it goes.
        let full = BugInput {
            text: "я".repeat(MAX_TEXT_CHARS),
            product: "a-1".into(),
            modules: vec!["m".repeat(MAX_MODULE_CHARS); MAX_MODULES],
            ..input()
        };
        rt.bug_send(full, &fake).await.unwrap();
        rt.shutdown().await;
    }

    /// Three a minute, then `rate_limited`; a malformed one does not count;
    /// the reports of a person keep a count of their own.
    #[tokio::test]
    async fn more_than_three_a_minute_are_refused() {
        let (_dir, rt, _me) = started().await;
        let fake = Fake::default();
        assert!(rt
            .bug_send(
                BugInput {
                    text: "".into(),
                    ..input()
                },
                &fake
            )
            .await
            .is_err());
        for _ in 0..REPORTS_PER_MINUTE {
            rt.bug_send(input(), &fake).await.unwrap();
        }
        assert!(
            matches!(rt.bug_send(input(), &fake).await, Err(MessengerError::Invalid(ref w)) if w == "rate_limited")
        );
        assert_eq!(
            fake.got().len(),
            REPORTS_PER_MINUTE,
            "the fourth never left"
        );
        rt.reports
            .admit(Instant::now())
            .expect("the reports of a person are counted apart");
        rt.shutdown().await;
    }

    /// A refusal is the server's word and nothing is kept; no way (the
    /// user's own servers) is `no_host`, not kept either; a failure on the
    /// way keeps the body, which goes at the next due try byte for byte, and
    /// the page is told.
    #[tokio::test]
    async fn a_bug_that_did_not_get_there_waits_and_goes_later_as_it_was() {
        let (_dir, rt, me) = started().await;
        let refused = Fake::answering([Delivery::Refused {
            error: "bug_text".into(),
        }]);
        assert!(
            matches!(rt.bug_send(input(), &refused).await, Err(MessengerError::Invalid(ref w)) if w == "bug_text")
        );
        let no_way = Fake::answering([Delivery::NoWay {
            why: "no_host".into(),
        }]);
        assert!(
            matches!(rt.bug_send(input(), &no_way).await, Err(MessengerError::Invalid(ref w)) if w == "no_host")
        );
        assert_eq!(
            rows::count_queued(rt.store()).await.unwrap(),
            0,
            "neither is kept"
        );

        let down = Fake::answering([Delivery::Failed {
            why: "transport".into(),
        }]);
        let r = rt.bug_send(input(), &down).await.unwrap();
        assert_eq!(
            r,
            BugResult {
                status: BugStatus::Queued,
                id: None
            }
        );
        assert_eq!(rows::count_queued(rt.store()).await.unwrap(), 1);

        let mut events = rt.ui_events();
        let t = now();
        let later = Fake::answering([
            Delivery::Failed {
                why: "status 503".into(),
            },
            Delivery::NoWay {
                why: "no_host".into(),
            },
            Delivery::Accepted {
                id: Some("abcdef012345".into()),
            },
        ]);
        assert_eq!(rt.bugs_retry(&later, t).await.unwrap(), 0, "not due yet");
        assert!(later.got().is_empty());
        assert_eq!(
            rt.bugs_retry(&later, t + FIRST_RETRY_SECS).await.unwrap(),
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
            rt.bugs_retry(&later, row.next_retry_at).await.unwrap(),
            0,
            "no way now: it waits on"
        );
        let row = rows::due(rt.store(), i64::MAX, 10).await.unwrap().remove(0);
        assert_eq!(row.attempts, 3);
        assert_eq!(rt.bugs_retry(&later, row.next_retry_at).await.unwrap(), 1);
        let got = later.got();
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], down.got()[0], "the same bytes as it was made");
        assert_eq!(
            event(&mut events),
            serde_json::json!({ "id": "abcdef012345", "status": "sent" })
        );
        assert_eq!(rows::count_queued(rt.store()).await.unwrap(), 0);
        assert_eq!(
            rows::get(rt.store(), &row.id)
                .await
                .unwrap()
                .unwrap()
                .body_json,
            "",
            "nothing of it stays"
        );

        // A refusal at a retry: done, and the page told with the word. (Kept
        // by hand: three were sent this minute already.)
        let kept = serde_json::json!({ "reporter": me.public_key().to_hex() }).to_string();
        rows::queue(
            rt.store(),
            "later",
            &kept,
            now(),
            1,
            now(),
            Some("transport"),
        )
        .await
        .unwrap();
        let refused = Fake::answering([Delivery::Refused {
            error: "bug_field".into(),
        }]);
        assert_eq!(
            rt.bugs_retry(&refused, now() + FIRST_RETRY_SECS)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            event(&mut events),
            serde_json::json!({ "status": "refused", "error": "bug_field" })
        );
        assert_eq!(rows::count_queued(rt.store()).await.unwrap(), 0);
        rt.shutdown().await;
    }

    /// Seven days without getting there: given up, and the page told; the
    /// row is forgotten a month later.
    #[tokio::test]
    async fn a_bug_is_given_up_after_seven_days() {
        let (_dir, rt, _me) = started().await;
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
        assert_eq!(rt.bugs_retry(&fake, t).await.unwrap(), 0);
        assert!(fake.got().is_empty(), "not tried");
        assert_eq!(
            rows::get(rt.store(), "old").await.unwrap().unwrap().state,
            rows::STATE_EXPIRED
        );
        assert_eq!(
            event(&mut events),
            serde_json::json!({ "status": "expired" })
        );
        rt.bugs_retry(&fake, t + KEEP_DONE_SECS + 1).await.unwrap();
        assert!(
            rows::get(rt.store(), "old").await.unwrap().is_none(),
            "forgotten a month later"
        );
        rt.shutdown().await;
    }

    /// A diagnosis that makes the body too large loses the `error` words
    /// of its longest list first, then those of the next, then itself; the
    /// user's words are never cut.
    #[test]
    fn a_large_diagnosis_is_cut_and_the_text_never() {
        let reporter = "ab".repeat(32);
        let item = |i: usize| serde_json::json!({ "host": format!("h{i}.example"), "ok": false, "error": "e".repeat(80) });
        // 600 × ~120 bytes of `direct` with their errors: past 64 KiB; without
        // them, ~40 bytes each: well within.
        let net = serde_json::json!({
            "v": 1,
            "direct": (0..600).map(item).collect::<Vec<_>>(),
            "bridges": (0..8).map(item).collect::<Vec<_>>(),
        });
        let big = BugInput {
            net: Some(net),
            ..input()
        };
        let text = checked(&big).unwrap();
        let body = body_of(&big, &text, &reporter, 1).unwrap();
        assert!(body.len() <= MAX_BODY_BYTES);
        let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            sent["net"]["direct"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| d.get("error").is_none()),
            "the longest list lost its words"
        );
        assert!(
            sent["net"]["bridges"][0].get("error").is_some(),
            "the shorter one kept them"
        );
        assert_eq!(
            sent["net"]["direct"].as_array().unwrap().len(),
            600,
            "nothing else of it went"
        );

        // Too large even without its words: the diagnosis goes, the text stays.
        let huge = serde_json::json!({ "v": 1, "note": "x".repeat(MAX_BODY_BYTES) });
        let big = BugInput {
            net: Some(huge),
            text: "я".repeat(MAX_TEXT_CHARS),
            ..input()
        };
        let text = checked(&big).unwrap();
        let body = body_of(&big, &text, &reporter, 1).unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(sent.get("net").is_none());
        assert_eq!(
            sent["text"].as_str().unwrap().chars().count(),
            MAX_TEXT_CHARS
        );

        // Within the limits of each field, too large as a whole: refused,
        // never cut.
        let escaped = BugInput {
            text: "\u{1}".repeat(MAX_TEXT_CHARS),
            modules: vec!["\u{1}".repeat(MAX_MODULE_CHARS); MAX_MODULES],
            ..input()
        };
        let text = checked(&escaped).unwrap();
        assert!(
            matches!(body_of(&escaped, &text, &reporter, 1), Err(MessengerError::Invalid(ref w)) if w == "payload_too_large")
        );
    }
}

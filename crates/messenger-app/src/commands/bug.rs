// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A report of a bug to the project, and the diagnosis of the network it
//! may carry: the commands of the page (`messenger_bug_send`,
//! `messenger_net_diagnose`; the types of
//! `ui/src/lib/messenger/generated/bug.ts` and `…/net.ts`). What a report
//! holds, how it is checked, kept and tried again, and how the network is
//! measured, is the runtime's (`messenger_runtime::bug`,
//! `messenger_runtime::net`); the way to the server is that of the reports
//! of a person ([`ProjectReports`] at [`BUG_PATH`]: `POST /cloud/v1/bug` on
//! a host of the project, NIP-98 signed, the next host on a failure of the
//! network, one signature again at the server's clock).
//!
//! A report that waits is tried again by [`spawn_retry`], a task of the
//! running messenger; its delivery reaches the page as the runtime event
//! `bug.report`.

use super::report::{unix_now, ProjectReports};
use super::{map_err, MessengerState, Task};
use crate::cloud_seams::ProjectHttp;
use messenger_runtime::bug::{BugInput, BugResult};
use messenger_runtime::net::NetDiagnosis;
use messenger_runtime::MessengerRuntime;
use std::sync::Arc;
use std::time::Duration;
use veydan_core::CmdResult;

/// Where a report of a bug goes on a host of the project.
pub const BUG_PATH: &str = "/cloud/v1/bug";

/// How often the reports of a bug that wait are looked at; each keeps its
/// own pause, as the reports of a person do.
const RETRY_TICK: Duration = Duration::from_secs(60);

/// Report a bug; the input and the answer are the contract of the page. A
/// refusal is a word: `bug_text`, `bug_field`, `payload_too_large`,
/// `rate_limited`, the word of the server, `no_host` with the user's own
/// servers in use.
#[tauri::command]
pub async fn messenger_bug_send(
    input: BugInput,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<BugResult> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let rt = messenger.runtime_unlocked().await?;
        let sender =
            ProjectReports::at(Arc::new(ProjectHttp::new(messenger.app.clone())), BUG_PATH);
        rt.bug_send(input, &sender).await.map_err(map_err)
    })
    .await
}

/// The diagnosis of the network, now (at most 45 s). With the user's own
/// servers nothing is asked and the lists are empty.
#[tauri::command]
pub async fn messenger_net_diagnose(
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<NetDiagnosis> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let rt = messenger.runtime_unlocked().await?;
        rt.net_diagnose().await.map_err(map_err)
    })
    .await
}

/// The task that tries the waiting reports of a bug again while the
/// messenger runs.
pub(super) fn spawn_retry(app: tauri::AppHandle, rt: Arc<MessengerRuntime>) -> Task {
    tauri::async_runtime::spawn(async move {
        let sender = ProjectReports::at(Arc::new(ProjectHttp::new(app)), BUG_PATH);
        loop {
            tokio::time::sleep(RETRY_TICK).await;
            if let Err(e) = rt.bugs_retry(&sender, unix_now() as i64).await {
                eprintln!("messenger bug: the waiting reports of a bug were not tried: {e}");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::report::tests::{messenger, signer_of, Answer, FakeHttp};
    use super::*;
    use messenger_runtime::bug::{BugStatus, UI_EVENT_BUG_REPORT};
    use std::sync::Mutex;
    use veydan_shell::seams::CloudReach;

    fn input() -> BugInput {
        BugInput {
            text: "the call does not ring".into(),
            product: "space".into(),
            version: "5.2.0".into(),
            os: "linux/x86_64".into(),
            locale: "ru".into(),
            modules: vec![],
            net: None,
        }
    }

    /// The server down: the report is kept. It goes at the next due try to
    /// the path of bugs, signed by the identity, the body as it was made;
    /// the page is told with the server's id. Sent at once, the id comes
    /// back in the answer.
    #[tokio::test]
    async fn a_bug_that_failed_is_kept_and_delivered_by_the_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (rt, me) = messenger(dir.path()).await;
        let down = FakeHttp::new(
            &["https://a.example", "https://b.example"],
            [
                Answer::Down,
                Answer::Status(
                    503,
                    r#"{"error":{"code":"unavailable","message":"…"}}"#.into(),
                ),
            ],
        );
        let r = rt
            .bug_send(input(), &ProjectReports::at(down.clone(), BUG_PATH))
            .await
            .unwrap();
        assert_eq!((r.status, r.id), (BugStatus::Queued, None));
        let tried: Vec<String> = down.got().iter().map(|r| r.url.clone()).collect();
        assert_eq!(
            tried,
            vec![
                "https://a.example/cloud/v1/bug",
                "https://b.example/cloud/v1/bug"
            ]
        );

        let mut events = rt.ui_events();
        let up = FakeHttp::new(
            &["https://a.example"],
            [Answer::Status(202, r#"{"id":"0a1b2c3d4e5f"}"#.into())],
        );
        let later = unix_now() as i64 + 2 * 60;
        assert_eq!(
            rt.bugs_retry(&ProjectReports::at(up.clone(), BUG_PATH), later)
                .await
                .unwrap(),
            1
        );
        let got = up.got();
        assert_eq!(
            (got[0].method.as_str(), got[0].url.as_str()),
            ("POST", "https://a.example/cloud/v1/bug")
        );
        assert_eq!(signer_of(&got[0]), me.public_key().to_hex());
        assert_eq!(got[0].body, down.got()[0].body, "the same bytes");
        let told = loop {
            let ev = events.try_recv().expect("the page is told");
            if ev.name == UI_EVENT_BUG_REPORT {
                break ev.payload;
            }
        };
        assert_eq!(
            told,
            serde_json::json!({ "id": "0a1b2c3d4e5f", "status": "sent" })
        );

        let now = FakeHttp::new(
            &["https://a.example"],
            [Answer::Status(202, r#"{"id":"ffffffffffff"}"#.into())],
        );
        let r = rt
            .bug_send(input(), &ProjectReports::at(now.clone(), BUG_PATH))
            .await
            .unwrap();
        assert_eq!(
            (r.status, r.id.as_deref()),
            (BugStatus::Sent, Some("ffffffffffff"))
        );
        rt.shutdown().await;
    }

    /// A refusal is the server's word; the user's own servers are no way to
    /// the project, and nothing is sent or kept.
    #[tokio::test]
    async fn a_refusal_is_the_servers_word_and_own_servers_send_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (rt, _me) = messenger(dir.path()).await;
        let refused = FakeHttp::new(
            &["https://a.example", "https://b.example"],
            [Answer::Status(
                400,
                r#"{"error":{"code":"bug_field","message":"…"}}"#.into(),
            )],
        );
        let e = rt
            .bug_send(input(), &ProjectReports::at(refused.clone(), BUG_PATH))
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "invalid input: bug_field");
        assert_eq!(refused.got().len(), 1, "an answer of the server is final");

        let own = Arc::new(FakeHttp {
            reach: CloudReach::Own,
            answers: Mutex::default(),
            got: Mutex::default(),
        });
        let e = rt
            .bug_send(input(), &ProjectReports::at(own.clone(), BUG_PATH))
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "invalid input: no_host");
        assert!(own.got().is_empty(), "nothing goes to the project");
        assert_eq!(
            messenger_store::bugs::count_queued(rt.store())
                .await
                .unwrap(),
            0,
            "nor is it kept"
        );
        rt.shutdown().await;
    }
}

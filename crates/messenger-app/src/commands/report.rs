// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A report of a person to the moderators of the project: the command of
//! the page (`messenger_report_send`, the types of
//! `ui/src/lib/messenger/generated/moderation.ts`) and the way of a report
//! to the server ([`ProjectReports`]). What a report holds, how it is
//! checked, kept and tried again is the runtime's (`messenger_runtime::
//! report`); here is only the HTTP: `POST /cloud/v1/report` on a host of
//! the project, by the way the module cloud takes too (`ProjectHttp`: the
//! hosts of the manifest in use, the bridges), signed by the identity
//! (NIP-98, `Authorization: Nostr …`).
//!
//! A report that waits is tried again by [`spawn_retry`], a task of the
//! running messenger; its delivery reaches the page as the runtime event
//! `moderation.report`.

use super::{map_err, MessengerState, Task};
use crate::cloud_seams::{nip98_event, ProjectHttp};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_runtime::report::{Delivery, ReportInput, ReportResult, ReportSender};
use messenger_runtime::MessengerRuntime;
use nostr::prelude::Keys;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use veydan_core::CmdResult;
use veydan_shell::seams::{code, CloudError, CloudReach, Http, HttpRequest, HttpResponse};

/// Where a report goes on a host of the project.
pub const REPORT_PATH: &str = "/cloud/v1/report";

/// How often the reports that wait are looked at. Each one keeps its own
/// pause (a minute after the first failure, doubling up to six hours);
/// this only wakes the runtime up.
const RETRY_TICK: Duration = Duration::from_secs(60);

/// Report a person; the input and the answer are the contract of the page.
/// A refusal is a word: `rate_limited`, `report_*` of a malformed input,
/// the word of the server, `no_host` with the user's own servers in use.
#[tauri::command]
pub async fn messenger_report_send(
    input: ReportInput,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<ReportResult> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let rt = messenger.runtime_unlocked().await?;
        let sender = ProjectReports::new(Arc::new(ProjectHttp::new(messenger.app.clone())));
        let result = rt.report_send(input, &sender).await.map_err(map_err)?;
        if result.blocked {
            // A stranger blocked is a chat the list no longer has.
            super::unread_changed();
        }
        Ok(result)
    })
    .await
}

/// The task that tries the waiting reports again while the messenger runs.
pub(super) fn spawn_retry(app: tauri::AppHandle, rt: Arc<MessengerRuntime>) -> Task {
    tauri::async_runtime::spawn(async move {
        let sender = ProjectReports::new(Arc::new(ProjectHttp::new(app)));
        loop {
            tokio::time::sleep(RETRY_TICK).await;
            if let Err(e) = rt.reports_retry(&sender, unix_now() as i64).await {
                eprintln!("messenger report: the waiting reports were not tried: {e}");
            }
        }
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// [`ReportSender`] over the way to the hosts of the project. The hosts
/// are tried in their order, the next one only when the network failed or
/// a gateway answered for a server that is not there (as the module cloud
/// does, cloud-api.md §12); an answer of the server is final.
pub struct ProjectReports {
    http: Arc<dyn Http>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorFields,
}

#[derive(Deserialize)]
struct ErrorFields {
    code: String,
    server_time: Option<u64>,
}

#[derive(Deserialize)]
struct Taken {
    id: Option<String>,
}

impl ProjectReports {
    pub fn new(http: Arc<dyn Http>) -> Self {
        Self { http }
    }

    /// One `POST` of `body` to `url`, signed at `at`.
    async fn post(
        &self,
        keys: &Keys,
        url: &str,
        body: &[u8],
        at: u64,
    ) -> Result<HttpResponse, CloudError> {
        let digest: [u8; 32] = Sha256::digest(body).into();
        let event = nip98_event(keys, "POST", url, Some(digest), at)?;
        let json =
            serde_json::to_vec(&event).map_err(|e| CloudError::new(code::CRYPTO, e.to_string()))?;
        self.http
            .send(HttpRequest {
                method: "POST".into(),
                url: url.to_string(),
                headers: vec![
                    ("accept".into(), "application/json".into()),
                    ("content-type".into(), "application/json".into()),
                    (
                        "authorization".into(),
                        format!("Nostr {}", B64.encode(json)),
                    ),
                ],
                body: body.to_vec(),
            })
            .await
    }
}

/// A gateway that answered for a server it could not reach: its page is no
/// answer of ours, and another host may do.
fn gateway_failed(answer: &HttpResponse) -> bool {
    matches!(answer.status, 502..=504) && serde_json::from_slice::<ErrorBody>(&answer.body).is_err()
}

/// The word of a refusal: the server's, or one for its status.
fn refusal_word(answer: &HttpResponse) -> String {
    match serde_json::from_slice::<ErrorBody>(&answer.body) {
        Ok(body) if !body.error.code.is_empty() => body.error.code,
        _ => match answer.status {
            401 => "auth_invalid".into(),
            404 => "not_found".into(),
            413 => "payload_too_large".into(),
            other => format!("http_{other}"),
        },
    }
}

#[async_trait]
impl ReportSender for ProjectReports {
    async fn send(&self, keys: &Keys, body: &[u8]) -> Delivery {
        let hosts = match self.http.reach().await {
            Ok(CloudReach::Veydan { hosts } | CloudReach::Unchosen { hosts })
                if !hosts.is_empty() =>
            {
                hosts
            }
            // Nothing goes to the project while the servers are the user's
            // own (vcloud-plan.md 4.5); a report waits for the switch back.
            Ok(_) => {
                return Delivery::NoWay {
                    why: code::NO_HOST.into(),
                }
            }
            Err(e) => return Delivery::NoWay { why: e.code },
        };
        let mut failed = None;
        'hosts: for host in hosts {
            let url = format!("{host}{REPORT_PATH}");
            let mut at = unix_now();
            // Once more at the server's clock when it says the signature is
            // out of time: the device's clock is off (cloud-api.md 2.2).
            for _ in 0..2 {
                let answer = match self.post(keys, &url, body, at).await {
                    Ok(answer) => answer,
                    Err(e) if e.is(code::TRANSPORT) => {
                        failed = Some(e.message);
                        continue 'hosts;
                    }
                    Err(e) => return Delivery::NoWay { why: e.code },
                };
                if gateway_failed(&answer) {
                    failed = Some(format!("the gateway of {host} answered {}", answer.status));
                    continue 'hosts;
                }
                if (200..300).contains(&answer.status) {
                    let id = serde_json::from_slice::<Taken>(&answer.body)
                        .ok()
                        .and_then(|t| t.id);
                    return Delivery::Accepted { id };
                }
                if answer.status == 429 || answer.status >= 500 {
                    return Delivery::Failed {
                        why: format!("the server answered {}", answer.status),
                    };
                }
                if let Ok(ErrorBody {
                    error:
                        ErrorFields {
                            code,
                            server_time: Some(server),
                        },
                }) = serde_json::from_slice(&answer.body)
                {
                    if code == "auth_expired" && server != at {
                        at = server;
                        continue;
                    }
                }
                return Delivery::Refused {
                    error: refusal_word(&answer),
                };
            }
            return Delivery::Refused {
                error: "auth_expired".into(),
            };
        }
        Delivery::Failed {
            why: failed.unwrap_or_else(|| "no host answered".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{MessengerConfig, SecretStore};
    use messenger_runtime::report::{
        ReportContext, ReportReason, ReportStatus, UI_EVENT_MODERATION_REPORT,
    };
    use messenger_runtime::{ManifestFetcher, ManifestRemote};
    use nostr::prelude::{Event, Kind};
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;
    use veydan_shell::seams::SeamFuture;
    use zeroize::Zeroizing;

    /// What the fake server answers to one request.
    enum Answer {
        Down,
        Status(u16, String),
    }

    /// `Http` of the project with a server that answers as it is told, and
    /// keeps every request.
    struct FakeHttp {
        reach: CloudReach,
        answers: Mutex<VecDeque<Answer>>,
        got: Mutex<Vec<HttpRequest>>,
    }

    impl FakeHttp {
        fn new(hosts: &[&str], answers: impl IntoIterator<Item = Answer>) -> Arc<Self> {
            Arc::new(Self {
                reach: CloudReach::Veydan {
                    hosts: hosts.iter().map(|h| h.to_string()).collect(),
                },
                answers: Mutex::new(answers.into_iter().collect()),
                got: Mutex::default(),
            })
        }

        fn got(&self) -> Vec<HttpRequest> {
            self.got.lock().unwrap().clone()
        }
    }

    impl Http for FakeHttp {
        fn reach(&self) -> SeamFuture<'_, CloudReach> {
            let reach = self.reach.clone();
            Box::pin(async move { Ok(reach) })
        }

        fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
            self.got.lock().unwrap().push(request);
            let answer = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Answer::Down);
            Box::pin(async move {
                match answer {
                    Answer::Down => Err(CloudError::new(code::TRANSPORT, "connection refused")),
                    Answer::Status(status, body) => Ok(HttpResponse {
                        status,
                        headers: vec![],
                        body: body.into_bytes(),
                    }),
                }
            })
        }
    }

    #[derive(Default)]
    struct MemorySecrets(Mutex<HashMap<String, Vec<u8>>>);

    #[async_trait]
    impl SecretStore for MemorySecrets {
        async fn get(&self, key: &str) -> messenger_core::Result<Option<Zeroizing<Vec<u8>>>> {
            Ok(self.0.lock().unwrap().get(key).cloned().map(Zeroizing::new))
        }
        async fn put(&self, key: &str, value: &[u8]) -> messenger_core::Result<()> {
            self.0.lock().unwrap().insert(key.into(), value.to_vec());
            Ok(())
        }
        async fn delete(&self, key: &str) -> messenger_core::Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
        async fn is_unlocked(&self) -> bool {
            true
        }
    }

    struct Offline;

    #[async_trait]
    impl ManifestFetcher for Offline {
        async fn fetch(&self, _: &str) -> messenger_core::Result<String> {
            Err(messenger_core::MessengerError::Transport("offline".into()))
        }
    }

    /// A messenger with a session and no network: the project's servers
    /// from the built-in manifest, relays silent.
    async fn messenger(dir: &std::path::Path) -> (MessengerRuntime, Keys) {
        let rt = MessengerRuntime::start(
            MessengerConfig::new(dir.join("messenger")),
            Arc::new(MemorySecrets::default()),
        )
        .await
        .unwrap();
        rt.relays().set_silent(true).await.unwrap();
        rt.set_manifest_remote(ManifestRemote {
            fetcher: Arc::new(Offline),
            urls: vec![],
            pinned: String::new(),
        });
        rt.servers_use_veydan().await.unwrap();
        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        let keys = rt.identity().load_keys().await.unwrap();
        (rt, keys)
    }

    fn input(accused: &Keys) -> ReportInput {
        ReportInput {
            accused: accused.public_key().to_hex(),
            target: None,
            context: ReportContext::Dm,
            reason: ReportReason::Spam,
            sub_reason: None,
            comment: None,
            attach: false,
            message_id: None,
            block: false,
        }
    }

    /// The signer of a request, checked as the server checks it: the event
    /// of the header is of kind 27235, signed, for this URL, this method
    /// and this body.
    fn signer_of(request: &HttpRequest) -> String {
        let header = request
            .headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        let json = B64.decode(header.strip_prefix("Nostr ").unwrap()).unwrap();
        let event: Event = serde_json::from_slice(&json).unwrap();
        assert_eq!(event.kind, Kind::HttpAuth);
        event.verify().unwrap();
        let tag = |name: &str| {
            event
                .tags
                .iter()
                .find(|t| t.kind() == name)
                .and_then(|t| t.as_slice().get(1).cloned())
        };
        assert_eq!(tag("u").as_deref(), Some(request.url.as_str()));
        assert_eq!(tag("method").as_deref(), Some("POST"));
        let digest: [u8; 32] = Sha256::digest(&request.body).into();
        assert_eq!(
            tag("payload"),
            Some(
                digest
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ),
            "signed for this body"
        );
        event.pubkey.to_hex()
    }

    /// The server down: the report is kept. It goes at the next due try, to
    /// the path of the contract, signed by the identity, and the page is
    /// told with the server's id.
    #[tokio::test]
    async fn a_report_that_failed_is_kept_and_delivered_by_the_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (rt, me) = messenger(dir.path()).await;
        let bob = Keys::generate();
        let down = FakeHttp::new(
            &["https://a.example", "https://b.example"],
            [
                Answer::Down,
                Answer::Status(502, "<html>bad gateway</html>".into()),
            ],
        );
        let r = rt
            .report_send(input(&bob), &ProjectReports::new(down.clone()))
            .await
            .unwrap();
        assert_eq!(r.status, ReportStatus::Queued);
        let tried: Vec<String> = down.got().iter().map(|r| r.url.clone()).collect();
        assert_eq!(
            tried,
            vec![
                "https://a.example/cloud/v1/report",
                "https://b.example/cloud/v1/report"
            ],
            "each host once"
        );

        let mut events = rt.ui_events();
        let up = FakeHttp::new(
            &["https://a.example"],
            [Answer::Status(202, r#"{"id":"rep-7"}"#.into())],
        );
        let later = unix_now() as i64 + 2 * 60;
        assert_eq!(
            rt.reports_retry(&ProjectReports::new(up.clone()), later)
                .await
                .unwrap(),
            1
        );
        let got = up.got();
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].method.as_str(), got[0].url.as_str()),
            ("POST", "https://a.example/cloud/v1/report")
        );
        assert_eq!(signer_of(&got[0]), me.public_key().to_hex());
        assert_eq!(got[0].body, down.got()[0].body, "the report as it was made");
        let report: serde_json::Value = serde_json::from_slice(&got[0].body).unwrap();
        assert_eq!(
            (report["accused"].as_str(), report["reason"].as_str()),
            (Some(bob.public_key().to_hex().as_str()), Some("spam"))
        );
        let told = loop {
            let ev = events.try_recv().expect("the page is told");
            if ev.name == UI_EVENT_MODERATION_REPORT {
                break ev.payload;
            }
        };
        assert_eq!(told, serde_json::json!({ "id": "rep-7", "status": "sent" }));
        assert_eq!(
            rt.reports_retry(&ProjectReports::new(up.clone()), later + 86_400)
                .await
                .unwrap(),
            0,
            "sent once"
        );
        rt.shutdown().await;
    }

    /// A refusal of the server is its word and is not kept; a signature out
    /// of time is made again at the server's clock; the user's own servers
    /// are no way to the project.
    #[tokio::test]
    async fn a_refusal_is_the_servers_word_and_the_clock_of_the_server_is_heeded() {
        let dir = tempfile::tempdir().unwrap();
        let (rt, _me) = messenger(dir.path()).await;
        let bob = Keys::generate();
        let refused = FakeHttp::new(
            &["https://a.example", "https://b.example"],
            [Answer::Status(
                422,
                r#"{"error":{"code":"too_many_messages","message":"…"}}"#.into(),
            )],
        );
        let e = rt
            .report_send(input(&bob), &ProjectReports::new(refused.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "invalid input: too_many_messages");
        assert_eq!(
            refused.got().len(),
            1,
            "an answer of the server is final: no second host"
        );

        let server = unix_now() + 3_600;
        let expired = format!(
            r#"{{"error":{{"code":"auth_expired","message":"…","server_time":{server}}}}}"#
        );
        let skewed = FakeHttp::new(
            &["https://a.example"],
            [
                Answer::Status(401, expired),
                Answer::Status(202, "{}".into()),
            ],
        );
        let r = rt
            .report_send(input(&bob), &ProjectReports::new(skewed.clone()))
            .await
            .unwrap();
        assert_eq!(r.status, ReportStatus::Sent);
        let got = skewed.got();
        assert_eq!(got.len(), 2);
        let second: Event = serde_json::from_slice(
            &B64.decode(got[1].headers[2].1.strip_prefix("Nostr ").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            second.created_at.as_secs(),
            server,
            "signed again at the server's clock"
        );

        let own = Arc::new(FakeHttp {
            reach: CloudReach::Own,
            answers: Mutex::default(),
            got: Mutex::default(),
        });
        let e = rt
            .report_send(input(&bob), &ProjectReports::new(own.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "invalid input: no_host");
        assert!(own.got().is_empty(), "nothing goes to the project");
        rt.shutdown().await;
    }
}

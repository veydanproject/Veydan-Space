// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cloud_status`: what the page of the module shows first — whose servers
//! are in use, whether Veydan Cloud answered for the identity, the account
//! and plan when there is one, what this device keeps of it, and where an
//! activation is.

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use veydan_shell::seams::{CloudError, CloudReach, Http, Signer};

use crate::account::{self, KeptAccount};
use crate::activate::{Activation, ActivationView};
use crate::api::{Account, CloudApi};

/// Whose servers the messenger uses, as the page names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Servers {
    /// The product offers no way to the project (no messenger).
    None,
    /// Not chosen yet: the onboarding of the messenger.
    Unchosen,
    /// The user's own: Veydan Cloud works only with the project's servers.
    Own,
    /// The project's.
    Veydan,
}

/// The answer of `cloud_status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudStatus {
    /// A host of Veydan Cloud answered for the identity, with an account
    /// or with `not_activated`.
    pub configured: bool,
    pub account: Option<Account>,
    /// The id of the plan of the account (`free`, `plus`, …).
    pub plan: Option<String>,
    pub servers: Servers,
    /// What this device keeps of the account of this identity: shown when
    /// the server did not answer.
    pub kept: Option<KeptAccount>,
    /// Why the server did not answer, when it did not: the page branches on
    /// its `code` (cloud-api.md §12, section 9).
    pub error: Option<CloudError>,
    /// Where an activation of this run is.
    pub activation: ActivationView,
}

impl CloudStatus {
    fn of(servers: Servers) -> Self {
        Self {
            configured: false,
            account: None,
            plan: None,
            servers,
            kept: None,
            error: None,
            activation: ActivationView::IDLE,
        }
    }
}

/// The status as the client sees it. Nothing is asked unless the servers
/// are the project's (or the product has no way to tell, and the client
/// answers alone); `not_activated` is an answer — no account — and any
/// other failure is kept in `error` for the page.
pub async fn status_of(api: &dyn CloudApi, reach: Option<CloudReach>) -> CloudStatus {
    let servers = match reach {
        None => Servers::None,
        Some(CloudReach::Unchosen { .. }) => return CloudStatus::of(Servers::Unchosen),
        Some(CloudReach::Own) => return CloudStatus::of(Servers::Own),
        Some(CloudReach::Veydan { .. }) => Servers::Veydan,
    };
    match api.account().await {
        Ok(account) => CloudStatus {
            configured: true,
            plan: Some(account.plan.clone()),
            account: Some(account),
            ..CloudStatus::of(servers)
        },
        Err(e) if e.is("not_activated") => CloudStatus {
            configured: true,
            ..CloudStatus::of(servers)
        },
        Err(e) => CloudStatus {
            error: Some(e),
            ..CloudStatus::of(servers)
        },
    }
}

/// The page asks once each time it opens (cloud-api.md 4.3: the screen
/// is refreshed by opening it, not by a timer); what the server says is
/// kept in the box, and what the box keeps is shown when it says nothing.
#[tauri::command]
pub async fn cloud_status<R: Runtime>(app: AppHandle<R>) -> Result<CloudStatus, CloudError> {
    let reach = match veydan_shell::seam::<dyn Http, _, _>(&app) {
        Some(http) => Some(http.reach().await?),
        None => None,
    };
    let mut status = status_of(crate::client(&app).as_ref(), reach).await;
    if let Some(activation) = app.try_state::<Activation>() {
        status.activation = activation.view();
    }
    if let Some(account) = &status.account {
        let _ = account::keep(&app, account).await;
    } else if status.configured {
        let _ = account::forget(&app).await;
    } else if status.servers == Servers::Veydan {
        // Only the account of the identity in use; an earlier one is not shown.
        let me = match veydan_shell::seam::<dyn Signer, _, _>(&app) {
            Some(signer) => signer.pubkey().await.ok(),
            None => None,
        };
        let kept = account::load(&app).await.ok().flatten();
        status.kept = kept.filter(|k| Some(&k.npub) == me.as_ref());
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Unconfigured;
    use crate::fake::Fake;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use veydan_shell::seams::code;

    #[tokio::test]
    async fn nothing_is_asked_off_the_servers_of_the_project() {
        let fake = Fake::start(2).await;
        let api = fake.client();
        for (reach, servers) in [
            (CloudReach::Own, Servers::Own),
            (CloudReach::Unchosen { hosts: vec![] }, Servers::Unchosen),
        ] {
            let status = status_of(&api, Some(reach)).await;
            assert_eq!(status, CloudStatus::of(servers));
        }
        assert_eq!(fake.requests(), 0);
    }

    #[tokio::test]
    async fn no_account_is_an_answer_and_an_account_is_its_plan() {
        let fake = Fake::start(2).await;
        let api = fake.client();
        let veydan = || Some(fake.reach());
        let none = status_of(&api, veydan()).await;
        assert!(none.configured && none.account.is_none() && none.error.is_none());
        fake.add_account();
        let status = status_of(&api, veydan()).await;
        assert!(status.configured);
        assert_eq!(status.plan.as_deref(), Some("free"));
        assert_eq!(status.servers, Servers::Veydan);
        let unconfigured = status_of(&Unconfigured, None).await;
        assert!(!unconfigured.configured);
        assert_eq!(
            unconfigured.error.map(|e| e.code).as_deref(),
            Some(code::NO_HOST)
        );
    }

    /// The command as the page calls it, through the IPC of a mock app.
    fn ask(app: &tauri::App<tauri::test::MockRuntime>) -> Result<Value, Value> {
        let webview = tauri::WebviewWindowBuilder::new(app, "main", Default::default())
            .build()
            .expect("webview");
        let url = if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        get_ipc_response(
            &webview,
            InvokeRequest {
                cmd: "cloud_status".into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: url.parse().expect("url"),
                body: InvokeBody::default(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().expect("json"))
    }

    #[test]
    fn a_product_without_the_seams_answers_not_configured() {
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![cloud_status])
            .build(mock_context(noop_assets()))
            .expect("app");
        veydan_shell::provide::<dyn CloudApi, _, _>(app.handle(), Arc::new(Unconfigured));
        let answer = ask(&app).expect("answer");
        assert_eq!(answer["configured"], false);
        assert_eq!(answer["servers"], "none");
        assert_eq!(answer["error"]["code"], "no_host");
        assert_eq!(
            answer["activation"],
            json!({ "phase": "idle", "attempts": 0, "expected": 0, "error": null })
        );
    }

    /// Through the IPC with the fake: the account is answered and kept; when
    /// the host stops answering, the kept account is shown with the error.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_sees_the_account_and_the_kept_one_when_the_host_is_down() {
        let fake = Fake::start(2).await;
        fake.add_account();
        let app = fake.app();
        let status = cloud_status(app.handle().clone()).await.expect("status");
        assert_eq!(status.plan.as_deref(), Some("free"));
        assert!(account::load(app.handle()).await.expect("load").is_some());
        fake.go_down();
        let down = cloud_status(app.handle().clone()).await.expect("status");
        assert!(!down.configured);
        assert_eq!(down.error.map(|e| e.code).as_deref(), Some(code::NO_HOST));
        assert_eq!(down.kept.map(|k| k.plan).as_deref(), Some("free"));
    }
}

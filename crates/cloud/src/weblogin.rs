// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The web cabinet of Veydan Cloud from the app (cloud-api.md §16.4): the
//! approval of a sign-in by its QR code (`veydan://web/<id>`, read by the
//! app's own scanner or pasted), «Это не я» of a sign-in or a code, the
//! confirmation of a dangerous action the cabinet asks for, the live
//! sessions of the cabinet and the freeze of the code sign-in.
//!
//! What a screen shows of a request — its country and browser — is held
//! here to its shape before it leaves the module, as the cards of the bot
//! are (messenger-wire.md §3): a host is trusted, yet nothing of an answer
//! reaches the screen as a sentence.

use serde::Serialize;
use tauri::{AppHandle, Runtime};
use veydan_shell::seams::{code, CloudError, LockConfirm};
use zeroize::Zeroizing;

use crate::api::{WebLogin, WebRevoked, WebSession, WebSessions, WebState};
use crate::delete::{confirmed, lock_gate, LOCK_WRONG};

/// The families of browsers the cabinet names (16.1); anything else is `other`.
pub const UA_FAMILIES: [&str; 6] = ["chrome", "firefox", "safari", "edge", "opera", "other"];

/// A country as the screen may show it: two capital letters, else nothing.
fn country(text: &str) -> Option<String> {
    (text.len() == 2 && text.bytes().all(|b| b.is_ascii_uppercase())).then(|| text.to_string())
}

/// A family of a browser as the screen may show it.
fn ua_family(text: &str) -> String {
    if UA_FAMILIES.contains(&text) {
        text
    } else {
        "other"
    }
    .to_string()
}

/// A word of the server (`qr`, `pending`, …) as the screen branches on it:
/// a short word of small letters, else `unknown`.
fn word(text: &str) -> String {
    let ok = !text.is_empty()
        && text.len() <= 32
        && text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    if ok { text } else { "unknown" }.to_string()
}

/// A sign-in as the screen of approval shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebLoginView {
    pub id: String,
    /// `qr` or `code`: only `qr` is approved here.
    pub method: String,
    /// `pending`, `approved`, `denied`, `expired`, `used`.
    pub state: String,
    pub country: Option<String>,
    pub ua_family: String,
    pub created_at: u64,
    pub expires_at: u64,
}

impl WebLoginView {
    fn of(login: WebLogin) -> Self {
        Self {
            id: login.id,
            method: word(&login.method),
            state: word(&login.state),
            country: country(&login.country),
            ua_family: ua_family(&login.ua_family),
            created_at: login.created_at,
            expires_at: login.expires_at,
        }
    }
}

/// A live session of the cabinet as the block «Входы на сайт» lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebSessionView {
    pub id: String,
    pub method: String,
    pub created_at: u64,
    pub last_seen_at: u64,
    pub country: Option<String>,
    pub ua_family: String,
}

impl WebSessionView {
    fn of(session: WebSession) -> Option<Self> {
        // A session without an id of its shape cannot be ended: not listed.
        crate::api::is_web_id(&session.id).then(|| Self {
            id: session.id,
            method: word(&session.method),
            created_at: session.created_at,
            last_seen_at: session.last_seen_at,
            country: country(&session.country),
            ua_family: ua_family(&session.ua_family),
        })
    }
}

/// The answer of `cloud_web_sessions`: the sessions, and until when the
/// code sign-in is frozen when the server says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebSessionsView {
    pub sessions: Vec<WebSessionView>,
    pub frozen_until: Option<u64>,
}

impl WebSessionsView {
    fn of(answer: WebSessions, now: u64) -> Self {
        Self {
            sessions: answer
                .sessions
                .into_iter()
                .filter_map(WebSessionView::of)
                .collect(),
            // A freeze already over is no freeze to lift.
            frozen_until: answer.freeze.map(|f| f.until).filter(|until| *until > now),
        }
    }
}

/// The answer of approve, deny and confirm, its state as a word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebStateView {
    pub state: String,
    pub frozen_until: Option<u64>,
}

impl From<WebState> for WebStateView {
    fn from(answer: WebState) -> Self {
        Self {
            state: word(&answer.state),
            frozen_until: answer.frozen_until,
        }
    }
}

/// What a sign-in by `id` is: the country and the browser of the request,
/// its state and until when it waits (`GET /cloud/v1/web/login`, not signed).
#[tauri::command]
pub async fn cloud_web_login_inspect<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<WebLoginView, CloudError> {
    let login = crate::client(&app).web_login(&id).await?;
    Ok(WebLoginView::of(login))
}

/// «Войти»: the sign-in `id` is approved by the identity (`POST …/web/approve`).
#[tauri::command]
pub async fn cloud_web_login_approve<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<WebStateView, CloudError> {
    crate::client(&app).web_approve(&id).await.map(Into::into)
}

/// «Это не я»: the sign-in `id` is denied; a sign-in by code freezes the
/// code sign-in of the identity for 30 days (`POST …/web/deny`).
#[tauri::command]
pub async fn cloud_web_login_deny<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<WebStateView, CloudError> {
    crate::client(&app).web_deny(&id).await.map(Into::into)
}

/// Confirms the action `id` the cabinet asked for (`POST …/web/confirm`).
/// The one action is the deletion of the account, so the guards are those
/// of `cloud_delete_account`: the word DELETE (else `confirm_required`)
/// and the secret of the lock when one is set (else `lock_wrong`), checked
/// here and not only on the screen: a page of the app cannot skip them.
#[tauri::command]
pub async fn cloud_web_confirm<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    confirm: String,
    lock_secret: Option<String>,
) -> Result<WebStateView, CloudError> {
    let lock_secret = lock_secret.map(Zeroizing::new);
    if !confirmed(&confirm) {
        return Err(CloudError::new(
            "confirm_required",
            "type the word DELETE to confirm",
        ));
    }
    match lock_gate(&app)
        .confirm(lock_secret.as_ref().map(|s| s.as_str()))
        .await?
    {
        LockConfirm::NoLock | LockConfirm::Confirmed => {}
        LockConfirm::Wrong => {
            return Err(CloudError::new(
                LOCK_WRONG,
                "the secret of the lock is not right",
            ))
        }
    }
    crate::client(&app).web_confirm(&id).await.map(Into::into)
}

/// Lifts the freeze of the code sign-in (`POST …/web/unfreeze`).
#[tauri::command]
pub async fn cloud_web_unfreeze<R: Runtime>(app: AppHandle<R>) -> Result<(), CloudError> {
    crate::client(&app).web_unfreeze().await
}

/// The live sessions of the cabinet (`GET …/web/sessions`).
#[tauri::command]
pub async fn cloud_web_sessions<R: Runtime>(
    app: AppHandle<R>,
) -> Result<WebSessionsView, CloudError> {
    let answer = crate::client(&app).web_sessions().await?;
    Ok(WebSessionsView::of(answer, crate::api::now()?))
}

/// Ends the session `id`, or every session with `all` (`POST …/web/sessions/revoke`).
#[tauri::command]
pub async fn cloud_web_sessions_revoke<R: Runtime>(
    app: AppHandle<R>,
    id: Option<String>,
    all: Option<bool>,
) -> Result<WebRevoked, CloudError> {
    match (id.as_deref(), all.unwrap_or(false)) {
        (Some(id), false) => crate::client(&app).web_sessions_revoke(Some(id)).await,
        (None, true) => crate::client(&app).web_sessions_revoke(None).await,
        _ => Err(CloudError::new(
            code::BAD_REQUEST,
            "either the id of a session or all",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    const ID: &str = "0123456789abcdef";

    #[test]
    fn what_a_server_says_is_held_to_its_shape() {
        assert_eq!(country("DE").as_deref(), Some("DE"));
        for bad in ["", "de", "DEU", "D1", "Germany"] {
            assert_eq!(country(bad), None, "{bad}");
        }
        assert_eq!(ua_family("firefox"), "firefox");
        assert_eq!(ua_family("Mozilla/5.0 (X11)"), "other");
        assert_eq!(word("pending"), "pending");
        assert_eq!(word("Go to evil.example"), "unknown");
        let view = WebLoginView::of(WebLogin {
            id: ID.into(),
            method: "qr".into(),
            state: "pending".into(),
            country: "Visit evil.example".into(),
            ua_family: "Chrome 120".into(),
            created_at: 1,
            expires_at: 121,
        });
        assert_eq!((view.country, view.ua_family.as_str()), (None, "other"));
    }

    /// A sign-in by QR: inspected without a signature, approved by the
    /// identity, then shown approved.
    #[tokio::test]
    async fn a_sign_in_is_inspected_and_approved() {
        let fake = Fake::start(2).await;
        fake.add_web_login(ID, "qr");
        let app = fake.app();
        let view = cloud_web_login_inspect(app.handle().clone(), ID.into())
            .await
            .expect("inspect");
        assert_eq!(
            (
                view.method.as_str(),
                view.state.as_str(),
                view.country.as_deref(),
                view.ua_family.as_str()
            ),
            ("qr", "pending", Some("DE"), "chrome")
        );
        let done = cloud_web_login_approve(app.handle().clone(), ID.into())
            .await
            .expect("approve");
        assert_eq!(done.state, "approved");
        assert_eq!(
            fake.web_login_npub(ID),
            Some(fake.npub()),
            "the signer is who signed in"
        );
        let view = cloud_web_login_inspect(app.handle().clone(), ID.into())
            .await
            .expect("inspect");
        assert_eq!(view.state, "approved");
        // A sign-in by code is approved by its code, never by a signature.
        fake.add_web_login("fedcba9876543210", "code");
        let refused =
            cloud_web_login_approve(app.handle().clone(), "fedcba9876543210".into()).await;
        assert_eq!(refused.map_err(|e| e.code), Err("method_mismatch".into()));
    }

    #[tokio::test]
    async fn an_id_that_is_not_one_never_leaves_the_device() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        for bad in ["", "0123456789ABCDEF", "0123456789abcdef&x=1", "../account"] {
            let refused = cloud_web_login_inspect(app.handle().clone(), bad.into()).await;
            assert_eq!(
                refused.map_err(|e| e.code),
                Err(code::BAD_REQUEST.into()),
                "{bad}"
            );
            let refused = cloud_web_login_deny(app.handle().clone(), bad.into()).await;
            assert_eq!(
                refused.map_err(|e| e.code),
                Err(code::BAD_REQUEST.into()),
                "{bad}"
            );
        }
        assert_eq!(fake.requests(), 0);
    }

    /// «Это не я» of a code: denied and the code sign-in frozen; the freeze
    /// is reported with the sessions and lifted.
    #[tokio::test]
    async fn a_code_denied_freezes_the_code_sign_in_until_lifted() {
        let fake = Fake::start(2).await;
        fake.add_web_login(ID, "code");
        let app = fake.app();
        let denied = cloud_web_login_deny(app.handle().clone(), ID.into())
            .await
            .expect("deny");
        assert_eq!(denied.state, "denied");
        let until = denied.frozen_until.expect("frozen");
        let listed = cloud_web_sessions(app.handle().clone())
            .await
            .expect("sessions");
        assert_eq!(listed.frozen_until, Some(until));
        cloud_web_unfreeze(app.handle().clone())
            .await
            .expect("unfreeze");
        let listed = cloud_web_sessions(app.handle().clone())
            .await
            .expect("sessions");
        assert_eq!(listed.frozen_until, None);
    }

    #[tokio::test]
    async fn sessions_are_listed_and_ended_one_or_all() {
        let fake = Fake::start(2).await;
        fake.add_web_session("1111111111111111");
        fake.add_web_session("2222222222222222");
        fake.add_web_session("3333333333333333");
        let app = fake.app();
        let listed = cloud_web_sessions(app.handle().clone())
            .await
            .expect("sessions");
        assert_eq!(listed.sessions.len(), 3);
        assert_eq!(
            (
                listed.sessions[0].country.as_deref(),
                listed.sessions[0].ua_family.as_str()
            ),
            (Some("DE"), "firefox")
        );
        let one =
            cloud_web_sessions_revoke(app.handle().clone(), Some("2222222222222222".into()), None)
                .await
                .expect("one");
        assert_eq!(one.revoked, 1);
        assert_eq!(
            cloud_web_sessions(app.handle().clone())
                .await
                .expect("sessions")
                .sessions
                .len(),
            2
        );
        let all = cloud_web_sessions_revoke(app.handle().clone(), None, Some(true))
            .await
            .expect("all");
        assert_eq!(all.revoked, 2);
        assert!(cloud_web_sessions(app.handle().clone())
            .await
            .expect("sessions")
            .sessions
            .is_empty());
        for (id, all) in [
            (None, None),
            (Some(ID.to_string()), Some(true)),
            (None, Some(false)),
        ] {
            let refused = cloud_web_sessions_revoke(app.handle().clone(), id, all).await;
            assert_eq!(refused.map_err(|e| e.code), Err(code::BAD_REQUEST.into()));
        }
    }

    /// The confirmation of a deletion asks the word and the lock, as the
    /// deletion in the app does, before anything is sent.
    #[tokio::test]
    async fn a_confirmation_asks_the_word_and_the_lock() {
        let fake = Fake::start(2).await;
        fake.add_web_action(ID);
        let app = fake.app_locked(Some("1234"));
        let refused = cloud_web_confirm(
            app.handle().clone(),
            ID.into(),
            "delete".into(),
            Some("1234".into()),
        )
        .await;
        assert_eq!(refused.map_err(|e| e.code), Err("confirm_required".into()));
        let refused = cloud_web_confirm(
            app.handle().clone(),
            ID.into(),
            "DELETE".into(),
            Some("0000".into()),
        )
        .await;
        assert_eq!(refused.map_err(|e| e.code), Err(LOCK_WRONG.into()));
        let refused =
            cloud_web_confirm(app.handle().clone(), ID.into(), "DELETE".into(), None).await;
        assert_eq!(refused.map_err(|e| e.code), Err(LOCK_WRONG.into()));
        assert_eq!(fake.requests(), 0);
        let done = cloud_web_confirm(
            app.handle().clone(),
            ID.into(),
            "DELETE".into(),
            Some("1234".into()),
        )
        .await
        .expect("confirm");
        assert_eq!(done.state, "confirmed");
        // Another action, or one already gone: not found.
        let refused = cloud_web_confirm(
            app.handle().clone(),
            "fedcba9876543210".into(),
            "DELETE".into(),
            Some("1234".into()),
        )
        .await;
        assert_eq!(refused.map_err(|e| e.code), Err("not_found".into()));
    }
}

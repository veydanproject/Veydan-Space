// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The VoIP pushes of an iPhone: the invitation to a call that PushKit
//! hands the app, woken or started for it (VPush spec/protocol.md, «VoIP
//! pushes»).
//!
//! The Swift side of the call plugin (`VoipPushes.swift`) reports every
//! VoIP push to CallKit at once, with a placeholder («Incoming call», no
//! name), before the handler of the push returns: Apple kills an app that
//! does not, and stops handing it VoIP pushes. The push then comes here,
//! and is opened by the same `messenger_notify::describe_as` the push
//! handler of Android runs, in the app's own process and with the keys of
//! the session that runs ([`MessengerRuntime::notify_bundle`]): the call
//! it rings takes the placeholder's place on CallKit's screen
//! (`voip_ring`, the name and the video), anything else ends the
//! placeholder (`voip_end`). A press made on the placeholder comes as a
//! press of the call once it is known: an Answer before the runtime heard
//! the invitation from the relays is the expected answer of
//! [`super::EXPECTED_FOR`], taken the moment the call rings in.
//!
//! The runtime's own `call.incoming`, if it hears the invitation first,
//! rings through `show_incoming` as ever, and the Swift side takes it for
//! the placeholder; whichever comes second changes nothing.
//!
//! No key leaves the app for this: nothing on iOS hands keys to a handler
//! (Android's `keep_handler_keys`), and there is no extension yet. So a
//! push is opened only while the messenger runs. On a cold start the
//! placeholder rings until the runtime is up and hears it (the Swift side
//! holds it for the life of an invitation); with the app's PIN the session
//! opens at the unlock, and the placeholder stays nameless until then
//! (the owner's decision of 2026-11-19). The Notification Service
//! Extension (stage Э12) moves the opening out of the app, with the keys
//! in the Keychain, and this file then hands it the same answers.

use messenger_notify::Outcome;
use std::sync::Mutex;
#[cfg(target_os = "ios")]
use std::time::Duration;

/// How long a VoIP push is given to be opened once the session is open:
/// the event inside, or fetched from its relay. Past it the placeholder
/// ends at once (`voip_end`), whatever the Swift side's own limit of the
/// life of an invitation (`VoipLimits.placeholder`, 45 s) has left.
#[cfg(target_os = "ios")]
const DESCRIBE_WITHIN: Duration = Duration::from_secs(7);

/// How long a VoIP push waits for the session (the app's PIN on a cold
/// start), and how often it looks: the Swift side ends the placeholder by
/// then anyway.
#[cfg(target_os = "ios")]
const SESSION_WITHIN: Duration = Duration::from_secs(45);
#[cfg(target_os = "ios")]
const SESSION_EVERY: Duration = Duration::from_millis(500);

/// The VoIP token PushKit gave this process, for the channel the push
/// registration brings (`commands::push::channel`).
static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// The plugin keeps the listener of tokens for as long as the app runs.
#[cfg(target_os = "ios")]
static TOKEN_LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The VoIP token of this process, if PushKit gave one.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn token() -> Option<String> {
    TOKEN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Keeps `token` (an empty one is none); `true` when it changed.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn keep_token(token: Option<String>) -> bool {
    let token = token.filter(|t| !t.is_empty());
    let mut slot = TOKEN.lock().unwrap_or_else(|e| e.into_inner());
    if *slot == token {
        return false;
    }
    *slot = token;
    true
}

/// What a VoIP push comes to.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The call it rings for.
    Ring(messenger_notify::CallNotice),
    /// The call is over already.
    Over,
    /// No call to ring, and why (for the log).
    Failed(String),
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn verdict(outcome: messenger_core::Result<Outcome>) -> Verdict {
    match outcome {
        Ok(Outcome::Call(call)) => Verdict::Ring(call),
        Ok(Outcome::CallEnd(_)) => Verdict::Over,
        Ok(Outcome::Quiet { reason }) => Verdict::Failed(format!("quiet: {reason:?}")),
        Ok(Outcome::Show(_) | Outcome::Plain(_)) => Verdict::Failed("a message, no call".into()),
        Err(e) => Verdict::Failed(e.to_string()),
    }
}

/// Follows the VoIP token: the one PushKit gave now, and every change,
/// into the channel the push server knows (re-registered when it
/// changed). Started with the push bridge, once the runtime runs.
#[cfg(target_os = "ios")]
pub(crate) async fn follow_token(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    let Ok(call) = super::bridge(app) else { return };
    if !TOKEN_LISTENING.swap(true, Ordering::SeqCst) {
        let tokens = app.clone();
        if let Err(e) = call
            .on_voip_token(move |token| {
                let app = tokens.clone();
                tauri::async_runtime::spawn(async move { took(&app, token).await });
            })
            .await
        {
            eprintln!("messenger call: cannot listen for VoIP tokens: {e}");
        }
    }
    match call.voip_token().await {
        Ok(token) => took(app, token).await,
        Err(e) => eprintln!("messenger call: no VoIP token: {e}"),
    }
}

/// A VoIP token, or none: kept for the next channel, and put into the
/// one the runtime keeps; told to the push server when that changed.
#[cfg(target_os = "ios")]
async fn took(app: &tauri::AppHandle, token: Option<String>) {
    if keep_token(token.clone()) {
        eprintln!("messenger call: PushKit {}", if token.is_some() { "gave a VoIP token" } else { "took the VoIP token back" });
    }
    let Some(rt) = super::runtime(app) else { return };
    match rt.push_set_voip_token(token).await {
        Ok(true) => {
            if let Err(e) = rt.push_reconcile(false).await {
                eprintln!("messenger push: {e}");
            }
        }
        Ok(false) => {}
        Err(e) => eprintln!("messenger push: the VoIP token was not kept: {e}"),
    }
}

/// Listens for VoIP pushes and takes those that came before: the push
/// that started the process came before any listener. Called once, with
/// the other listeners of the call bridge.
#[cfg(target_os = "ios")]
pub(super) async fn listen(app: &tauri::AppHandle, call: &tauri_plugin_veydan_call::VeydanCall<tauri::Wry>) {
    let pushes = app.clone();
    if let Err(e) = call
        .on_voip_push(move |push| {
            let app = pushes.clone();
            tauri::async_runtime::spawn(async move { answer(&app, push).await });
        })
        .await
    {
        eprintln!("messenger call: cannot listen for VoIP pushes: {e}");
    }
    match call.take_voip_pushes().await {
        Ok(waiting) => {
            for push in waiting {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { answer(&app, push).await });
            }
        }
        Err(e) => eprintln!("messenger call: the VoIP pushes before the start are lost: {e}"),
    }
}

/// Opens a VoIP push with the keys of the session; the outcome as the
/// push handler of Android has it. Built on a computer too, for the test
/// that its future may go to another thread.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
async fn describe(
    rt: &messenger_runtime::MessengerRuntime,
    data: &std::collections::BTreeMap<String, String>,
) -> messenger_core::Result<Outcome> {
    let push = messenger_notify::PushData::parse(data)?;
    // Only an invitation goes as VoIP (`["call","1"]` on the wrap); a
    // test of the API or anything else rings nothing.
    if !push.call {
        return Ok(Outcome::Quiet { reason: messenger_notify::Reason::NotACall });
    }
    let bundle = rt.notify_bundle().await?;
    messenger_notify::describe_as(rt.config(), bundle.as_ref(), &push).await
}

/// Waits for the session of the runtime: open at once, or at the unlock of
/// the app's PIN (the keys are behind it on a cold start). `false` after
/// [`SESSION_WITHIN`].
#[cfg(target_os = "ios")]
async fn session_opens(rt: &messenger_runtime::MessengerRuntime) -> bool {
    let start = std::time::Instant::now();
    loop {
        if rt.status().await.is_ok_and(|s| s.session_active) {
            return true;
        }
        if start.elapsed() >= SESSION_WITHIN {
            return false;
        }
        tokio::time::sleep(SESSION_EVERY).await;
    }
}

/// One VoIP push: opened, and its placeholder named as the call or ended.
#[cfg(target_os = "ios")]
async fn answer(app: &tauri::AppHandle, push: tauri_plugin_veydan_call::VoipPush) {
    use tauri_plugin_veydan_call::{VoipEnd, VoipRing};
    let Ok(call) = super::bridge(app) else { return };
    // The push's name in the server's log, for this one; nothing else of it.
    let trace: String = push.data.get("trace").map_or("-", String::as_str).chars().take(16).collect();
    // The listener lives as long as the app; the messenger may have stopped.
    let Some(rt) = super::runtime(app) else {
        eprintln!("messenger call: a VoIP push ({trace}) while the messenger does not run; ended");
        if let Err(e) = call.voip_end(&push.push_id, VoipEnd::Failed).await {
            eprintln!("messenger call: the placeholder of a VoIP push ({trace}) was not told: {e}");
        }
        return;
    };
    // Under the app's PIN the session opens at the unlock: the placeholder
    // rings nameless until then (decision 7), for the life of an
    // invitation at most, which the Swift side ends.
    if !session_opens(&rt).await {
        eprintln!("messenger call: a VoIP push ({trace}) and no session (the app's lock); left to its time");
        return;
    }
    let verdict = match tokio::time::timeout(DESCRIBE_WITHIN, describe(&rt, &push.data)).await {
        Ok(outcome) => verdict(outcome),
        Err(_) => Verdict::Failed(format!("not opened within {} s", DESCRIBE_WITHIN.as_secs())),
    };
    let done = match verdict {
        Verdict::Ring(notice) => {
            eprintln!("messenger call: a VoIP push ({trace}) rings the call {}", notice.call_id);
            let (name, hidden) = super::named(app, &rt, notice.name).await;
            let ring = VoipRing {
                push_id: push.push_id.clone(),
                call_id: notice.call_id,
                name,
                video: notice.media == "video",
                hide_on_lockscreen: hidden || notice.hide_on_lockscreen,
                expires_at: notice.expires_at,
            };
            call.voip_ring(&ring).await
        }
        Verdict::Over => {
            eprintln!("messenger call: a VoIP push ({trace}) of a call over already; ended");
            call.voip_end(&push.push_id, VoipEnd::RemoteEnded).await
        }
        Verdict::Failed(why) => {
            eprintln!("messenger call: a VoIP push ({trace}) rings no call ({why}); ended");
            call.voip_end(&push.push_id, VoipEnd::Failed).await
        }
    };
    if let Err(e) = done {
        eprintln!("messenger call: the placeholder of a VoIP push ({trace}) was not told: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_notify::{CallEnd, CallNotice, Reason};

    fn notice() -> CallNotice {
        CallNotice {
            call_id: "c1".into(),
            media: "video".into(),
            name: "Ann".into(),
            peer_key: "ab".repeat(32),
            picture: None,
            created_at: 1_800_000_000,
            expires_at: 1_800_000_045,
            hide_on_lockscreen: false,
        }
    }

    #[test]
    fn a_voip_push_rings_only_a_call_it_opens_to() {
        assert_eq!(verdict(Ok(Outcome::Call(notice()))), Verdict::Ring(notice()));
        assert_eq!(verdict(Ok(Outcome::CallEnd(CallEnd { call_id: "c1".into() }))), Verdict::Over);
        for reason in [Reason::NoKeys, Reason::NotACall, Reason::Muted] {
            assert!(matches!(verdict(Ok(Outcome::Quiet { reason })), Verdict::Failed(_)), "{reason:?}");
        }
        let broken = verdict(Err(messenger_core::MessengerError::Invalid("push v1".into())));
        assert!(matches!(broken, Verdict::Failed(ref why) if why.contains("push v1")), "{broken:?}");
    }

    /// The opening of a push runs in a task of its own (`answer`, spawned
    /// from the plugin's listener): its future must be `Send`. Checked
    /// here, where the iOS half cannot be compiled.
    #[allow(dead_code)]
    fn the_opening_of_a_push_may_run_on_any_thread(
        rt: &messenger_runtime::MessengerRuntime,
        data: &std::collections::BTreeMap<String, String>,
    ) {
        fn send<T: Send>(_: &T) {}
        send(&describe(rt, data));
    }

    #[test]
    fn the_voip_token_is_kept_once_and_an_empty_one_is_none() {
        assert!(keep_token(Some("aa".into())));
        assert_eq!(token().as_deref(), Some("aa"));
        assert!(!keep_token(Some("aa".into())), "the same token at every start");
        assert!(keep_token(Some("bb".into())));
        assert!(keep_token(Some(String::new())), "taken back");
        assert_eq!(token(), None);
        assert!(!keep_token(None));
    }
}

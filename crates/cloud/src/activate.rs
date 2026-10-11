// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The free plan and the keys of activation (cloud-api.md 3, 4.1, 4.2;
//! vcloud-plan.md 4.3, 4.5): a challenge, the work of the proof in a thread
//! of its own while the screen tells its progress, the activation. Asking
//! again is harmless: an account that exists is answered as it is.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use veydan_recovery::pow;
use veydan_shell::seams::{code, CloudError};

use crate::account;
use crate::api::{Account, Challenge, CloudApi, Pow, Purpose};

/// The event of the progress of an activation, with an [`ActivationView`].
pub const EVENT_ACTIVATION: &str = "cloud://activation";

/// A local refusal of the module, never the server's: the work of a proof
/// was stopped before it found a solution.
pub const CANCELLED: &str = "cancelled";

/// The longest a screen waits between two events of progress.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// Where an activation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing asked yet on this run of the app.
    Idle,
    /// Asking whether the account exists, and for a challenge.
    Asking,
    /// The work of the proof (Argon2id, cloud-api.md 3.2).
    Solving,
    /// The solution goes to the server.
    Activating,
    /// The account is there.
    Done,
    /// It failed; `error` says why.
    Failed,
}

/// What the screens show of an activation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActivationView {
    pub phase: Phase,
    /// Attempts of the work made so far.
    pub attempts: u64,
    /// About how many attempts the work takes: 2^d.
    pub expected: u64,
    /// The failure, when `phase` is `failed`.
    pub error: Option<CloudError>,
}

impl ActivationView {
    /// Nothing under way.
    pub const IDLE: Self = Self {
        phase: Phase::Idle,
        attempts: 0,
        expected: 0,
        error: None,
    };

    fn at(phase: Phase) -> Self {
        Self {
            phase,
            ..Self::IDLE
        }
    }
}

/// The work of a challenge to `d` leading zero bits (at least the `d` of
/// the challenge), in a blocking thread; `progress` hears the number of
/// every attempt.
pub(crate) async fn solve(
    challenge: &Challenge,
    d: u8,
    progress: impl Fn(u64) + Send + 'static,
    stop: Option<Arc<AtomicBool>>,
) -> Result<Pow, CloudError> {
    let bytes = pow::from_base64url(&challenge.challenge)
        .map_err(|_| CloudError::new("pow_invalid", "the challenge does not read"))?;
    let read = pow::Challenge::peek(&bytes)
        .map_err(|_| CloudError::new("pow_invalid", "the challenge does not read"))?;
    let d = d.max(read.d);
    let worked = tokio::task::spawn_blocking(move || {
        let mut attempts = 0u64;
        pow::solve_while(&bytes, d, &mut || {
            attempts += 1;
            progress(attempts);
            !stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed))
        })
    })
    .await
    .map_err(|e| CloudError::new(code::CRYPTO, format!("the work stopped: {e}")))?;
    match worked {
        Ok(Some(nonce)) => Ok(Pow {
            challenge: challenge.challenge.clone(),
            nonce,
        }),
        Ok(None) => Err(CloudError::new(CANCELLED, "the work was stopped")),
        // A difficulty past the ceiling is the server's `429` (3.3).
        Err(pow::PowError::Difficulty(d)) => Err(CloudError::new(
            "rate_limited",
            format!("the difficulty {d} is past the ceiling"),
        )),
        Err(e) => Err(CloudError::new(code::CRYPTO, e.to_string())),
    }
}

/// Activates the free plan for the identity (4.1), telling `report` where
/// it is: an account that exists is answered at once; else a challenge of
/// `activate`, its work, and `POST …/activate`. A challenge that ran out or
/// was spent meanwhile is taken anew once.
pub async fn activate_free(
    api: &dyn CloudApi,
    report: &(dyn Fn(ActivationView) + Send + Sync),
) -> Result<Account, CloudError> {
    report(ActivationView::at(Phase::Asking));
    match api.account().await {
        Ok(account) => return Ok(account),
        Err(e) if e.is("not_activated") => {}
        Err(e) => return Err(e),
    }
    activate_new(api, report, PROGRESS_EVERY).await
}

/// The activation of an identity the server has no account for: the
/// challenge, its work — its attempts told every `every` — and the request.
async fn activate_new(
    api: &dyn CloudApi,
    report: &(dyn Fn(ActivationView) + Send + Sync),
    every: Duration,
) -> Result<Account, CloudError> {
    let mut again = true;
    loop {
        let challenge = api.challenge(Purpose::Activate).await?;
        let expected = 1u64 << challenge.d.min(63);
        let solving = |attempts| ActivationView {
            phase: Phase::Solving,
            attempts,
            expected,
            error: None,
        };
        report(solving(0));
        let seen = Arc::new(AtomicU64::new(0));
        let counter = seen.clone();
        let work = solve(
            &challenge,
            challenge.d,
            move |n| counter.store(n, Ordering::Relaxed),
            None,
        );
        tokio::pin!(work);
        let pow = loop {
            tokio::select! {
                done = &mut work => break done?,
                _ = tokio::time::sleep(every) => report(solving(seen.load(Ordering::Relaxed))),
            }
        };
        report(ActivationView {
            phase: Phase::Activating,
            attempts: seen.load(Ordering::Relaxed),
            expected,
            error: None,
        });
        match api.activate(Some(pow)).await {
            Err(e) if again && (e.is("pow_expired") || e.is("pow_replayed")) => again = false,
            done => return done,
        }
    }
}

/// The activation of this run of the app: one at a time, its view for the
/// screens that open while it runs.
#[derive(Default)]
pub struct Activation {
    running: AtomicBool,
    view: Mutex<Option<ActivationView>>,
}

impl Activation {
    /// What the screens show now.
    pub fn view(&self) -> ActivationView {
        self.view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or(ActivationView::IDLE)
    }

    pub(crate) fn set(&self, view: ActivationView) {
        *self
            .view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(view);
    }

    /// Back to nothing, as after the account was deleted.
    pub fn reset(&self) {
        self.set(ActivationView::IDLE);
    }
}

/// Runs [`activate_free`] for the app: the view kept in [`Activation`],
/// sent as [`EVENT_ACTIVATION`] when the phase changes and at most every
/// [`PROGRESS_EVERY`] while the work goes; the account kept in the box.
pub async fn run<R: Runtime>(app: &AppHandle<R>) -> Result<Account, CloudError> {
    let api = crate::client(app);
    let state = app.state::<Activation>();
    let last = Mutex::new((Phase::Idle, Instant::now()));
    let report = |view: ActivationView| {
        state.set(view.clone());
        let mut last = last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.0 != view.phase || last.1.elapsed() >= PROGRESS_EVERY {
            *last = (view.phase, Instant::now());
            let _ = app.emit(EVENT_ACTIVATION, &view);
        }
    };
    let done = activate_free(api.as_ref(), &report).await;
    let end = match &done {
        Ok(account) => {
            // The box may be closed; the account is asked again next time.
            let _ = account::keep(app, account).await;
            ActivationView::at(Phase::Done)
        }
        Err(e) => ActivationView {
            error: Some(e.clone()),
            ..ActivationView::at(Phase::Failed)
        },
    };
    state.set(end.clone());
    let _ = app.emit(EVENT_ACTIVATION, &end);
    done
}

/// Starts the activation of the free plan in the background and answers at
/// once with where it is: onboarding calls it after «Veydan servers» and
/// the work goes while the next screen is read (plan 4.5). Called while one
/// runs, it answers that one.
#[tauri::command]
pub async fn cloud_activate_free<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ActivationView, CloudError> {
    let Some(state) = app.try_state::<Activation>() else {
        return Err(CloudError::new(
            code::NOT_SUPPORTED,
            "the module cloud is not set up",
        ));
    };
    if state.running.swap(true, Ordering::SeqCst) {
        return Ok(state.view());
    }
    state.set(ActivationView::at(Phase::Asking));
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run(&handle).await;
        if let Some(state) = handle.try_state::<Activation>() {
            state.running.store(false, Ordering::SeqCst);
        }
    });
    Ok(state.view())
}

/// A key of activation as the server reads it (4.2): upper case, `-` and
/// spaces dropped, `O→0`, `I/L→1`, `U→V`, exactly 20 characters of Crockford
/// base32; written back in groups of four. `None` is `key_malformed`.
pub fn normalize_key(text: &str) -> Option<String> {
    let mut plain = String::with_capacity(20);
    for c in text.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            'U' => 'V',
            other => other,
        };
        if !c.is_ascii() || !pow::CROCKFORD.contains(&(c as u8)) {
            return None;
        }
        plain.push(c);
    }
    if plain.len() != 20 {
        return None;
    }
    let groups: Vec<&str> = (0..5).map(|i| &plain[i * 4..i * 4 + 4]).collect();
    Some(groups.join("-"))
}

/// Redeems a key (4.2) in the contract's order: the account is asked
/// first (4.3), an identity without one is activated (4.1, the work in the
/// background of the screen), and only then the key goes to `/redeem`, so
/// no attempt of the 10 an hour (8.1) is spent on `not_activated`.
pub async fn redeem_key(
    api: &dyn CloudApi,
    key: &str,
    report: &(dyn Fn(ActivationView) + Send + Sync),
) -> Result<Account, CloudError> {
    let Some(key) = normalize_key(key) else {
        return Err(CloudError::new(
            "key_malformed",
            "a key of activation is 20 characters of Crockford base32",
        ));
    };
    match api.account().await {
        Ok(_) => {}
        Err(e) if e.is("not_activated") => {
            report(ActivationView::at(Phase::Asking));
            activate_new(api, report, PROGRESS_EVERY).await?;
        }
        Err(e) => return Err(e),
    }
    api.redeem(&key).await
}

/// Redeems a key of activation for the identity and keeps the account it
/// gives; the same key again answers the same (4.2).
#[tauri::command]
pub async fn cloud_redeem<R: Runtime>(
    app: AppHandle<R>,
    key: String,
) -> Result<Account, CloudError> {
    let api = crate::client(&app);
    let report = |view: ActivationView| {
        let _ = app.emit(EVENT_ACTIVATION, &view);
    };
    let account = redeem_key(api.as_ref(), &key, &report).await?;
    let _ = account::keep(&app, &account).await;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    #[test]
    fn a_key_is_read_as_the_server_reads_it() {
        assert_eq!(
            normalize_key("7k2m-abcd-efgh-jkmn-pqrs").as_deref(),
            Some("7K2M-ABCD-EFGH-JKMN-PQRS")
        );
        // Dictated: O, I, L and U, spaces instead of dashes.
        assert_eq!(
            normalize_key(" 7K2M ABCD EFGH JKMN PQRo ").as_deref(),
            Some("7K2M-ABCD-EFGH-JKMN-PQR0")
        );
        assert_eq!(
            normalize_key("ILUU-0000-0000-0000-0000").as_deref(),
            Some("11VV-0000-0000-0000-0000")
        );
        for bad in [
            "",
            "7K2M-ABCD-EFGH-JKMN",
            "7K2M-ABCD-EFGH-JKMN-PQRS-T",
            "7K2M-ABCD-EFGH-JKMN-PQR!",
            "7K2M-ABCD-EFGH-JKMN-PQRЖ",
        ] {
            assert_eq!(normalize_key(bad), None, "{bad}");
        }
    }

    /// The whole of an activation against the fake of /cloud/v1: no
    /// account, a challenge of d = 3, its work, the account; asked again,
    /// the account as it is, without new work.
    #[tokio::test]
    async fn the_free_plan_is_activated_once_and_answered_after() {
        let fake = Fake::start(3).await;
        let api = fake.client();
        let seen = Mutex::new(Vec::new());
        let report = |view: ActivationView| seen.lock().expect("seen").push(view.phase);
        let account = activate_free(&api, &report).await.expect("activated");
        assert_eq!(account.plan, "free");
        assert_eq!(account.npub, fake.npub());
        let mut phases = seen.lock().expect("seen").clone();
        phases.dedup();
        assert_eq!(
            phases,
            vec![Phase::Asking, Phase::Solving, Phase::Activating]
        );
        assert_eq!(fake.solved(), 1);
        seen.lock().expect("seen").clear();
        let again = activate_free(&api, &report).await.expect("again");
        assert_eq!(again, account);
        assert_eq!(*seen.lock().expect("seen"), vec![Phase::Asking]);
        assert_eq!(fake.solved(), 1);
    }

    /// A challenge spent meanwhile is taken anew once; a second time the
    /// word reaches the caller.
    #[tokio::test]
    async fn a_spent_challenge_is_taken_anew_once() {
        let fake = Fake::start(2).await;
        fake.refuse_next_activations("pow_replayed", 1);
        let api = fake.client();
        let account = activate_free(&api, &|_| {}).await.expect("activated");
        assert_eq!(account.state, crate::api::AccountState::Free);
        assert_eq!(fake.solved(), 2);

        let other = Fake::start(2).await;
        other.refuse_next_activations("pow_expired", 2);
        let failed = activate_free(&other.client(), &|_| {}).await;
        assert_eq!(failed.map_err(|e| e.code), Err("pow_expired".to_string()));
    }

    /// While the work goes, the screens hear its attempts: the view of
    /// `solving` carries them, not only the change of phase.
    #[tokio::test]
    async fn the_work_tells_its_attempts_while_it_goes() {
        let fake = Fake::start(3).await;
        let api = fake.client();
        let seen = Mutex::new(Vec::new());
        let report = |view: ActivationView| seen.lock().expect("seen").push(view);
        activate_new(&api, &report, Duration::from_millis(1))
            .await
            .expect("activated");
        let seen = seen.lock().expect("seen");
        let solving: Vec<&ActivationView> =
            seen.iter().filter(|v| v.phase == Phase::Solving).collect();
        assert!(solving.iter().all(|v| v.expected == 8), "{solving:?}");
        assert!(solving.iter().any(|v| v.attempts > 0), "{solving:?}");
        let last = seen.last().expect("last");
        assert_eq!(last.phase, Phase::Activating);
        assert!(last.attempts > 0);
    }

    /// A stopped work is the module's own word, not one of the contract.
    #[tokio::test]
    async fn a_stopped_work_is_cancelled() {
        let fake = Fake::start(8).await;
        let challenge = fake
            .client()
            .challenge(Purpose::Activate)
            .await
            .expect("challenge");
        let stop = Arc::new(AtomicBool::new(true));
        let stopped = solve(&challenge, challenge.d, |_| {}, Some(stop)).await;
        assert_eq!(stopped.map_err(|e| e.code), Err(CANCELLED.to_string()));
    }

    /// The account is asked first; an identity without one is activated,
    /// and the key goes to `/redeem` once, after. With an account, only the
    /// question and the key.
    #[tokio::test]
    async fn a_key_activates_an_identity_without_an_account_and_is_idempotent() {
        let fake = Fake::start(2).await;
        let api = fake.client();
        let account = redeem_key(&api, "7k2m abcd efgh jkmn pqrs", &|_| {})
            .await
            .expect("redeemed");
        assert_eq!(account.plan, "plus");
        assert_eq!(account.subscription.map(|s| s.source), Some("key:1".into()));
        assert_eq!(
            fake.log(),
            vec![
                "GET /cloud/v1/account",
                "GET /cloud/v1/challenge?purpose=activate",
                "POST /cloud/v1/activate",
                "POST /cloud/v1/redeem",
            ]
        );
        let again = redeem_key(&api, "7K2M-ABCD-EFGH-JKMN-PQRS", &|_| {})
            .await
            .expect("again");
        assert_eq!(again.plan, "plus");
        assert_eq!(
            fake.log()[4..],
            ["GET /cloud/v1/account", "POST /cloud/v1/redeem"]
        );
        assert_eq!(
            redeem_key(&api, "7K2M-ABCD-EFGH-JKMN-PQRT", &|_| {})
                .await
                .map_err(|e| e.code),
            Err("key_unknown".to_string())
        );
        assert_eq!(
            redeem_key(&api, "nope", &|_| {}).await.map_err(|e| e.code),
            Err("key_malformed".to_string())
        );
    }

    /// The command answers at once; the work goes on in the background and
    /// ends in the box and in `done`.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_command_starts_the_work_and_answers_at_once() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        let first = cloud_activate_free(app.handle().clone())
            .await
            .expect("view");
        assert!(matches!(
            first.phase,
            Phase::Asking | Phase::Solving | Phase::Activating
        ));
        let state = app.state::<Activation>();
        for _ in 0..600 {
            if state.view().phase == Phase::Done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(state.view().phase, Phase::Done);
        let kept = account::load(&app.handle().clone())
            .await
            .expect("load")
            .expect("kept");
        assert_eq!(kept.plan, "free");
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The check for a new version of the product (internal/platform-spec.md 14.3).
//!
//! A computer has the Tauri updater; a phone has none, so the shell reads the
//! `latest.json` the updater would read — the first endpoint of
//! `plugins.updater` in the product's Tauri config, which points at the
//! product's own repository (`Veydan-<Product>/releases/latest/download/
//! latest.json`) — and compares its version with the product's. The UI of
//! the phone shows the answer in Settings and links to the release page,
//! where the signed APK is.
//!
//! The check runs at most once a day by itself (`update_check` with
//! `manual: false`, called by the UI at start-up), never by itself on a
//! metered network where the platform tells (Android), and always when the
//! user asks (`manual: true`). The time and the answer of the last check are
//! the local setting `update_checked`; sync does not carry it.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::{SystemTime, UNIX_EPOCH};
use veydan_core::{settings, AppError, CmdResult, Core};

/// The local setting: [`Stored`] as JSON.
pub(crate) const KEY: &str = "update_checked";
/// How often the check runs by itself.
pub(crate) const INTERVAL_SECS: u64 = 24 * 60 * 60;

/// What the last successful check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stored {
    /// Seconds since the Unix epoch.
    pub at: u64,
    /// The version `latest.json` offered.
    pub latest: String,
}

/// Why a check did not ask the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Skipped {
    /// The last check is less than a day old.
    Recent,
    /// The active network is metered.
    Metered,
}

/// The answer of `update_check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateCheck {
    /// The version of this build.
    pub current: String,
    /// The version the product's `latest.json` offers, as of `checked_at`.
    pub latest: Option<String>,
    /// `latest` is newer than `current`.
    pub available: bool,
    /// When `latest` was read (seconds since the Unix epoch).
    pub checked_at: Option<u64>,
    /// The page of the release that has `latest`, to download it from.
    pub page: Option<String>,
    /// Set when this call did not ask the network.
    pub skipped: Option<Skipped>,
}

/// Reads a URL. The real one is [`Http`]; tests give a fake.
pub(crate) trait Fetcher {
    fn get(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send;
}

/// What a check works with, apart from the network.
pub(crate) struct Input<'a> {
    pub endpoint: &'a str,
    pub current: &'a str,
    pub stored: Option<Stored>,
    pub now: u64,
    pub manual: bool,
    /// The active network is metered; `None` where the platform does not tell.
    pub metered: Option<bool>,
}

/// A version of `latest.json` or of the app: `X.Y.Z`, an optional leading
/// `v`, pre-release and build parts as semver has them.
fn version(text: &str) -> Option<semver::Version> {
    semver::Version::parse(text.trim().trim_start_matches('v')).ok()
}

/// `offered` is newer than `current` (semver precedence, as the Tauri updater
/// decides). An unreadable version is never newer.
pub(crate) fn is_newer(current: &str, offered: &str) -> bool {
    match (version(current), version(offered)) {
        (Some(current), Some(offered)) => offered > current,
        _ => false,
    }
}

/// Whether a check that the user did not ask for may ask the network now.
pub(crate) fn due(stored: Option<&Stored>, now: u64) -> bool {
    match stored {
        None => true,
        // A clock that went back does not hold the check off for long.
        Some(last) => now < last.at || now - last.at >= INTERVAL_SECS,
    }
}

/// The release page of version `latest` in the repository of a GitHub
/// endpoint `https://github.com/<owner>/<repo>/releases/latest/download/…`;
/// `None` for an endpoint of another shape.
pub(crate) fn release_page(endpoint: &str, latest: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://github.com/")?;
    let mut parts = rest.splitn(3, '/');
    let (owner, repo, tail) = (parts.next()?, parts.next()?, parts.next()?);
    if owner.is_empty() || repo.is_empty() || !tail.starts_with("releases/") {
        return None;
    }
    let tag = format!("v{}", latest.trim().trim_start_matches('v'));
    Some(format!("https://github.com/{owner}/{repo}/releases/tag/{tag}"))
}

/// The version a `latest.json` offers.
pub(crate) fn offered(json: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Latest {
        version: String,
    }
    let latest: Latest = serde_json::from_str(json).map_err(|e| format!("latest.json: {e}"))?;
    let v = version(&latest.version).ok_or_else(|| format!("latest.json: '{}' is not a version", latest.version))?;
    Ok(v.to_string())
}

fn answer(input: &Input<'_>, stored: Option<&Stored>, skipped: Option<Skipped>) -> UpdateCheck {
    let latest = stored.map(|s| s.latest.clone());
    UpdateCheck {
        current: input.current.to_string(),
        available: latest.as_deref().is_some_and(|l| is_newer(input.current, l)),
        page: latest.as_deref().and_then(|l| release_page(input.endpoint, l)),
        checked_at: stored.map(|s| s.at),
        latest,
        skipped,
    }
}

/// One check: the answer, and what to store when the network was asked.
/// A check the user did not ask for that fails answers with what is stored;
/// one the user asked for reports the failure.
pub(crate) async fn check(input: Input<'_>, fetcher: &impl Fetcher) -> Result<(UpdateCheck, Option<Stored>), String> {
    if !input.manual {
        if !due(input.stored.as_ref(), input.now) {
            return Ok((answer(&input, input.stored.as_ref(), Some(Skipped::Recent)), None));
        }
        if input.metered == Some(true) {
            return Ok((answer(&input, input.stored.as_ref(), Some(Skipped::Metered)), None));
        }
    }
    let fetched = match fetcher.get(input.endpoint).await {
        Ok(body) => offered(&body),
        Err(e) => Err(e),
    };
    match fetched {
        Ok(latest) => {
            let stored = Stored { at: input.now, latest };
            Ok((answer(&input, Some(&stored), None), Some(stored)))
        }
        Err(e) if input.manual => Err(e),
        Err(_) => Ok((answer(&input, input.stored.as_ref(), None), None)),
    }
}

/// The first updater endpoint of the product's Tauri config.
fn endpoint<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")?
        .get("endpoints")?
        .get(0)?
        .as_str()
        .map(str::to_string)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The network, through the HTTP client of sync: timeouts, and on Android
/// the bundled root certificates and the IPv4-first resolver.
struct Http;

impl Fetcher for Http {
    async fn get(&self, url: &str) -> Result<String, String> {
        let client = veydan_sync::storage::http_client().map_err(|e| e.to_string())?;
        let response = client.get(url).send().await.map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("{url}: HTTP {status}"));
        }
        response.text().await.map_err(|e| e.to_string())
    }
}

/// Whether the active network is metered: Android's
/// `ConnectivityManager.isActiveNetworkMetered()` (the permission
/// ACCESS_NETWORK_STATE comes with the shell's Android library). `None`
/// where the platform does not tell — every computer.
#[cfg(target_os = "android")]
fn metered() -> Option<bool> {
    use jni::objects::JValue;
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::wry::prelude::dispatch(move |env, activity, _webview| {
        let mut ask = || -> jni::errors::Result<bool> {
            let name = env.new_string("connectivity")?;
            let manager = env
                .call_method(
                    activity,
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            if manager.is_null() {
                return Ok(false);
            }
            env.call_method(&manager, "isActiveNetworkMetered", "()Z", &[])?.z()
        };
        let answer = ask().ok();
        if answer.is_none() {
            let _ = env.exception_clear();
        }
        let _ = tx.send(answer);
    });
    rx.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
}

#[cfg(not(target_os = "android"))]
fn metered() -> Option<bool> {
    None
}

/// Check the product's `latest.json`. `manual: false` is the check of the
/// start: at most once a day, not on a metered network, silent on failure.
#[tauri::command]
pub async fn update_check(
    manual: bool,
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
) -> CmdResult<UpdateCheck> {
    let endpoint = endpoint(&app).ok_or_else(|| AppError::other("the product has no updater endpoint"))?;
    let current = app.package_info().version.to_string();
    let stored = settings::get_json::<Stored>(&core.db, KEY).await;
    let input = Input {
        endpoint: &endpoint,
        current: &current,
        stored,
        now: now(),
        manual,
        metered: if manual { None } else { metered() },
    };
    let (answer, store) = check(input, &Http).await.map_err(AppError::other)?;
    if let Some(store) = store {
        settings::set_json(&core.db, KEY, &store).await?;
    }
    Ok(answer)
}

/// Open the release page of the version the last check found (the latest
/// release when there was none) in the system browser. The page is the
/// product's own; the UI cannot name another address.
#[tauri::command]
pub async fn update_open(app: tauri::AppHandle, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let endpoint = endpoint(&app).ok_or_else(|| AppError::other("the product has no updater endpoint"))?;
    let stored = settings::get_json::<Stored>(&core.db, KEY).await;
    let page = stored
        .and_then(|s| release_page(&endpoint, &s.latest))
        .or_else(|| endpoint.split_once("/latest/download/").map(|(base, _)| format!("{base}/latest")))
        .ok_or_else(|| AppError::other("the updater endpoint is not a GitHub release"))?;
    open(&app, &page)
}

#[cfg(desktop)]
fn open<R: tauri::Runtime>(app: &tauri::AppHandle<R>, url: &str) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(AppError::other)
}

/// An `ACTION_VIEW` intent: the browser of the phone opens the page.
#[cfg(target_os = "android")]
fn open<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, url: &str) -> CmdResult<()> {
    use jni::objects::JValue;
    let (tx, rx) = std::sync::mpsc::channel();
    let url = url.to_string();
    tauri::wry::prelude::dispatch(move |env, activity, _webview| {
        let mut view = || -> jni::errors::Result<()> {
            let text = env.new_string(&url)?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&text)],
                )?
                .l()?;
            let action = env.new_string("android.intent.action.VIEW")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        };
        let result = view().map_err(|e| e.to_string());
        if result.is_err() {
            let _ = env.exception_clear();
        }
        let _ = tx.send(result);
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| AppError::other("the activity did not answer"))?
        .map_err(AppError::other)
}

#[cfg(all(mobile, not(target_os = "android")))]
fn open<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, _url: &str) -> CmdResult<()> {
    Err(AppError::other("not available on this platform"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ENDPOINT: &str = "https://github.com/veydanproject/Veydan-Notes/releases/latest/download/latest.json";
    const DAY: u64 = INTERVAL_SECS;

    /// Answers every request with the same body (or error) and counts them.
    struct Fake {
        answer: Result<String, String>,
        asked: AtomicUsize,
    }

    impl Fake {
        fn offering(version: &str) -> Self {
            Self {
                answer: Ok(format!(
                    r#"{{"version":"{version}","notes":"n","pub_date":"2026-10-03T00:00:00.000Z","platforms":{{}}}}"#
                )),
                asked: AtomicUsize::new(0),
            }
        }
        fn failing() -> Self {
            Self { answer: Err("offline".into()), asked: AtomicUsize::new(0) }
        }
        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
    }

    impl Fetcher for Fake {
        async fn get(&self, url: &str) -> Result<String, String> {
            assert_eq!(url, ENDPOINT);
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.answer.clone()
        }
    }

    fn input(stored: Option<Stored>, now: u64, manual: bool, metered: Option<bool>) -> Input<'static> {
        Input { endpoint: ENDPOINT, current: "5.0.0", stored, now, manual, metered }
    }

    fn stored(at: u64, latest: &str) -> Option<Stored> {
        Some(Stored { at, latest: latest.into() })
    }

    fn run<T>(future: impl Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn versions_compare_as_semver() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.9", "1.0.10"));
        assert!(is_newer("1.9.0", "2.0.0"));
        assert!(is_newer("1.0.0", "v1.1.0"));
        assert!(is_newer("1.0.0-alpha.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("5.0.0", "4.0.7"));
        assert!(!is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("1.0.0", "latest"));
        assert!(!is_newer("garbage", "2.0.0"));
    }

    #[test]
    fn the_check_by_itself_runs_once_a_day() {
        assert!(due(None, 1000));
        assert!(!due(stored(1000, "5.0.0").as_ref(), 1000));
        assert!(!due(stored(1000, "5.0.0").as_ref(), 1000 + DAY - 1));
        assert!(due(stored(1000, "5.0.0").as_ref(), 1000 + DAY));
        // The clock went back: the stored time is in the future.
        assert!(due(stored(5000, "5.0.0").as_ref(), 1000));
    }

    #[test]
    fn the_release_page_is_the_tag_of_the_product_repository() {
        assert_eq!(
            release_page(ENDPOINT, "1.2.3").as_deref(),
            Some("https://github.com/veydanproject/Veydan-Notes/releases/tag/v1.2.3")
        );
        assert_eq!(
            release_page(ENDPOINT, "v1.2.3").as_deref(),
            Some("https://github.com/veydanproject/Veydan-Notes/releases/tag/v1.2.3")
        );
        assert_eq!(release_page("https://example.com/latest.json", "1.2.3"), None);
        assert_eq!(release_page("https://github.com/only-owner", "1.2.3"), None);
    }

    #[test]
    fn latest_json_gives_its_version() {
        assert_eq!(offered(r#"{"version":"v1.2.0","platforms":{}}"#), Ok("1.2.0".into()));
        assert!(offered(r#"{"version":"soon"}"#).is_err());
        assert!(offered("<html>").is_err());
    }

    #[test]
    fn a_first_check_asks_and_stores_what_it_found() {
        let fake = Fake::offering("5.1.0");
        let (answer, store) = run(check(input(None, 100, false, Some(false)), &fake)).unwrap();
        assert_eq!(fake.asked(), 1);
        assert_eq!(store, stored(100, "5.1.0"));
        assert_eq!(
            answer,
            UpdateCheck {
                current: "5.0.0".into(),
                latest: Some("5.1.0".into()),
                available: true,
                checked_at: Some(100),
                page: Some("https://github.com/veydanproject/Veydan-Notes/releases/tag/v5.1.0".into()),
                skipped: None,
            }
        );
    }

    #[test]
    fn within_a_day_the_check_by_itself_answers_from_the_last_one() {
        let fake = Fake::offering("9.9.9");
        let (answer, store) = run(check(input(stored(100, "5.1.0"), 100 + DAY - 1, false, Some(false)), &fake)).unwrap();
        assert_eq!(fake.asked(), 0);
        assert_eq!(store, None);
        assert_eq!(answer.skipped, Some(Skipped::Recent));
        assert_eq!(answer.latest.as_deref(), Some("5.1.0"));
        assert!(answer.available);
        assert_eq!(answer.checked_at, Some(100));
    }

    #[test]
    fn after_a_day_it_asks_again() {
        let fake = Fake::offering("5.0.0");
        let (answer, store) = run(check(input(stored(100, "4.9.0"), 100 + DAY, false, None), &fake)).unwrap();
        assert_eq!(fake.asked(), 1);
        assert_eq!(store, stored(100 + DAY, "5.0.0"));
        assert!(!answer.available);
        assert_eq!(answer.skipped, None);
    }

    #[test]
    fn a_metered_network_holds_the_check_by_itself_but_not_the_users() {
        let fake = Fake::offering("5.1.0");
        let (answer, store) = run(check(input(None, 100, false, Some(true)), &fake)).unwrap();
        assert_eq!((fake.asked(), store, answer.skipped), (0, None, Some(Skipped::Metered)));
        assert!(!answer.available);

        let (answer, store) = run(check(input(None, 100, true, Some(true)), &fake)).unwrap();
        assert_eq!(fake.asked(), 1);
        assert_eq!(store, stored(100, "5.1.0"));
        assert!(answer.available);
    }

    #[test]
    fn the_user_asks_even_right_after_a_check() {
        let fake = Fake::offering("5.2.0");
        let (answer, store) = run(check(input(stored(100, "5.1.0"), 101, true, None), &fake)).unwrap();
        assert_eq!(fake.asked(), 1);
        assert_eq!(store, stored(101, "5.2.0"));
        assert_eq!(answer.latest.as_deref(), Some("5.2.0"));
    }

    #[test]
    fn a_failure_is_silent_by_itself_and_reported_to_the_user() {
        let fake = Fake::failing();
        let (answer, store) = run(check(input(stored(100, "5.1.0"), 100 + DAY, false, None), &fake)).unwrap();
        assert_eq!(fake.asked(), 1);
        // Nothing stored: the next start tries again.
        assert_eq!(store, None);
        assert_eq!(answer.latest.as_deref(), Some("5.1.0"));
        assert_eq!(answer.skipped, None);

        assert_eq!(run(check(input(None, 100, true, None), &fake)), Err("offline".into()));
    }

    #[test]
    fn a_newer_build_than_the_release_offers_nothing() {
        let fake = Fake::offering("4.0.7");
        let mut i = input(None, 100, true, None);
        i.current = "5.0.0";
        let (answer, _) = run(check(i, &fake)).unwrap();
        assert!(!answer.available);
    }
}

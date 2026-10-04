// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::models::{Profile, Proxy};

pub fn generate(profile: &Profile, proxy: Option<&Proxy>) -> String {
    let mut prefs: Vec<String> = vec![
        // Basic privacy
        pref_bool("privacy.resistFingerprinting", false),
        pref_bool("privacy.trackingprotection.enabled", true),
        pref_bool("geo.enabled", profile.geolocation_enabled),
        // Disable cursor activity indicator (camoufox red dot under cursor)
        pref_bool("ui.use_activity_cursor", false),
        // Disable telemetry
        pref_bool("toolkit.telemetry.enabled", false),
        pref_bool("datareporting.healthreport.uploadEnabled", false),
        pref_bool("browser.send_additional_telemetry_data", false),
        // Disable auto-update
        pref_bool("app.update.auto", false),
        pref_bool("app.update.enabled", false),
        // Disable crash reporter
        pref_bool("breakpad.reportURL", false),
        // First run
        pref_bool("browser.firstrun.show.localepicker", false),
        pref_bool("browser.firstrun.show.uidiscovery", false),
        // Session restore
        pref_bool("browser.sessionstore.resume_from_crash", false),
        // History: places (sidebar) + session (back/forward buttons)
        pref_bool("places.history.enabled", profile.history_enabled),
        pref_bool(
            "browser.privatebrowsing.autostart",
            !profile.history_enabled,
        ),
        pref_int(
            "browser.sessionhistory.max_entries",
            if profile.history_enabled { 50 } else { 0 },
        ),
    ];

    // Startup page
    prefs.push(pref_string("browser.startup.homepage", "about:blank"));
    prefs.push(pref_int("browser.startup.page", 0));

    // User-Agent override
    let ua = profile
        .user_agent
        .as_deref()
        .or_else(|| {
            crate::fingerprint::get_preset(&profile.fingerprint_preset).map(|p| p.user_agent)
        })
        .unwrap_or("Mozilla/5.0 (X11; Linux x86_64; rv:127.0) Gecko/20100101 Firefox/127.0");

    prefs.push(pref_string("general.useragent.override", ua));

    // Platform
    let platform = profile
        .platform
        .as_deref()
        .or_else(|| crate::fingerprint::get_preset(&profile.fingerprint_preset).map(|p| p.platform))
        .unwrap_or("Linux x86_64");

    prefs.push(pref_string("general.platform.override", platform));

    // Languages
    prefs.push(pref_string("intl.accept_languages", &profile.languages));

    // Locale
    prefs.push(pref_string("intl.locale.requested", &profile.locale));

    // WebRTC
    match profile.webrtc_mode.as_str() {
        "disable" => {
            prefs.push(pref_bool("media.peerconnection.enabled", false));
        }
        "proxy_only" => {
            prefs.push(pref_bool("media.peerconnection.enabled", true));
            prefs.push(pref_bool("media.peerconnection.ice.no_host", true));
            prefs.push(pref_bool(
                "media.peerconnection.ice.default_address_only",
                true,
            ));
        }
        "real_ip" => {
            prefs.push(pref_bool("media.peerconnection.enabled", true));
            prefs.push(pref_bool("media.peerconnection.ice.no_host", false));
            prefs.push(pref_bool(
                "media.peerconnection.ice.default_address_only",
                false,
            ));
        }
        _ => {}
    }

    // Geolocation coordinates
    if profile.geolocation_enabled {
        if let (Some(lat), Some(lng)) = (profile.latitude, profile.longitude) {
            let geo_url = format!(
                "data:application/json,{{\"location\":{{\"lat\":{lat},\"lng\":{lng},\"accuracy\":100}}}}"
            );
            prefs.push(pref_string("geo.provider.network.url", &geo_url));
        }
    }

    // Force WebGL: Camoufox on Linux с Win32-профилем блокирует WebGL через
    // внутренний флаг WebglAllowWindowsNativeGl:false. Принудительно включаем.
    prefs.push(pref_bool("webgl.force-enabled", true));
    prefs.push(pref_bool("webgl.disabled", false));

    // Camoufox-specific UI fixes
    if profile.browser_type == "camoufox" {
        // CSD mode: Firefox draws its own window controls in the tab bar (no separate OS title bar)
        prefs.push(pref_int("browser.tabs.inTitlebar", 1));
        prefs.push(pref_bool(
            "toolkit.legacyUserProfileCustomizations.stylesheets",
            true,
        ));
        // Show bookmarks toolbar always
        prefs.push(pref_string(
            "browser.toolbars.bookmarks.visibility",
            "always",
        ));
    }

    // Search engine display name: set in user.js so it overrides any stale
    // value in prefs.js. The SearchService uses search.json.mozlz4 for actual
    // engine selection; this pref controls what the browser UI displays.
    if profile.browser_type == "camoufox" {
        let engine_name = search_engine_display_name(&profile.default_search_engine);
        prefs.push(pref_string("browser.search.defaultenginename", engine_name));
        prefs.push(pref_string(
            "browser.search.defaultenginename.private",
            engine_name,
        ));
        // Address bar placeholder text — set here so it matches the engine immediately
        // on first launch (SearchService updates it async, causing stale display otherwise)
        prefs.push(pref_string("browser.urlbar.placeholderName", engine_name));
        prefs.push(pref_string(
            "browser.urlbar.placeholderName.private",
            engine_name,
        ));
    }

    // Proxy
    if let Some(proxy) = proxy {
        apply_proxy_prefs(&mut prefs, proxy);
    } else {
        prefs.push(pref_int("network.proxy.type", 0));
        prefs.push(pref_string("network.proxy.http", ""));
        prefs.push(pref_int("network.proxy.http_port", 0));
        prefs.push(pref_string("network.proxy.ssl", ""));
        prefs.push(pref_int("network.proxy.ssl_port", 0));
        prefs.push(pref_string("network.proxy.socks", ""));
        prefs.push(pref_int("network.proxy.socks_port", 0));
    }

    // Sideloaded unsigned notes extension (firefox-profile/extensions/*.xpi)
    prefs.push(pref_bool("xpinstall.signatures.required", false));
    prefs.push(pref_int("extensions.autoDisableScopes", 0));
    prefs.push(pref_int("extensions.enabledScopes", 5));

    prefs.join("\n")
}

fn apply_proxy_prefs(prefs: &mut Vec<String>, proxy: &Proxy) {
    match proxy.proxy_type.as_str() {
        "http" | "https" => {
            prefs.push(pref_int("network.proxy.type", 1));
            prefs.push(pref_string("network.proxy.http", &proxy.host));
            prefs.push(pref_int("network.proxy.http_port", proxy.port));
            prefs.push(pref_string("network.proxy.ssl", &proxy.host));
            prefs.push(pref_int("network.proxy.ssl_port", proxy.port));
            prefs.push(pref_bool("network.proxy.share_proxy_settings", true));
            // Явно сбрасываем SOCKS чтобы стареющий prefs.js не перехватывал трафик
            prefs.push(pref_string("network.proxy.socks", ""));
            prefs.push(pref_int("network.proxy.socks_port", 0));
        }
        "socks5" => {
            prefs.push(pref_int("network.proxy.type", 1));
            prefs.push(pref_string("network.proxy.socks", &proxy.host));
            prefs.push(pref_int("network.proxy.socks_port", proxy.port));
            prefs.push(pref_int("network.proxy.socks_version", 5));
            // remote DNS: hostname уходит в прокси, прокси сам резолвит (избегает IPv6-проблем и DNS-утечек)
            prefs.push(pref_bool("network.proxy.socks_remote_dns", true));
            // Явно сбрасываем HTTP/SSL чтобы стареющий prefs.js не перехватывал трафик
            prefs.push(pref_string("network.proxy.http", ""));
            prefs.push(pref_int("network.proxy.http_port", 0));
            prefs.push(pref_string("network.proxy.ssl", ""));
            prefs.push(pref_int("network.proxy.ssl_port", 0));
            prefs.push(pref_bool("network.proxy.share_proxy_settings", false));
            if let Some(u) = &proxy.username {
                if !u.is_empty() {
                    prefs.push(pref_string("network.proxy.socks_username", u));
                }
            }
            if let Some(p) = &proxy.password {
                if !p.is_empty() {
                    prefs.push(pref_string("network.proxy.socks_password", p));
                }
            }
        }
        // On the launch path `setup_proxy` has already wrapped every supported
        // type in a local HTTP proxy, so only "http" reaches this match and an
        // unhandled type is rejected before launch. Emitting nothing here would
        // mean no proxy prefs at all — a silent direct connection — so this arm
        // must stay unreachable for anything that actually launches.
        _ => {}
    }
}

/// Generates per-profile userChrome.css.
/// Contains only the profile-specific color stripe indicator for #TabsToolbar.
/// All structural UI overrides live in the shared chrome.css (patch_chrome_css).
/// userChrome.css is loaded after chrome.css so !important here wins over chrome.css.
pub fn camoufox_user_chrome(color_left: &str, color_right: &str, label: &str) -> String {
    use crate::commands::camoufox::{build_stripe_bg, STRIPE_HEIGHT};
    let stripe_bg = build_stripe_bg(color_left, color_right, label);
    format!(
        r#"/* Veydan Space: per-profile color indicator — overrides chrome.css #TabsToolbar */
#TabsToolbar {{
  background-image: {stripe_bg} !important;
  background-size: 100% {STRIPE_HEIGHT}px !important;
  background-position: top !important;
  background-repeat: no-repeat !important;
}}
"#
    )
}

fn pref_string(key: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("user_pref(\"{key}\", \"{escaped}\");")
}

/// Profile locale options. Same order as `ui/src/lib/core/locales.ts`: a bare language
/// code (`ru`) maps to the first matching entry (`ru-RU`).
const LOCALE_OPTIONS: &[(&str, &str)] = &[
    ("en-US", "en-US,en"),
    ("en-GB", "en-GB,en"),
    ("ru-RU", "ru-RU,ru"),
    ("de-DE", "de-DE,de"),
    ("fr-FR", "fr-FR,fr"),
    ("es-ES", "es-ES,es"),
    ("es-MX", "es-MX,es"),
    ("pt-BR", "pt-BR,pt"),
    ("pt-PT", "pt-PT,pt"),
    ("it-IT", "it-IT,it"),
    ("nl-NL", "nl-NL,nl"),
    ("pl-PL", "pl-PL,pl"),
    ("tr-TR", "tr-TR,tr"),
    ("uk-UA", "uk-UA,uk"),
    ("zh-CN", "zh-CN,zh"),
    ("zh-TW", "zh-TW,zh"),
    ("ja-JP", "ja-JP,ja"),
    ("ko-KR", "ko-KR,ko"),
    ("ar-SA", "ar-SA,ar"),
    ("fa-IR", "fa-IR,fa"),
    ("hi-IN", "hi-IN,hi"),
    ("id-ID", "id-ID,id"),
    ("vi-VN", "vi-VN,vi"),
    ("th-TH", "th-TH,th"),
];

/// Maps a Firefox `intl.locale.requested` value to the profile locale and its languages list.
pub fn match_locale(requested: &str) -> Option<(&'static str, &'static str)> {
    let requested = requested.trim();
    if requested.is_empty() {
        return None;
    }
    if let Some(hit) = LOCALE_OPTIONS
        .iter()
        .find(|(locale, _)| locale.eq_ignore_ascii_case(requested))
    {
        return Some(*hit);
    }
    let prefix = requested.split(['-', '_']).next().unwrap_or(requested);
    LOCALE_OPTIONS
        .iter()
        .find(|(locale, _)| {
            locale
                .split('-')
                .next()
                .is_some_and(|lang| lang.eq_ignore_ascii_case(prefix))
        })
        .copied()
}

/// Last string value of `key` in a prefs.js file. Firefox appends overrides, so the last one wins.
pub fn read_pref_string(content: &str, key: &str) -> Option<String> {
    let needle = format!("user_pref(\"{key}\", \"");
    let mut last = None;
    let mut rest = content;
    while let Some(idx) = rest.find(&needle) {
        let after = &rest[idx + needle.len()..];
        match take_js_string(after) {
            Some((raw, consumed)) => {
                last = Some(raw);
                rest = &after[consumed..];
            }
            None => break,
        }
    }
    last
}

/// Decodes a JS string body and returns how many bytes of `input` it consumed, including the closing quote.
fn take_js_string(input: &str) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut chars = input.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            let (_, next) = chars.next()?;
            out.push(next);
            continue;
        }
        if c == '"' {
            return Some((out, i + c.len_utf8()));
        }
        out.push(c);
    }
    None
}

fn pref_bool(key: &str, value: bool) -> String {
    format!("user_pref(\"{key}\", {value});")
}

fn pref_int(key: &str, value: i64) -> String {
    format!("user_pref(\"{key}\", {value});")
}

pub fn search_engine_display_name(identifier: &str) -> &'static str {
    match identifier {
        "google" => "Google",
        "brave" => "Brave Search",
        "startpage" => "Startpage",
        _ => "DuckDuckGo",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Profile, Proxy};

    #[test]
    fn webrtc_disable_turns_off_peerconnection() {
        let mut p = Profile::test_default();
        p.webrtc_mode = "disable".into();
        let js = generate(&p, None);
        assert!(js.contains("user_pref(\"media.peerconnection.enabled\", false);"));
    }

    #[test]
    fn webrtc_proxy_only_masks_host_candidates() {
        let mut p = Profile::test_default();
        p.webrtc_mode = "proxy_only".into();
        let js = generate(&p, None);
        assert!(js.contains("user_pref(\"media.peerconnection.enabled\", true);"));
        assert!(js.contains("user_pref(\"media.peerconnection.ice.no_host\", true);"));
        assert!(js.contains("user_pref(\"media.peerconnection.ice.default_address_only\", true);"));
    }

    #[test]
    fn socks5_proxy_enables_remote_dns() {
        let p = Profile::test_default();
        let mut proxy = Proxy::test_default();
        proxy.proxy_type = "socks5".into();
        proxy.host = "10.0.0.1".into();
        proxy.port = 9050;
        let js = generate(&p, Some(&proxy));
        assert!(js.contains("user_pref(\"network.proxy.socks\", \"10.0.0.1\");"));
        assert!(js.contains("user_pref(\"network.proxy.socks_port\", 9050);"));
        assert!(js.contains("user_pref(\"network.proxy.socks_remote_dns\", true);"));
    }

    #[test]
    fn no_proxy_resets_proxy_type() {
        let js = generate(&Profile::test_default(), None);
        assert!(js.contains("user_pref(\"network.proxy.type\", 0);"));
    }

    #[test]
    fn search_engine_display_name_maps() {
        assert_eq!(search_engine_display_name("brave"), "Brave Search");
        assert_eq!(search_engine_display_name("google"), "Google");
        assert_eq!(search_engine_display_name("anything-else"), "DuckDuckGo");
    }

    #[test]
    fn pref_string_escapes_quotes_and_backslashes() {
        let out = pref_string("some.key", r#"a"b\c"#);
        assert_eq!(out, r#"user_pref("some.key", "a\"b\\c");"#);
    }

    #[test]
    fn user_chrome_escapes_label_and_omits_svg_when_empty() {
        let with_label = camoufox_user_chrome("#000", "#fff", "a&b");
        assert!(with_label.contains("a&amp;b"));
        assert!(with_label.contains("url("));

        let empty = camoufox_user_chrome("#000", "#fff", "");
        assert!(!empty.contains("url("));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! «Поделиться» outside Veydan (ROO-173): a text the page made (an
//! invitation link and words around it) goes into another app the user
//! picks there, with the addressee they pick there. Nothing comes back.
//!
//! On a phone the gallery plugin (`tauri-plugin-veydan-media`) hands it on:
//! `send` the system's list of apps, `sms` the SMS app, `mail` a letter. A
//! computer has the letter alone: a `mailto:` address for the mail client,
//! through the opener (no shell).

use veydan_core::{AppError, CmdResult};

#[cfg(target_os = "android")]
use tauri_plugin_veydan_media::VeydanMediaExt;

/// A text longer than this is not the page's: a link and a sentence.
const MAX_TEXT: usize = 2000;
const MAX_SUBJECT: usize = 200;

/// The page's text into another app; `via` is `send`, `sms` or `mail`
/// (`subject` for a letter). Refused with `share_no_app` when the phone has
/// no app for it, `share_unavailable` for a way this platform has not.
#[tauri::command]
pub async fn messenger_share_out(
    via: String,
    text: String,
    subject: Option<String>,
    app: tauri::AppHandle,
) -> CmdResult<()> {
    let text = text.trim();
    let subject = subject.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if text.is_empty() || text.len() > MAX_TEXT || subject.is_some_and(|s| s.len() > MAX_SUBJECT) {
        return Err(AppError::Other("share_bad_text".into()));
    }
    if !matches!(via.as_str(), "send" | "sms" | "mail") {
        return Err(AppError::Other("share_unavailable".into()));
    }
    #[cfg(target_os = "android")]
    {
        app.veydan_media().share_text(&via, text, subject).await.map_err(|e| {
            let e = e.to_string();
            AppError::Other(if e.contains("no_app") { "share_no_app".into() } else { e })
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        if via != "mail" {
            return Err(AppError::Other("share_unavailable".into()));
        }
        // The opener starts xdg-open apart and never hears that no app took the
        // address: the sheet would close on nothing. Asked first.
        #[cfg(target_os = "linux")]
        if !has_mail_client() {
            return Err(AppError::Other("share_no_app".into()));
        }
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(mailto(subject.unwrap_or(""), text), None::<&str>)
            .map_err(|_| AppError::Other("share_no_app".into()))
    }
}

/// Whether the desktop names an app for `mailto:` (xdg-utils, as xdg-open
/// itself); no answer is no app.
#[cfg(target_os = "linux")]
fn has_mail_client() -> bool {
    std::process::Command::new("xdg-mime")
        .args(["query", "default", "x-scheme-handler/mailto"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
}

/// A letter with no addressee (RFC 6068): its subject and body, every byte
/// but the unreserved ones escaped, so the text can add no header.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn mailto(subject: &str, body: &str) -> String {
    format!("mailto:?subject={}&body={}", escape(subject), escape(body))
}

#[cfg_attr(target_os = "android", allow(dead_code))]
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_carries_its_text_escaped_and_no_addressee() {
        let m = mailto("Veydan", "Это я: veydan://c/abc?x=1&cc=a@b\r\nBcc: x");
        assert!(m.starts_with("mailto:?subject=Veydan&body="));
        let body = m.split_once("&body=").unwrap().1;
        for c in ['&', '?', '=', ':', '/', '@', '\r', '\n', ' '] {
            assert!(!body.contains(c), "{c:?} left in {body}");
        }
        assert!(body.contains("%D0%AD"));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a window of the app may load: the pages of the app alone. A link or
//! a redirect in what a page shows (a note, a message) would otherwise take
//! the window to another site, which then runs in the app's own frame, where
//! a PIN screen of its making looks like ours. A web link opens in the
//! user's browser instead, and no page opens a window of its own.

#[cfg(desktop)]
use tauri::webview::NewWindowResponse;
use tauri::{Manager, Runtime, Url, WebviewWindowBuilder};

/// Whether `url` is a page of the app: its own protocol (`tauri://localhost`
/// on Linux and macOS, `http(s)://tauri.localhost` on Windows and Android),
/// the dev server in a dev build, or a `blob:` of one of them.
pub(crate) fn is_own(url: &Url, dev_url: Option<&Url>) -> bool {
    match (url.scheme(), url.host_str()) {
        ("blob", _) => Url::parse(url.path())
            .is_ok_and(|inner| inner.scheme() != "blob" && is_own(&inner, dev_url)),
        ("tauri", Some("localhost")) => true,
        ("http" | "https", Some("tauri.localhost")) => true,
        ("about", _) => url.as_str() == "about:blank",
        _ => dev_url.is_some_and(|dev| {
            url.scheme() == dev.scheme()
                && url.host_str() == dev.host_str()
                && url.port_or_known_default() == dev.port_or_known_default()
        }),
    }
}

/// The dev server of a dev build; `None` in a release, whose pages never
/// come from it.
pub(crate) fn dev_url<R: Runtime, M: Manager<R>>(manager: &M) -> Option<Url> {
    if tauri::is_dev() {
        manager.config().build.dev_url.clone()
    } else {
        None
    }
}

/// A web or mail link: to the user's browser or mail client.
#[cfg(desktop)]
fn open_outside<R: Runtime>(app: &tauri::AppHandle<R>, url: &Url) {
    use tauri_plugin_opener::OpenerExt;
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
            eprintln!("[navigation] cannot open {url}: {e}");
        }
    }
}

/// Keep a window on the pages of the app (platform-spec 5.2): a navigation
/// anywhere else is refused, and on a computer a web or mail link goes to
/// the browser or the mail client, as `open_url` does; a page asking for a
/// new window gets none.
pub fn guard_window<'a, R: Runtime, M: Manager<R>>(
    manager: &M,
    builder: WebviewWindowBuilder<'a, R, M>,
) -> WebviewWindowBuilder<'a, R, M> {
    let dev = dev_url(manager);
    #[cfg(desktop)]
    let app = manager.app_handle().clone();
    let builder = builder.on_navigation(move |url| {
        if is_own(url, dev.as_ref()) {
            return true;
        }
        #[cfg(desktop)]
        open_outside(&app, url);
        false
    });
    #[cfg(desktop)]
    let builder = {
        let app = manager.app_handle().clone();
        builder.on_new_window(move |url, _features| {
            open_outside(&app, &url);
            NewWindowResponse::Deny
        })
    };
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(url: &str, dev: Option<&str>) -> bool {
        let dev = dev.map(|d| Url::parse(d).unwrap());
        is_own(&Url::parse(url).unwrap(), dev.as_ref())
    }

    #[test]
    fn the_pages_of_the_app_are_its_own() {
        assert!(own("tauri://localhost/notes?open=1", None));
        assert!(own("http://tauri.localhost/", None));
        assert!(own("https://tauri.localhost/settings", None));
        assert!(own("blob:tauri://localhost/5f0c", None));
        assert!(own("blob:http://tauri.localhost/5f0c", None));
        assert!(own("about:blank", None));
    }

    #[test]
    fn any_other_page_is_not() {
        for url in [
            "https://example.com/",
            "http://tauri.localhost.example.com/",
            "tauri://example.com/",
            "https://localhost:1440/",
            "http://localhost:1440/",
            "data:text/html,<p>hi</p>",
            "blob:https://example.com/5f0c",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "about:srcdoc",
        ] {
            assert!(!own(url, None), "{url}");
        }
    }

    /// The dev server, in a dev build alone: the same scheme, host and port.
    #[test]
    fn the_dev_server_is_own_only_when_named() {
        let dev = Some("http://localhost:1440");
        assert!(own("http://localhost:1440/notes", dev));
        assert!(own("blob:http://localhost:1440/5f0c", dev));
        assert!(!own("http://localhost:1441/", dev));
        assert!(!own("https://localhost:1440/", dev));
        assert!(!own("http://127.0.0.1:1440/", dev));
    }
}

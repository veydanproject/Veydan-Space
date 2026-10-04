// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! HTTPS client factory for the messenger.
//!
//! The TLS setup is explicit instead of inherited from the process:
//!
//! - **Roots** are the bundled Mozilla set. The platform verifier needs a
//!   JNI context on Android that a library cannot count on, and a bundled
//!   set behaves the same on every system.
//! - **Crypto provider** is `ring`, named explicitly, so a host that
//!   installed another default (or none) changes nothing here.
//!
//! The way a request takes is decided per host, at the moment it connects:
//! a server of the project goes through a bridge while bridges are in use
//! (`messenger-vlink`), everything else goes directly. So a client built
//! here follows a change of that rule without being rebuilt:
//!
//! - no connection is kept for a later request: each one takes the way the
//!   rule says at that moment (requests here are few and large, a
//!   connection more costs nothing worth keeping);
//! - a redirect may not lead a request that went through a bridge to a
//!   host the rule sends directly;
//! - a request under way when the way of its host changes is given up and
//!   made again, see [`following_route`].
//!
//! The system's own proxy settings are not looked at.

use messenger_core::{MessengerError, Result};
use messenger_vlink::{Net, Route};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// Redirects one request may follow.
const MAX_REDIRECTS: usize = 10;
/// How many times a request is made again because its way changed.
const MAX_RESTARTS: usize = 3;

/// TLS as the messenger speaks it to a server: bundled roots, `ring`.
/// No ALPN is set; whoever uses the config for a protocol names it.
pub fn tls_config() -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| MessengerError::Crypto(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(config)
}

/// A builder with TLS, timeouts and the way through a bridge set; callers
/// add what is specific to them (redirect policy, user agent).
pub fn builder(connect: Duration, total: Duration) -> Result<reqwest::ClientBuilder> {
    builder_on(Net::global().clone(), connect, total)
}

/// The same, following the rule `net` instead of the one of the process.
pub fn builder_on(net: Net, connect: Duration, total: Duration) -> Result<reqwest::ClientBuilder> {
    // `socks5h`: the name goes to the door as it is, so no resolver of this
    // machine is asked about a server that is reached through a bridge.
    let rule = net.clone();
    let way = reqwest::Proxy::custom(move |url| url.host_str().and_then(|host| rule.socks_url(host)));
    // A server of the project that redirects somewhere else would take the
    // request off the bridge.
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let bridged = |url: &reqwest::Url| url.host_str().is_some_and(|h| net.route(h) == Route::Bridge);
        if attempt.previous().iter().any(bridged) && !bridged(attempt.url()) {
            return attempt.error("a redirect would leave the bridge");
        }
        attempt.follow()
    });
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls_config()?)
        .proxy(way)
        .redirect(redirects)
        .pool_max_idle_per_host(0)
        .connect_timeout(connect)
        .timeout(total))
}

/// The way of the request's host changed while it was under way, more
/// often than it is worth making it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteChanged;

impl std::fmt::Display for RouteChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the way to the server changed")
    }
}

/// Makes a request to `host` with `attempt`, and makes it again when the
/// way to that host changes while it is under way: a file that went out
/// directly and stalled goes out through the bridge as soon as bridges are
/// turned on, instead of waiting out its timeout on a way nobody uses.
/// Changes that leave this host's way as it was interrupt nothing.
pub async fn following_route<T, F, Fut>(host: &str, attempt: F) -> std::result::Result<T, RouteChanged>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
{
    following_route_on(Net::global(), host, attempt).await
}

/// The same, under the rule `net`.
pub async fn following_route_on<T, F, Fut>(net: &Net, host: &str, mut attempt: F) -> std::result::Result<T, RouteChanged>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
{
    for _ in 0..=MAX_RESTARTS {
        // Watched before the request starts, so that no change slips by.
        let mut changes = net.changes();
        changes.mark_unchanged();
        let way = net.route(host);
        let request = attempt();
        tokio::pin!(request);
        loop {
            tokio::select! {
                out = &mut request => return Ok(out),
                seen = changes.changed() => {
                    if seen.is_err() {
                        // The rule is gone: nothing will change any more.
                        return Ok(request.await);
                    }
                    if net.route(host) != way {
                        // Given up; the next round takes the new way.
                        break;
                    }
                }
            }
        }
    }
    Err(RouteChanged)
}

/// A client with the given connect and total timeouts.
pub fn client(connect: Duration, total: Duration) -> Result<reqwest::Client> {
    builder(connect, total)?.build().map_err(|e| MessengerError::Transport(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_without_a_process_default_provider() {
        let c = client(Duration::from_secs(1), Duration::from_secs(2));
        assert!(c.is_ok(), "{:?}", c.err());
    }
}

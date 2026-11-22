// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A relay of a test, listening.

use nostr_sdk::local_relay::{LocalRelay, LocalRelayBuilder};

/// A local relay, already listening.
///
/// `LocalRelay` picks a random port by trying it free, lets it go and binds
/// it later, so another test or job on the same host can take it in between
/// (`AddrInUse` on a loaded CI runner): a relay that failed to bind is
/// replaced by a new one, which picks a new port, five times at the most.
pub async fn local_relay() -> LocalRelay {
    local_relay_with(LocalRelay::builder).await
}

/// The same for a relay with policies: `make` builds its builder anew for
/// every try, as a builder is spent by the relay it makes.
pub async fn local_relay_with(make: impl Fn() -> LocalRelayBuilder) -> LocalRelay {
    let mut last = None;
    for _ in 0..5 {
        let relay = make().build();
        match relay.run().await {
            Ok(()) => return relay,
            Err(e) => last = Some(e),
        }
    }
    panic!("no local relay after five ports: {last:?}");
}

/// The environment variable that points the suites at a relay that is not
/// started by the test: its `ws://` or `wss://` URL.
pub const EXTERNAL_URL_VAR: &str = "VEYDAN_TEST_RELAY_URL";
/// The app key of that relay, when it asks for one (`?key=`).
pub const EXTERNAL_KEY_VAR: &str = "VEYDAN_TEST_RELAY_KEY";

/// The URL of the external relay the environment names, with its app key as
/// `?key=`; `None` when `VEYDAN_TEST_RELAY_URL` is not set or is empty.
pub fn external_relay_url() -> Option<String> {
    external_url(std::env::var(EXTERNAL_URL_VAR).ok(), std::env::var(EXTERNAL_KEY_VAR).ok())
}

fn external_url(url: Option<String>, key: Option<String>) -> Option<String> {
    let url = url?.trim().to_string();
    if url.is_empty() {
        return None;
    }
    let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    Some(match key {
        None => url,
        Some(key) if url.contains('?') => format!("{url}&key={key}"),
        Some(key) => format!("{}/?key={key}", url.trim_end_matches('/')),
    })
}

/// A relay for a test: the local one, or the external one the environment
/// names. A test that only needs the URL of a relay takes this instead of
/// [`local_relay`] and runs against another relay by setting
/// `VEYDAN_TEST_RELAY_URL`. A test that needs the `LocalRelay` itself (its
/// policies, its database, its events) keeps [`local_relay`] /
/// [`local_relay_with`].
pub enum TestRelay {
    Local(Box<LocalRelay>),
    External(String),
}

impl TestRelay {
    /// The URL to connect to, without a trailing slash.
    pub async fn url(&self) -> String {
        match self {
            TestRelay::Local(relay) => relay.url().await.as_str_without_trailing_slash().to_string(),
            TestRelay::External(url) => url.trim_end_matches('/').to_string(),
        }
    }

    /// Whether the relay is the external one.
    pub fn is_external(&self) -> bool {
        matches!(self, TestRelay::External(_))
    }

    /// The local relay, when it is one.
    pub fn local(&self) -> Option<&LocalRelay> {
        match self {
            TestRelay::Local(relay) => Some(relay.as_ref()),
            TestRelay::External(_) => None,
        }
    }
}

/// The external relay when `VEYDAN_TEST_RELAY_URL` is set, otherwise a local
/// one, already listening ([`local_relay`]).
pub async fn test_relay() -> TestRelay {
    match external_relay_url() {
        Some(url) => TestRelay::External(url),
        None => TestRelay::Local(Box::new(local_relay().await)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.into())
    }

    #[test]
    fn the_external_url_takes_its_key() {
        assert_eq!(external_url(None, s("k")), None);
        assert_eq!(external_url(s("  "), None), None);
        assert_eq!(external_url(s("wss://r.example"), None), s("wss://r.example"));
        assert_eq!(external_url(s("wss://r.example/"), s("")), s("wss://r.example/"));
        assert_eq!(external_url(s("wss://r.example/"), s("k")), s("wss://r.example/?key=k"));
        assert_eq!(external_url(s("ws://r.example:7777"), s("k")), s("ws://r.example:7777/?key=k"));
        assert_eq!(external_url(s("wss://r.example/?x=1"), s("k")), s("wss://r.example/?x=1&key=k"));
    }

    #[tokio::test]
    async fn an_external_relay_is_handed_out_as_it_is() {
        let relay = TestRelay::External("wss://r.example/?key=k".into());
        assert_eq!(relay.url().await, "wss://r.example/?key=k");
        assert!(relay.is_external() && relay.local().is_none());
    }
}

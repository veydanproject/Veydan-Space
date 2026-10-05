// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Tor as a proxy type: a row of `proxies` with `proxy_type = "tor"`.
//!
//! The row names no server. Its `country` column holds the exit countries
//! (two-letter codes separated by commas; empty is any country), and `host`
//! and `port`, which the table does not let be empty, hold a placeholder no
//! code connects to. The traffic goes to the SOCKS port of the tor instance
//! the module Tor runs for that set of countries (`veydan_tor::TorManager`).

use crate::models::{CreateProxyRequest, Proxy};
use veydan_tor::{ExitSet, Lease, TorManager};

pub const TYPE: &str = "tor";

/// What `host` and `port` of a Tor row hold: the columns are NOT NULL, and
/// a client of 4.0.x inserts the row with them.
pub const PLACEHOLDER_HOST: &str = "127.0.0.1";
pub const PLACEHOLDER_PORT: i64 = 9050;

pub fn is_tor(proxy: &Proxy) -> bool {
    proxy.proxy_type == TYPE
}

/// The exit countries of a Tor row.
pub fn exit_set(proxy: &Proxy) -> Result<ExitSet, String> {
    ExitSet::parse(proxy.country.as_deref().unwrap_or("")).map_err(|e| e.to_string())
}

/// A hold on the tor instance of the row's countries, started and connected
/// if it was not. `consumer` names who asks: tor gives each consumer
/// circuits of its own. Every error here means no connection at all, never
/// a direct one.
pub async fn lease(tor: &TorManager, proxy: &Proxy, consumer: &str) -> Result<Lease, String> {
    let exit = exit_set(proxy)?;
    tor.acquire(&exit, consumer).await.map_err(|e| e.to_string())
}

/// A Tor row as it is stored, whatever the form sent: the placeholder
/// address, no credentials, no city, and the countries tidied (`"DE, nl"`
/// becomes `"de,nl"`). Rows of other types pass untouched.
pub fn normalize(req: &mut CreateProxyRequest) -> Result<(), String> {
    if req.proxy_type != TYPE {
        return Ok(());
    }
    let exit = ExitSet::parse(req.country.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
    req.host = PLACEHOLDER_HOST.to_string();
    req.port = PLACEHOLDER_PORT;
    req.username = None;
    // An empty string, not None: an update keeps a stored secret on None.
    req.password = Some(String::new());
    req.private_key = Some(String::new());
    req.city = None;
    req.country = (!exit.is_any()).then(|| exit.codes().join(","));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(proxy_type: &str, country: Option<&str>) -> CreateProxyRequest {
        CreateProxyRequest {
            name: "p".into(),
            proxy_type: proxy_type.into(),
            host: "example.com".into(),
            port: 1080,
            tags: None,
            username: Some("u".into()),
            password: Some("secret".into()),
            country: country.map(str::to_string),
            city: Some("Berlin".into()),
            private_key: None,
        }
    }

    #[test]
    fn a_tor_row_keeps_only_its_countries() {
        let mut req = request("tor", Some("DE, nl"));
        normalize(&mut req).unwrap();
        assert_eq!(req.host, PLACEHOLDER_HOST);
        assert_eq!(req.port, PLACEHOLDER_PORT);
        assert_eq!(req.username, None);
        assert_eq!(req.password.as_deref(), Some(""));
        assert_eq!(req.private_key.as_deref(), Some(""));
        assert_eq!(req.city, None);
        assert_eq!(req.country.as_deref(), Some("de,nl"));
    }

    #[test]
    fn no_country_is_any_exit() {
        for country in [None, Some(""), Some("  ")] {
            let mut req = request("tor", country);
            normalize(&mut req).unwrap();
            assert_eq!(req.country, None);
        }
    }

    #[test]
    fn a_country_that_is_not_a_code_is_refused() {
        let mut req = request("tor", Some("Germany"));
        assert!(normalize(&mut req).is_err());
    }

    #[test]
    fn rows_of_other_types_pass_untouched() {
        let mut req = request("socks5", Some("Germany"));
        normalize(&mut req).unwrap();
        assert_eq!(req.host, "example.com");
        assert_eq!(req.password.as_deref(), Some("secret"));
        assert_eq!(req.country.as_deref(), Some("Germany"));
    }

    /// The launch of a profile and the SSH transport both take the lease
    /// first: without tor there is an error, and nothing to connect through.
    #[tokio::test]
    async fn without_tor_installed_there_is_no_lease() {
        // A directory nothing was installed into; the manager creates none.
        let dir = std::env::temp_dir().join(format!("veydan-no-tor-{}", uuid::Uuid::new_v4()));
        let tor = TorManager::new(
            dir.clone(),
            veydan_tor::TorSettings::defaults(),
            |_| {},
        );
        let mut proxy = Proxy::test_default();
        proxy.proxy_type = TYPE.into();
        let err = lease(&tor, &proxy, "profile-1").await.unwrap_err();
        assert!(err.starts_with("tor_not_installed"), "{err}");

        proxy.country = Some("Germany".into());
        assert!(lease(&tor, &proxy, "profile-1").await.is_err());
    }

    /// The way a profile takes, with the real tor: the browser's requests go
    /// to the local relay, the relay to the instance. Run by hand:
    /// `VEYDAN_TOR_BUNDLE=/path/to/bundle cargo test -p veydanspace --locked
    /// -- --ignored through_the_real_tor --nocapture`
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs VEYDAN_TOR_BUNDLE and the network"]
    async fn through_the_real_tor_the_relay_reaches_the_web_and_an_onion() {
        let bundle = std::env::var_os("VEYDAN_TOR_BUNDLE").expect("VEYDAN_TOR_BUNDLE");
        let data = std::env::temp_dir().join(format!("veydan-tor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(data.join("tor")).unwrap();
        std::os::unix::fs::symlink(bundle, data.join("tor").join("bundle")).unwrap();
        let tor = TorManager::new(data.clone(), veydan_tor::TorSettings::defaults(), |_| {});

        let mut proxy = Proxy::test_default();
        proxy.proxy_type = TYPE.into();
        proxy.country = None;

        // The check of the proxy list.
        let checked = crate::proxy::check::check_proxy(&proxy, &tor).await.unwrap();
        assert!(checked.ok, "the check found no address");
        println!("check: {}", checked.ip);

        // The relay of a profile.
        let lease = lease(&tor, &proxy, "profile-1").await.unwrap();
        let (port, stop) = crate::proxy::local::spawn(crate::proxy::local::Upstream::Tor { lease })
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap())
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap();
        let answer: serde_json::Value = client
            .get("https://check.torproject.org/api/ip")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        println!("https through the relay: {answer}");
        assert_eq!(answer["IsTor"], true);

        // Plain HTTP takes the other branch of the relay.
        let plain = client
            .get("http://check.torproject.org/api/ip")
            .send()
            .await
            .unwrap();
        println!("http through the relay: {}", plain.status());
        assert!(plain.status().is_success() || plain.status().is_redirection());

        // An onion name is resolved by tor.
        let onion = client
            .get("https://duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion/")
            .send()
            .await
            .unwrap();
        println!("onion through the relay: {}", onion.status());
        assert!(onion.status().is_success());

        // A row with a country runs an instance of its own.
        proxy.country = Some("de".into());
        let german = crate::proxy::check::check_proxy(&proxy, &tor).await.unwrap();
        println!("check de: {}", german.ip);
        assert!(german.ok);
        let keys: Vec<String> = tor.instances().into_iter().map(|i| i.key).collect();
        assert_eq!(keys, ["any", "de"]);

        stop.send(()).ok();
        tor.stop_all().await;
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn the_exit_set_of_a_row_is_read_from_its_country() {
        let mut proxy = Proxy::test_default();
        proxy.proxy_type = TYPE.into();
        proxy.country = Some("nl,de".into());
        assert_eq!(exit_set(&proxy).unwrap().key(), "de-nl");
        proxy.country = None;
        assert!(exit_set(&proxy).unwrap().is_any());
        proxy.country = Some("Germany".into());
        assert!(exit_set(&proxy).is_err());
    }
}

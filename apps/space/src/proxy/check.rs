// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::models::{Proxy, ProxyCheckResult};
use anyhow::Result;
use serde::Deserialize;

/// HTTPS geo endpoint; the response can't be tampered with by the proxy operator.
const GEO_URL: &str = "https://ipwho.is/";

#[derive(Deserialize, Default)]
struct GeoResponse {
    ip: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
}

pub async fn check_proxy(proxy: &Proxy) -> Result<ProxyCheckResult> {
    if proxy.proxy_type == "ssh" {
        return check_ssh(proxy).await;
    }

    let geo = fetch_geo(&build_proxy_url(proxy)).await?;
    Ok(ProxyCheckResult {
        ok: !geo.ip.is_empty(),
        ip: geo.ip,
        country: geo.country,
        city: geo.city,
        ssh_fingerprint: None,
        ssh_fingerprint_is_new: None,
    })
}

/// SSH: connect, authenticate, then run the same HTTPS check through the
/// local bridge so the TLS handshake happens over the direct-tcpip channel.
async fn check_ssh(proxy: &Proxy) -> Result<ProxyCheckResult> {
    let auth = match proxy.private_key.as_deref().filter(|k| !k.is_empty()) {
        Some(key) => crate::proxy::ssh::SshAuth::PrivateKey(key.to_string()),
        None => crate::proxy::ssh::SshAuth::Password(proxy.password.clone().unwrap_or_default()),
    };

    let username = proxy.username.clone().unwrap_or_default();
    let result = crate::proxy::ssh::SshSession::connect(
        &proxy.host,
        proxy.port as u16,
        &username,
        auth,
        proxy.server_fingerprint.clone(),
    )
    .await?;

    let upstream = crate::proxy::local::Upstream::Ssh {
        session: result.session,
    };
    let (local_port, stop_tx) = crate::proxy::local::spawn(upstream).await?;
    let geo = fetch_geo(&format!("http://127.0.0.1:{local_port}")).await;
    let _ = stop_tx.send(());
    let geo = geo.unwrap_or_default();

    Ok(ProxyCheckResult {
        ok: !geo.ip.is_empty(),
        ip: geo.ip,
        country: geo.country,
        city: geo.city,
        ssh_fingerprint: Some(result.fingerprint),
        ssh_fingerprint_is_new: Some(result.is_new),
    })
}

#[derive(Default)]
struct Geo {
    ip: String,
    country: Option<String>,
    city: Option<String>,
}

async fn fetch_geo(proxy_url: &str) -> Result<Geo> {
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url)?)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp = client
        .get(GEO_URL)
        .send()
        .await?
        .json::<GeoResponse>()
        .await?;
    Ok(Geo {
        ip: resp.ip.unwrap_or_default(),
        country: resp.country_code,
        city: resp.city,
    })
}

pub fn build_proxy_url(proxy: &Proxy) -> String {
    let scheme = match proxy.proxy_type.as_str() {
        // socks5h:// = DNS resolves via proxy (consistent with Firefox socks_remote_dns=true)
        "socks5" => "socks5h",
        "https" => "https",
        _ => "http",
    };

    match (&proxy.username, &proxy.password) {
        (Some(user), Some(pass)) if !user.is_empty() => {
            use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
            let user = utf8_percent_encode(user, NON_ALPHANUMERIC);
            let pass = utf8_percent_encode(pass, NON_ALPHANUMERIC);
            format!("{scheme}://{}:{}@{}:{}", user, pass, proxy.host, proxy.port)
        }
        _ => format!("{scheme}://{}:{}", proxy.host, proxy.port),
    }
}

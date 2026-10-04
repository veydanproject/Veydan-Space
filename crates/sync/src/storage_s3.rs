// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! S3-compatible adapter (AWS S3, Cloudflare R2, MinIO, Backblaze B2) with
//! hand-rolled SigV4 signing. Only four operations are needed.

use crate::storage::{http_client, is_noise_key, send_retry, status_error, Storage};
use crate::{sha256_hex, Result, SyncError};
use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::{Client, Method, StatusCode};
use sha2::Sha256;
use url::Url;

/// RFC 3986 unreserved set as S3 expects: everything except A-Z a-z 0-9 - _ . ~
const S3_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Clone, Debug)]
pub struct S3Config {
    /// e.g. `https://s3.amazonaws.com`, `https://<acct>.r2.cloudflarestorage.com`, `http://localhost:9000`
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Optional key prefix inside the bucket, without leading slash.
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    /// `true`: `endpoint/bucket/key`; `false`: `bucket.endpoint/key`.
    pub path_style: bool,
}

pub struct S3Storage {
    cfg: S3Config,
    client: Client,
    base: Url,
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, S3_ENCODE).to_string()
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length is valid");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

impl S3Storage {
    pub fn new(cfg: S3Config) -> Result<Self> {
        let mut base = Url::parse(cfg.endpoint.trim_end_matches('/'))
            .map_err(|e| SyncError::Storage(e.to_string()))?;
        if !cfg.path_style {
            let host = base
                .host_str()
                .ok_or_else(|| SyncError::Storage("endpoint has no host".into()))?;
            let vhost = format!("{}.{}", cfg.bucket, host);
            base.set_host(Some(&vhost))
                .map_err(|e| SyncError::Storage(e.to_string()))?;
        }
        Ok(Self {
            cfg,
            client: http_client()?,
            base,
        })
    }

    /// Prefix + key. An empty key is the prefix alone, so list can strip `prefix/` once.
    fn full_key(&self, key: &str) -> String {
        let p = self.cfg.prefix.trim_matches('/');
        let k = key.trim_start_matches('/');
        if p.is_empty() {
            k.to_string()
        } else if k.is_empty() {
            p.to_string()
        } else {
            format!("{p}/{k}")
        }
    }

    /// Canonical URI path for an object key ("" = bucket root).
    fn object_path(&self, key: &str) -> String {
        let encoded: Vec<String> = key.split('/').filter(|s| !s.is_empty()).map(enc).collect();
        let tail = encoded.join("/");
        if self.cfg.path_style {
            if tail.is_empty() {
                format!("/{}", self.cfg.bucket)
            } else {
                format!("/{}/{}", self.cfg.bucket, tail)
            }
        } else if tail.is_empty() {
            "/".to_string()
        } else {
            format!("/{tail}")
        }
    }

    fn host_header(&self) -> String {
        match (self.base.host_str(), self.base.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => String::new(),
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<reqwest::Response> {
        self.request_with(method, path, query, body, &[]).await
    }

    /// Signed request with transport retries; 429/503 get one more attempt after a pause.
    async fn request_with(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Vec<u8>,
        extra: &[(&str, &str)],
    ) -> Result<reqwest::Response> {
        let build = || self.signed(method.clone(), path, query, body.clone(), extra);
        let resp = send_retry(build).await?;
        if !matches!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
        ) {
            return Ok(resp);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        send_retry(build).await
    }

    /// `extra` headers are sent unsigned (SigV4 only requires host/date/content-sha256).
    fn signed(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Vec<u8>,
        extra: &[(&str, &str)],
    ) -> reqwest::RequestBuilder {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let payload_hash = sha256_hex(&body);
        let host = self.host_header();

        let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (enc(k), enc(v))).collect();
        q.sort();
        let canonical_query = q
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "{}\n{path}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
            method.as_str()
        );

        let scope = format!("{date}/{}/s3/aws4_request", self.cfg.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        let k_date = hmac(
            format!("AWS4{}", self.cfg.secret_key).as_bytes(),
            date.as_bytes(),
        );
        let k_region = hmac(&k_date, self.cfg.region.as_bytes());
        let k_service = hmac(&k_region, b"s3");
        let k_signing = hmac(&k_service, b"aws4_request");
        let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.cfg.access_key
        );

        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(if canonical_query.is_empty() {
            None
        } else {
            Some(&canonical_query)
        });

        let mut req = self
            .client
            .request(method, url)
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("authorization", authorization);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        req.body(body)
    }

    async fn fail(resp: reqwest::Response, what: &str) -> SyncError {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        status_error(status, &format!("s3 {what}"), &text)
    }
}

/// Pull `<Key>` values and the continuation token out of a ListObjectsV2 response.
fn parse_list(xml: &[u8]) -> Result<(Vec<String>, Option<String>)> {
    let mut reader = Reader::from_reader(xml);
    let mut keys = Vec::new();
    let mut token = None;
    let mut truncated = false;
    let mut current = String::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| SyncError::Format(e.to_string()))?
        {
            Event::Start(e) => {
                current = String::from_utf8_lossy(e.local_name().as_ref()).to_string()
            }
            Event::Text(t) => {
                let text = t
                    .xml_content()
                    .map_err(|e| SyncError::Format(e.to_string()))?
                    .to_string();
                match current.as_str() {
                    "Key" => keys.push(text),
                    "NextContinuationToken" => token = Some(text),
                    "IsTruncated" => truncated = text == "true",
                    _ => {}
                }
            }
            Event::End(_) => current.clear(),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((keys, if truncated { token } else { None }))
}

#[async_trait]
impl Storage for S3Storage {
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let full_prefix = self.full_key(prefix);
        let strip = self.full_key("");
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![
                ("list-type".to_string(), "2".to_string()),
                ("prefix".to_string(), full_prefix.clone()),
            ];
            if let Some(t) = &token {
                query.push(("continuation-token".to_string(), t.clone()));
            }
            let resp = self
                .request(Method::GET, &self.object_path(""), &query, Vec::new())
                .await?;
            if !resp.status().is_success() {
                return Err(Self::fail(resp, "list").await);
            }
            let body = resp.bytes().await?;
            let (keys, next) = parse_list(&body)?;
            for k in keys {
                let rel = if strip.is_empty() {
                    k
                } else {
                    k.strip_prefix(&format!("{strip}/"))
                        .unwrap_or(&k)
                        .to_string()
                };
                if !is_noise_key(&rel) {
                    out.push(rel);
                }
            }
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(out)
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let resp = self
            .request(
                Method::GET,
                &self.object_path(&self.full_key(key)),
                &[],
                Vec::new(),
            )
            .await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => Ok(Some(resp.bytes().await?.to_vec())),
            _ => Err(Self::fail(resp, "get").await),
        }
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let resp = self
            .request(
                Method::PUT,
                &self.object_path(&self.full_key(key)),
                &[],
                data.to_vec(),
            )
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Self::fail(resp, "put").await)
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let resp = self
            .request(
                Method::DELETE,
                &self.object_path(&self.full_key(key)),
                &[],
                Vec::new(),
            )
            .await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(()),
            s if s.is_success() => Ok(()),
            _ => Err(Self::fail(resp, "delete").await),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let resp = self
            .request(
                Method::HEAD,
                &self.object_path(&self.full_key(key)),
                &[],
                Vec::new(),
            )
            .await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(false),
            s if s.is_success() => Ok(true),
            _ => Err(Self::fail(resp, "head").await),
        }
    }

    /// Conditional PUT; servers without `If-None-Match` fall back to HEAD + PUT.
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> Result<bool> {
        let path = self.object_path(&self.full_key(key));
        let resp = self
            .request_with(
                Method::PUT,
                &path,
                &[],
                data.to_vec(),
                &[("If-None-Match", "*")],
            )
            .await?;
        match resp.status() {
            s if s.is_success() => Ok(true),
            StatusCode::PRECONDITION_FAILED => Ok(false),
            StatusCode::NOT_IMPLEMENTED | StatusCode::BAD_REQUEST => {
                if self.exists(key).await? {
                    return Ok(false);
                }
                self.put(key, data).await?;
                Ok(true)
            }
            _ => Err(Self::fail(resp, "put").await),
        }
    }
}

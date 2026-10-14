// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! S3-compatible adapter (AWS S3, Cloudflare R2, MinIO, Backblaze B2) with
//! hand-rolled SigV4 signing. Only four operations are needed.

use crate::storage::{
    http_client, is_noise_key, send_retry, status_error, Storage, ERR_NO_CONDITIONAL_PUT,
    METADATA_TIMEOUT,
};
use crate::{random_hex, sha256_hex, Result, SyncError};
use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::{Client, Method, StatusCode};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
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

        let is_short = method == Method::HEAD || query.iter().any(|(k, _)| k == "list-type");
        let mut req = self
            .client
            .request(method, url)
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("authorization", authorization);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        if is_short {
            req = req.timeout(METADATA_TIMEOUT);
        }
        req.body(body)
    }

    async fn fail(resp: reqwest::Response, what: &str) -> SyncError {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        status_error(status, &format!("s3 {what}"), &text)
    }

    /// One PUT with `If-None-Match: *`.
    async fn put_if_none_match(&self, key: &str, data: &[u8]) -> Result<Conditional> {
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
            s if s.is_success() => Ok(Conditional::Written),
            StatusCode::PRECONDITION_FAILED => Ok(Conditional::Present),
            StatusCode::NOT_IMPLEMENTED | StatusCode::BAD_REQUEST => Ok(Conditional::Unsupported),
            _ => Err(Self::fail(resp, "put").await),
        }
    }

    /// Whether the server keeps `If-None-Match: *`: a fresh probe key written
    /// twice must answer 412 the second time. Asked once per storage in the
    /// process (the cycle builds its storage anew); an error of the network
    /// is not remembered. The probe key starts with a dot, so it is noise to
    /// every `list` even when its delete fails.
    async fn conditional_put_works(&self) -> Result<bool> {
        static KNOWN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
        let known = KNOWN.get_or_init(|| Mutex::new(HashMap::new()));
        let id = format!("{}|{}|{}", self.base, self.cfg.bucket, self.full_key(""));
        if let Some(works) = known.lock().ok().and_then(|k| k.get(&id).copied()) {
            return Ok(works);
        }
        let probe = format!(".veydan-probe-{}", random_hex(8));
        let works = async {
            Ok::<_, SyncError>(match self.put_if_none_match(&probe, b"probe").await? {
                Conditional::Written => {
                    self.put_if_none_match(&probe, b"again").await? == Conditional::Present
                }
                Conditional::Present | Conditional::Unsupported => false,
            })
        }
        .await;
        let _ = self.delete(&probe).await;
        let works = works?;
        if let Ok(mut k) = known.lock() {
            k.insert(id, works);
        }
        Ok(works)
    }
}

/// What a PUT with `If-None-Match: *` came to.
#[derive(Debug, PartialEq, Eq)]
enum Conditional {
    Written,
    /// 412: the key is there.
    Present,
    /// 501 or 400: the server does not take the header.
    Unsupported,
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
        let strip = self.full_key("");
        // `vault/`, not `vault`: the bucket's `vault2/…` is not this vault's.
        let full_prefix = if prefix.is_empty() && !strip.is_empty() {
            format!("{strip}/")
        } else {
            self.full_key(prefix)
        };
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
                    match k.strip_prefix(&format!("{strip}/")) {
                        Some(rel) => rel.to_string(),
                        None => continue,
                    }
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

    /// Conditional PUT. A server that ignores `If-None-Match` would answer 2xx
    /// and overwrite, so it is asked once (`conditional_put_works`) and
    /// refused (`ERR_NO_CONDITIONAL_PUT`) instead of a HEAD and a PUT apart.
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> Result<bool> {
        if !self.conditional_put_works().await? {
            return Err(SyncError::Storage(ERR_NO_CONDITIONAL_PUT.into()));
        }
        match self.put_if_none_match(key, data).await? {
            Conditional::Written => Ok(true),
            Conditional::Present => Ok(false),
            Conditional::Unsupported => Err(SyncError::Storage(ERR_NO_CONDITIONAL_PUT.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    //! `S3Storage` against an in-process fake of a bucket (path style, no
    //! check of the signature) that may ignore `If-None-Match`.

    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, Method as Verb, StatusCode as Code, Uri};
    use axum::response::{IntoResponse, Response};
    use axum::Router;
    use percent_encoding::percent_decode_str;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[derive(Default)]
    struct Bucket {
        objects: BTreeMap<String, Vec<u8>>,
        /// The server writes over a key whatever `If-None-Match` says.
        ignores_condition: bool,
        /// Conditional PUTs of probe keys that reached the server.
        probes: usize,
    }

    type Shared = Arc<Mutex<Bucket>>;

    fn decode(s: &str) -> String {
        percent_decode_str(s).decode_utf8_lossy().into_owned()
    }

    async fn handle(
        State(shared): State<Shared>,
        method: Verb,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let mut b = shared.lock().unwrap();
        let key = decode(uri.path().strip_prefix("/b/").unwrap_or(""));
        match method {
            Verb::GET if key.is_empty() => {
                let prefix = uri
                    .query()
                    .unwrap_or("")
                    .split('&')
                    .find_map(|p| p.strip_prefix("prefix="))
                    .map(decode)
                    .unwrap_or_default();
                let mut xml = String::from("<ListBucketResult>");
                for k in b.objects.keys().filter(|k| k.starts_with(&prefix)) {
                    xml.push_str(&format!("<Contents><Key>{k}</Key></Contents>"));
                }
                xml.push_str("<IsTruncated>false</IsTruncated></ListBucketResult>");
                (Code::OK, xml).into_response()
            }
            Verb::GET | Verb::HEAD => match b.objects.get(&key) {
                Some(data) if method == Verb::GET => (Code::OK, data.clone()).into_response(),
                Some(_) => Code::OK.into_response(),
                None => Code::NOT_FOUND.into_response(),
            },
            Verb::PUT => {
                let conditional = headers.get("if-none-match").is_some();
                if conditional
                    && key
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .starts_with(".veydan-probe-")
                {
                    b.probes += 1;
                }
                if conditional && !b.ignores_condition && b.objects.contains_key(&key) {
                    return Code::PRECONDITION_FAILED.into_response();
                }
                b.objects.insert(key, body.to_vec());
                Code::OK.into_response()
            }
            Verb::DELETE => {
                b.objects.remove(&key);
                Code::NO_CONTENT.into_response()
            }
            _ => Code::BAD_REQUEST.into_response(),
        }
    }

    async fn serve(bucket: Bucket, prefix: &str) -> (S3Storage, Shared) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let shared = Arc::new(Mutex::new(bucket));
        let app = Router::new().fallback(handle).with_state(shared.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let storage = S3Storage::new(S3Config {
            endpoint: format!("http://{addr}"),
            region: "us-east-1".into(),
            bucket: "b".into(),
            prefix: prefix.into(),
            access_key: "k".into(),
            secret_key: "s".into(),
            path_style: true,
        })
        .expect("storage");
        (storage, shared)
    }

    #[tokio::test]
    async fn a_server_that_ignores_if_none_match_is_refused_before_any_write() {
        let (s, shared) = serve(
            Bucket {
                ignores_condition: true,
                ..Bucket::default()
            },
            "vault",
        )
        .await;
        for _ in 0..2 {
            let err = s
                .put_if_absent("devices/a/log/00000001", b"chunk")
                .await
                .unwrap_err();
            assert!(
                matches!(&err, SyncError::Storage(m) if m == ERR_NO_CONDITIONAL_PUT),
                "{err}"
            );
        }
        let b = shared.lock().unwrap();
        assert!(
            b.objects.is_empty(),
            "no chunk, no probe left: {:?}",
            b.objects.keys()
        );
        assert_eq!(b.probes, 2, "asked once: two PUTs of one probe key");
    }

    #[tokio::test]
    async fn a_server_that_keeps_if_none_match_has_one_winner_and_is_asked_once() {
        let (s, shared) = serve(Bucket::default(), "vault").await;
        assert!(s
            .put_if_absent("devices/a/log/00000001", b"first")
            .await
            .unwrap());
        assert!(!s
            .put_if_absent("devices/a/log/00000001", b"second")
            .await
            .unwrap());
        let b = shared.lock().unwrap();
        assert_eq!(b.probes, 2);
        assert_eq!(
            b.objects.len(),
            1,
            "the probe key is gone: {:?}",
            b.objects.keys()
        );
        assert_eq!(b.objects["vault/devices/a/log/00000001"], b"first");
    }

    #[tokio::test]
    async fn a_neighbour_prefix_is_not_listed_as_this_vault() {
        let mut objects = BTreeMap::new();
        objects.insert("vault/manifest.json".to_string(), b"m".to_vec());
        objects.insert("vault2/manifest.json".to_string(), b"other".to_vec());
        objects.insert("vaultx".to_string(), b"other".to_vec());
        let (s, _) = serve(
            Bucket {
                objects,
                ..Bucket::default()
            },
            "vault",
        )
        .await;
        assert_eq!(s.list("").await.unwrap(), vec!["manifest.json"]);

        let (empty, _) = serve(
            Bucket {
                objects: [("vault2/manifest.json".to_string(), b"x".to_vec())].into(),
                ..Bucket::default()
            },
            "vault",
        )
        .await;
        assert!(empty.list("").await.unwrap().is_empty());
    }
}

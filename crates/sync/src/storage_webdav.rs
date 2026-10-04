// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! WebDAV adapter (Nextcloud, Yandex Disk, generic servers) with basic auth.

use crate::storage::{http_client, is_noise_key, send_retry, status_error, Storage, TMP_SUFFIX};
use crate::{random_hex, Result, SyncError};
use async_trait::async_trait;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::{Client, Method, StatusCode};
use std::collections::HashSet;
use std::sync::Mutex;
use url::Url;

/// Characters that must be escaped inside a path segment.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'%');

#[derive(Clone, Debug)]
pub struct WebDavConfig {
    /// Collection URL that becomes the vault root, e.g. `https://webdav.yandex.ru/veydan`.
    pub url: String,
    pub username: String,
    pub password: String,
}

pub struct WebDavStorage {
    cfg: WebDavConfig,
    client: Client,
    base: Url,
    /// Collections known to exist; avoids a MKCOL storm on every put.
    known_dirs: Mutex<HashSet<String>>,
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;

impl WebDavStorage {
    pub fn new(cfg: WebDavConfig) -> Result<Self> {
        let mut base = Url::parse(&cfg.url).map_err(|e| SyncError::Storage(e.to_string()))?;
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        Ok(Self {
            cfg,
            client: http_client()?,
            base,
            known_dirs: Mutex::new(HashSet::new()),
        })
    }

    fn url_for(&self, key: &str) -> Url {
        let mut url = self.base.clone();
        let encoded: Vec<String> = key
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
            .collect();
        let path = format!("{}{}", self.base.path(), encoded.join("/"));
        url.set_path(&path);
        url
    }

    fn req(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        let b = self.client.request(method, url);
        if self.cfg.username.is_empty() {
            b
        } else {
            b.basic_auth(&self.cfg.username, Some(&self.cfg.password))
        }
    }

    async fn fail(resp: reqwest::Response, what: &str) -> SyncError {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        status_error(status, &format!("webdav {what}"), &text)
    }

    async fn put_raw(&self, key: &str, data: &[u8]) -> Result<()> {
        if let Some((dir, _)) = key.rsplit_once('/') {
            self.ensure_dir(dir).await?;
        }
        let url = self.url_for(key);
        let body = data.to_vec();
        let resp = send_retry(|| self.req(Method::PUT, url.clone()).body(body.clone())).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Self::fail(resp, "put").await)
        }
    }

    /// MOVE `from` onto `to`. With `overwrite = false`, 412 means the destination
    /// already exists. `Ok(None)` when the server does not support MOVE at all.
    async fn move_to(&self, from: &str, to: &str, overwrite: bool) -> Result<Option<bool>> {
        let dest = self.url_for(to);
        let resp = send_retry(|| {
            self.req(
                Method::from_bytes(b"MOVE").expect("valid method"),
                self.url_for(from),
            )
            .header("Destination", dest.as_str())
            .header("Overwrite", if overwrite { "T" } else { "F" })
        })
        .await?;
        match resp.status() {
            s if s.is_success() => Ok(Some(true)),
            StatusCode::PRECONDITION_FAILED => Ok(Some(false)),
            StatusCode::NOT_IMPLEMENTED | StatusCode::METHOD_NOT_ALLOWED => Ok(None),
            _ => Err(Self::fail(resp, "move").await),
        }
    }

    async fn move_no_overwrite(&self, from: &str, to: &str) -> Result<Option<bool>> {
        self.move_to(from, to, false).await
    }

    async fn ensure_dir(&self, dir: &str) -> Result<()> {
        if dir.is_empty()
            || self
                .known_dirs
                .lock()
                .map(|k| k.contains(dir))
                .unwrap_or(false)
        {
            return Ok(());
        }
        if let Some((parent, _)) = dir.rsplit_once('/') {
            Box::pin(self.ensure_dir(parent)).await?;
        }
        let mut url = self.url_for(dir);
        let p = format!("{}/", url.path());
        url.set_path(&p);
        let resp = send_retry(|| {
            self.req(
                Method::from_bytes(b"MKCOL").expect("valid method"),
                url.clone(),
            )
        })
        .await?;
        // 405 = already exists; 301/302 some servers use for existing collections.
        match resp.status() {
            s if s.is_success() => {}
            StatusCode::METHOD_NOT_ALLOWED | StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND => {}
            _ => return Err(Self::fail(resp, "mkcol").await),
        }
        if let Ok(mut k) = self.known_dirs.lock() {
            k.insert(dir.to_string());
        }
        Ok(())
    }

    /// One PROPFIND with Depth 1: (relative key, is_collection) for each child.
    async fn list_dir(&self, dir: &str) -> Result<Option<Vec<(String, bool)>>> {
        let mut url = self.url_for(dir);
        if !dir.is_empty() {
            let p = format!("{}/", url.path());
            url.set_path(&p);
        }
        let resp = send_retry(|| {
            self.req(
                Method::from_bytes(b"PROPFIND").expect("valid method"),
                url.clone(),
            )
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(PROPFIND_BODY)
        })
        .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(Self::fail(resp, "propfind").await);
        }
        let body = resp.bytes().await?;
        let base_norm = decode_path(self.base.path());
        let self_norm = if dir.is_empty() {
            base_norm.clone()
        } else {
            format!("{base_norm}/{dir}")
        };

        let mut out = Vec::new();
        for (href, is_dir) in parse_multistatus(&body)? {
            let path_norm = decode_path(&href_path(&href));
            if path_norm == self_norm {
                continue;
            }
            let Some(rel) = path_norm.strip_prefix(&base_norm) else {
                continue;
            };
            let rel = rel.trim_matches('/').to_string();
            if rel.is_empty() || is_noise_key(&rel) {
                continue;
            }
            out.push((rel, is_dir));
        }
        Ok(Some(out))
    }
}

/// (href, is_collection) for each `<response>` in a multistatus body.
fn parse_multistatus(xml: &[u8]) -> Result<Vec<(String, bool)>> {
    let mut reader = Reader::from_reader(xml);
    let mut out = Vec::new();
    let mut href = String::new();
    let mut is_dir = false;
    let mut in_href = false;
    loop {
        match reader
            .read_event()
            .map_err(|e| SyncError::Format(e.to_string()))?
        {
            Event::Start(e) => match e.local_name().as_ref() {
                b"response" => {
                    href.clear();
                    is_dir = false;
                }
                b"href" => in_href = true,
                b"collection" => is_dir = true,
                _ => {}
            },
            Event::Empty(e) => {
                if e.local_name().as_ref() == b"collection" {
                    is_dir = true;
                }
            }
            Event::Text(t) if in_href => {
                href = t
                    .xml_content()
                    .map_err(|e| SyncError::Format(e.to_string()))?
                    .trim()
                    .to_string();
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"href" => in_href = false,
                b"response" => out.push((href.clone(), is_dir)),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Path of an href that may be a full URL or an absolute path.
fn href_path(href: &str) -> String {
    Url::parse(href)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| href.to_string())
}

/// Compare WebDAV paths after percent-decoding so Cyrillic collections match.
fn decode_path(p: &str) -> String {
    percent_decode_str(p)
        .decode_utf8_lossy()
        .trim_end_matches('/')
        .to_string()
}

#[async_trait]
impl Storage for WebDavStorage {
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        // Prefixes are directory-like in this crate; walk from the deepest full directory.
        let start_dir = match prefix.rsplit_once('/') {
            Some((dir, _)) => dir.to_string(),
            None => String::new(),
        };
        let mut out = Vec::new();
        let mut stack = vec![start_dir];
        while let Some(dir) = stack.pop() {
            let Some(children) = self.list_dir(&dir).await? else {
                continue;
            };
            for (rel, is_dir) in children {
                if is_dir {
                    stack.push(rel);
                } else if rel.starts_with(prefix) {
                    out.push(rel);
                }
            }
        }
        Ok(out)
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let url = self.url_for(key);
        let resp = send_retry(|| self.req(Method::GET, url.clone())).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => Ok(Some(resp.bytes().await?.to_vec())),
            _ => Err(Self::fail(resp, "get").await),
        }
    }

    /// PUT is not atomic on WebDAV: a reader could see a half-written chunk or
    /// manifest. Write a temp key and MOVE it into place; plain PUT only when
    /// the server has no MOVE.
    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let tmp = format!("{key}.{}{TMP_SUFFIX}", random_hex(4));
        self.put_raw(&tmp, data).await?;
        match self.move_to(&tmp, key, true).await {
            Ok(Some(true)) => Ok(()),
            Ok(Some(false)) => {
                let _ = self.delete(&tmp).await;
                Err(SyncError::Storage(format!(
                    "webdav move refused to overwrite {key}"
                )))
            }
            Ok(None) => {
                let _ = self.delete(&tmp).await;
                self.put_raw(key, data).await
            }
            Err(e) => {
                let _ = self.delete(&tmp).await;
                Err(e)
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let url = self.url_for(key);
        let resp = send_retry(|| self.req(Method::DELETE, url.clone())).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(()),
            s if s.is_success() => Ok(()),
            _ => Err(Self::fail(resp, "delete").await),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let url = self.url_for(key);
        let resp = send_retry(|| self.req(Method::HEAD, url.clone())).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(false),
            s if s.is_success() => Ok(true),
            _ => Err(Self::fail(resp, "head").await),
        }
    }

    /// PUT is not atomic on WebDAV: write a temp key, then MOVE it into place.
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> Result<bool> {
        if self.exists(key).await? {
            return Ok(false);
        }
        let tmp = format!("{key}.{}{TMP_SUFFIX}", random_hex(4));
        self.put_raw(&tmp, data).await?;
        match self.move_no_overwrite(&tmp, key).await {
            Ok(Some(moved)) => {
                if !moved {
                    self.delete(&tmp).await?;
                }
                Ok(moved)
            }
            Ok(None) => {
                self.delete(&tmp).await?;
                self.put_raw(key, data).await?;
                Ok(true)
            }
            Err(e) => {
                let _ = self.delete(&tmp).await;
                Err(e)
            }
        }
    }
}

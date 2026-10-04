// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! One preview from one address.

use crate::fetch::Fetcher;
use crate::html;
use crate::refuse;
use crate::target::Target;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use messenger_core::Result;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use std::sync::Arc;

/// The head of a page is at its beginning; the rest is not read.
pub const MAX_PAGE_BYTES: usize = 512 * 1024;
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;
/// Pictures that cannot carry a script. SVG can.
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(rename = "LinkPreview")]
pub struct Preview {
    /// The address that was asked.
    pub url: String,
    /// Host that answered, after redirects.
    pub host: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    /// `data:` with the picture itself: showing it asks nobody.
    pub image: Option<String>,
}

#[derive(Clone)]
pub struct PreviewService {
    fetcher: Arc<dyn Fetcher>,
}

impl PreviewService {
    pub fn new(fetcher: Arc<dyn Fetcher>) -> Self {
        Self { fetcher }
    }

    pub async fn preview(&self, url: &str) -> Result<Preview> {
        let target = Target::parse(url)?;
        let page = self.fetcher.get(&target, "text/html,application/xhtml+xml", MAX_PAGE_BYTES).await?;
        if page.content_type != "text/html" && page.content_type != "application/xhtml+xml" {
            return Err(refuse("preview_not_a_page"));
        }
        // A fetcher is trusted to fetch, not to decide where the answer may come from.
        let answered = Target::parse(&page.url)?;
        let meta = html::read(&String::from_utf8_lossy(&page.body));
        let image = match &meta.image {
            Some(reference) => self.picture(&answered, reference).await,
            None => None,
        };
        if meta.title.is_none() && meta.description.is_none() && image.is_none() {
            return Err(refuse("preview_empty"));
        }
        Ok(Preview {
            url: target.url().to_string(),
            host: answered.host().to_string(),
            title: meta.title,
            description: meta.description,
            site_name: meta.site_name,
            image,
        })
    }

    /// A preview without its picture is still a preview.
    async fn picture(&self, page: &Target, reference: &str) -> Option<String> {
        let target = page.join(reference).ok()?;
        let got = self.fetcher.get(&target, "image/png,image/jpeg,image/webp,image/gif", MAX_IMAGE_BYTES).await.ok()?;
        if got.truncated || got.body.is_empty() || !IMAGE_TYPES.contains(&got.content_type.as_str()) {
            return None;
        }
        Target::parse(&got.url).ok()?;
        Some(format!("data:{};base64,{}", got.content_type, B64.encode(&got.body)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Fetched;
    use async_trait::async_trait;
    use messenger_core::MessengerError;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        pages: HashMap<String, Fetched>,
        asked: Mutex<Vec<String>>,
    }

    impl Fake {
        fn with(mut self, url: &str, content_type: &str, body: &[u8]) -> Self {
            self.pages.insert(url.into(), Fetched { url: url.into(), content_type: content_type.into(), body: body.to_vec(), truncated: false });
            self
        }

        fn answered_from(mut self, url: &str, from: &str) -> Self {
            self.pages.get_mut(url).unwrap().url = from.into();
            self
        }
    }

    #[async_trait]
    impl Fetcher for Fake {
        async fn get(&self, target: &Target, _accept: &str, max_bytes: usize) -> Result<Fetched> {
            self.asked.lock().unwrap().push(target.url().to_string());
            let mut f = self.pages.get(target.url()).cloned().ok_or_else(|| refuse("preview_failed"))?;
            if f.body.len() > max_bytes {
                f.body.truncate(max_bytes);
                f.truncated = true;
            }
            Ok(f)
        }
    }

    fn code(r: Result<Preview>) -> String {
        match r {
            Err(MessengerError::Invalid(c)) => c,
            other => panic!("{other:?}"),
        }
    }

    const PAGE: &str = r#"<head><title>T</title><meta property="og:description" content="D"><meta property="og:image" content="/c.png"></head>"#;

    #[tokio::test]
    async fn a_page_with_a_picture() {
        let fake = Arc::new(
            Fake::default()
                .with("https://example.com/a", "text/html", PAGE.as_bytes())
                .with("https://example.com/c.png", "image/png", b"\x89PNG"),
        );
        let p = PreviewService::new(fake.clone()).preview(" https://example.com/a ").await.unwrap();
        assert_eq!(
            p,
            Preview {
                url: "https://example.com/a".into(),
                host: "example.com".into(),
                title: Some("T".into()),
                description: Some("D".into()),
                site_name: None,
                image: Some("data:image/png;base64,iVBORw==".into()),
            }
        );
        assert_eq!(*fake.asked.lock().unwrap(), vec!["https://example.com/a", "https://example.com/c.png"]);
    }

    #[tokio::test]
    async fn the_picture_is_found_where_the_answer_came_from() {
        let fake = Arc::new(
            Fake::default()
                .with("https://short.example/x", "text/html", PAGE.as_bytes())
                .answered_from("https://short.example/x", "https://long.example/news/1")
                .with("https://long.example/c.png", "image/jpeg", b"jpg"),
        );
        let p = PreviewService::new(fake).preview("https://short.example/x").await.unwrap();
        assert_eq!(p.url, "https://short.example/x");
        assert_eq!(p.host, "long.example");
        assert!(p.image.unwrap().starts_with("data:image/jpeg;base64,"));
    }

    #[tokio::test]
    async fn a_picture_that_may_not_be_shown_is_left_out() {
        let big = vec![0u8; MAX_IMAGE_BYTES + 1];
        for (kind, body) in [("image/svg+xml", b"<svg/>".as_slice()), ("text/html", b"<html>".as_slice()), ("image/png", big.as_slice()), ("image/png", b"".as_slice())] {
            let fake = Fake::default().with("https://example.com/a", "text/html", PAGE.as_bytes()).with("https://example.com/c.png", kind, body);
            let p = PreviewService::new(Arc::new(fake)).preview("https://example.com/a").await.unwrap();
            assert!(p.image.is_none(), "{kind}");
            assert_eq!(p.title.as_deref(), Some("T"));
        }
        // A picture inside is not even asked for.
        let inside = PAGE.replace("/c.png", "https://192.168.1.1/c.png");
        let fake = Arc::new(Fake::default().with("https://example.com/a", "text/html", inside.as_bytes()));
        let p = PreviewService::new(fake.clone()).preview("https://example.com/a").await.unwrap();
        assert!(p.image.is_none());
        assert_eq!(fake.asked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refusals() {
        let fake = Arc::new(
            Fake::default()
                .with("https://example.com/file.pdf", "application/pdf", b"%PDF")
                .with("https://example.com/empty", "text/html", b"<html><body>nothing to say</body></html>")
                .with("https://example.com/moved", "text/html", PAGE.as_bytes())
                .answered_from("https://example.com/moved", "https://10.0.0.1/admin"),
        );
        let svc = PreviewService::new(fake.clone());
        assert_eq!(code(svc.preview("http://example.com/a").await), "preview_not_https");
        assert_eq!(code(svc.preview("https://127.0.0.1/").await), "preview_private_host");
        assert_eq!(code(svc.preview("veydan://group/x").await), "preview_not_https");
        assert!(fake.asked.lock().unwrap().is_empty(), "refused before the network");
        assert_eq!(code(svc.preview("https://example.com/file.pdf").await), "preview_not_a_page");
        assert_eq!(code(svc.preview("https://example.com/empty").await), "preview_empty");
        assert_eq!(code(svc.preview("https://example.com/moved").await), "preview_private_host");
        assert_eq!(code(svc.preview("https://example.com/absent").await), "preview_failed");
    }

    #[tokio::test]
    async fn only_the_beginning_of_a_long_page_is_read() {
        let mut long = PAGE.as_bytes().to_vec();
        long.extend(std::iter::repeat_n(b' ', MAX_PAGE_BYTES));
        long.extend_from_slice(br#"<meta property="og:title" content="too far">"#);
        let fake = Fake::default().with("https://example.com/a", "text/html", &long);
        let p = PreviewService::new(Arc::new(fake)).preview("https://example.com/a").await.unwrap();
        assert_eq!(p.title.as_deref(), Some("T"));
    }
}

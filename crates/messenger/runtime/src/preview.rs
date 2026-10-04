// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Previews of pages outside: asked for by the user, one page at a time,
//! remembered so that the same page is not asked twice, and never asked
//! in silent mode.

use crate::relays::RelayService;
use crate::MessengerRuntime;
use messenger_core::{Clock, MessengerError, Result};
use messenger_preview::{Preview, PreviewService, Target};
use messenger_store::{link_previews as repo, Store};
use std::sync::Arc;

/// A page changes; a week later it is asked again.
const KEEP_SECS: i64 = 7 * 24 * 3600;
const KEEP_ROWS: i64 = 500;

#[derive(Clone)]
pub struct LinkPreviews {
    store: Store,
    relays: Arc<RelayService>,
    service: PreviewService,
    clock: Arc<dyn Clock>,
}

impl LinkPreviews {
    pub fn new(store: Store, relays: Arc<RelayService>, service: PreviewService, clock: Arc<dyn Clock>) -> Self {
        Self { store, relays, service, clock }
    }

    pub async fn get(&self, url: &str) -> Result<Preview> {
        // Refused here, before anything is looked up: a bad address has no preview, stored or not.
        let target = Target::parse(url)?;
        let now = self.clock.now().secs();
        if let Some(json) = repo::get(&self.store, target.url(), now - KEEP_SECS).await? {
            if let Ok(p) = serde_json::from_str::<Preview>(&json) {
                return Ok(p);
            }
        }
        if self.relays.is_silent().await? {
            return Err(MessengerError::Invalid("preview_silent".into()));
        }
        let preview = self.service.preview(target.url()).await?;
        repo::put(&self.store, target.url(), &serde_json::to_string(&preview)?, now).await?;
        repo::prune(&self.store, now - KEEP_SECS, KEEP_ROWS).await?;
        Ok(preview)
    }
}

impl MessengerRuntime {
    /// What the page behind an `https` link says about itself. Goes to the
    /// network: call it when the user asked, not when a message is shown.
    pub async fn link_preview(&self, url: &str) -> Result<Preview> {
        self.previews.get(url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use messenger_core::traits::SystemClock;
    use messenger_preview::{Fetched, Fetcher};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(AtomicUsize);

    #[async_trait]
    impl Fetcher for Counting {
        async fn get(&self, target: &Target, _accept: &str, _max: usize) -> Result<Fetched> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Fetched {
                url: target.url().into(),
                content_type: "text/html".into(),
                body: b"<title>Hello</title>".to_vec(),
                truncated: false,
            })
        }
    }

    fn code<T: std::fmt::Debug>(r: Result<T>) -> String {
        match r {
            Err(MessengerError::Invalid(c)) => c,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn asked_once_kept_and_silent_when_told() {
        let store = Store::open_in_memory().await.unwrap();
        let relays = Arc::new(RelayService::init(store.clone(), None).await.unwrap());
        let fetcher = Arc::new(Counting(AtomicUsize::new(0)));
        let previews = LinkPreviews::new(store, relays.clone(), PreviewService::new(fetcher.clone()), Arc::new(SystemClock));

        let first = previews.get("https://example.com/a").await.unwrap();
        assert_eq!(first.title.as_deref(), Some("Hello"));
        assert_eq!(previews.get(" https://example.com/a ").await.unwrap(), first);
        assert_eq!(fetcher.0.load(Ordering::SeqCst), 1, "the second answer is from the store");

        relays.set_silent(true).await.unwrap();
        assert_eq!(previews.get("https://example.com/a").await.unwrap(), first, "what is known stays known");
        assert_eq!(code(previews.get("https://example.com/b").await), "preview_silent");
        assert_eq!(code(previews.get("http://example.com/a").await), "preview_not_https");
        assert_eq!(code(previews.get("https://192.168.0.1/").await), "preview_private_host");
        assert_eq!(fetcher.0.load(Ordering::SeqCst), 1);
    }
}

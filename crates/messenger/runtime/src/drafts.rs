// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The unsent text of each conversation, kept on this device so that it
//! outlives the app (ROO-1112). It goes nowhere: not to a relay, not to my
//! other devices, not to the archive.

use crate::MessengerRuntime;
use messenger_core::{MessengerError, Result};
pub use messenger_store::drafts::DraftRow as DraftView;

/// The longest key of a draft: a chat id, `group:<id>`, `<chat>#<post>`.
pub const DRAFT_KEY_MAX_LEN: usize = 256;
/// The longest draft kept, in bytes.
pub const DRAFT_TEXT_MAX_LEN: usize = 64 * 1024;
/// A key or a text out of bounds.
pub const ERR_DRAFT_INVALID: &str = "draft_invalid";

impl MessengerRuntime {
    /// Every draft this device keeps, the newest first.
    pub async fn drafts_get(&self) -> Result<Vec<DraftView>> {
        messenger_store::drafts::list(&self.store).await
    }

    /// Keep `text` as the draft of `chat`; an empty text (or one of blanks)
    /// forgets it. Error: `draft_invalid`.
    pub async fn draft_set(&self, chat: &str, text: &str, md: bool) -> Result<()> {
        if chat.is_empty() || chat.len() > DRAFT_KEY_MAX_LEN || text.len() > DRAFT_TEXT_MAX_LEN {
            return Err(MessengerError::Invalid(ERR_DRAFT_INVALID.into()));
        }
        messenger_store::drafts::set(&self.store, chat, text, md).await
    }
}

#[cfg(test)]
mod tests {
    use crate::receipts::tests::with_a_message_of;
    use nostr::key::Keys;

    #[tokio::test]
    async fn a_draft_outlives_nothing_but_its_bounds() {
        let peer = Keys::generate();
        let (_dir, rt, _now) = with_a_message_of(&peer).await;
        assert!(rt.draft_set("", "x", false).await.is_err());
        assert!(rt.draft_set(&"a".repeat(257), "x", false).await.is_err());
        assert!(rt.draft_set("aa", &"x".repeat(64 * 1024 + 1), false).await.is_err());

        rt.draft_set("aa", "QA1105 draft", false).await.unwrap();
        let kept = rt.drafts_get().await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].chat.as_str(), kept[0].text.as_str(), kept[0].md), ("aa", "QA1105 draft", false));
        let queued = messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap().len();
        rt.draft_set("aa", "QA1105 draft, more", true).await.unwrap();
        assert_eq!(messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap().len(), queued, "a draft sends nothing");

        rt.draft_set("aa", "", false).await.unwrap();
        assert!(rt.drafts_get().await.unwrap().is_empty());
    }
}

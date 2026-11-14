// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The search of the conversation (ROO-1137): in one chat, as a filter of
//! its messages, and across the chats, as the list of chats finds them.
//! Read from the index of this device (`messenger_store::search`); nothing
//! is asked of a server, nothing of the query leaves the device.

use crate::MessengerRuntime;
use messenger_core::{MessengerError, Result};
use messenger_dm::MessageView;
use messenger_store::{search, settings, Store};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The longest query, in bytes.
pub const SEARCH_QUERY_MAX_LEN: usize = 256;
/// A query longer than [`SEARCH_QUERY_MAX_LEN`].
pub const ERR_SEARCH_INVALID: &str = "search_invalid";
/// A page of the search in a chat, unless asked for another.
pub const SEARCH_PAGE: u32 = 50;
const SEARCH_LIMIT_MAX: u32 = 200;

/// Where a page of the search in a chat ends: the next page is of the
/// messages strictly older than this one, by its time and then its id (of
/// the same second, none is lost or given twice).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SearchCursor {
    #[ts(type = "number")]
    pub created_at: i64,
    pub id: String,
}

/// What the search found in one chat (`messenger_search_messages`).
#[derive(Clone, Debug, Serialize, TS)]
pub struct SearchPage {
    /// The messages that hold every word, as the chat shows them, the
    /// newest first.
    #[ts(type = "MessengerMessage[]")]
    pub messages: Vec<MessageView>,
    /// How many messages of the chat match in all.
    pub total: u32,
    /// What to pass as `before` for the next page; absent on the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_before: Option<SearchCursor>,
    /// The index does not hold every message of this device yet (it is
    /// still being written, after an update): older ones may be missing.
    pub partial: bool,
}

/// A chat whose messages hold the words (`messenger_search_chats`).
#[derive(Clone, Debug, Serialize, TS)]
pub struct ChatHit {
    pub chat_id: String,
    /// How many of its messages match.
    pub count: u32,
    /// The newest of them: the line the list shows, and where the chat
    /// opens.
    #[ts(type = "MessengerMessage")]
    pub last: MessageView,
}

/// What the search found across the chats.
#[derive(Clone, Debug, Serialize, TS)]
pub struct ChatSearchResult {
    /// The chat with the newest match first.
    pub hits: Vec<ChatHit>,
    /// As in [`SearchPage`].
    pub partial: bool,
}

/// The query as it is searched: `None` for one of blanks only (an empty
/// answer, not an error). Error: `search_invalid` past 256 bytes.
fn checked(query: &str) -> Result<Option<&str>> {
    if query.len() > SEARCH_QUERY_MAX_LEN {
        return Err(MessengerError::Invalid(ERR_SEARCH_INVALID.into()));
    }
    Ok(Some(query.trim()).filter(|q| !q.is_empty()))
}

impl MessengerRuntime {
    /// The messages of a chat that hold every word of `query` (each as the
    /// start of a word, in any case, with or without marks), as I see the
    /// chat: in a group I moderate, those that wait for a moderator too.
    /// Past `before` (the `next_before` of the page before) when given.
    pub async fn search_messages(&self, chat_id: &str, query: &str, before: Option<SearchCursor>, limit: Option<u32>) -> Result<SearchPage> {
        let Some(query) = checked(query)? else { return Ok(SearchPage { messages: Vec::new(), total: 0, next_before: None, partial: false }) };
        let moderator = match (chat_id.strip_prefix("group:"), self.session_pubkey().await) {
            (Some(group), Some(me)) => self.groups().is_moderator(group, &me).await?,
            _ => false,
        };
        let limit = limit.unwrap_or(SEARCH_PAGE).clamp(1, SEARCH_LIMIT_MAX);
        let before = before.map(|b| search::Cursor { created_at: b.created_at, id: b.id });
        let (messages, total, next) = self.dm().search_in(chat_id, query, before.as_ref(), limit as i64, moderator).await?;
        let next_before = next.map(|c| SearchCursor { created_at: c.created_at, id: c.id });
        let partial = !search::is_whole(&self.store).await?;
        Ok(SearchPage { messages, total: total.clamp(0, u32::MAX as i64) as u32, next_before, partial })
    }

    /// The chats whose messages hold every word of `query`, the one with
    /// the newest match first, at most `limit`.
    pub async fn search_chats(&self, query: &str, limit: Option<u32>) -> Result<ChatSearchResult> {
        let Some(query) = checked(query)? else { return Ok(ChatSearchResult { hits: Vec::new(), partial: false }) };
        let mut moderated = Vec::new();
        if let Some(me) = self.session_pubkey().await {
            for chat in search::groups_matching(&self.store, query).await? {
                let group = chat.trim_start_matches("group:");
                if self.groups().is_moderator(group, &me).await? {
                    moderated.push(chat);
                }
            }
        }
        let limit = limit.unwrap_or(SEARCH_PAGE).clamp(1, SEARCH_LIMIT_MAX);
        let hits = self
            .dm()
            .search_across(query, limit as i64, &moderated)
            .await?
            .into_iter()
            .map(|(chat_id, count, last)| ChatHit { chat_id, count: count.clamp(0, u32::MAX as i64) as u32, last })
            .collect();
        Ok(ChatSearchResult { hits, partial: !search::is_whole(&self.store).await? })
    }

    /// Put in the index the messages that came before it, now, until all
    /// are; `step` hears the rows read so far after each batch (the CLI's
    /// `search-index`; the runtime does it by itself in the background at
    /// its start).
    pub async fn search_backfill(&self, step: impl FnMut(u64)) -> Result<Backfilled> {
        let _one = self.search_backfill.lock().await;
        backfill(&self.store, step).await
    }
}

/// What a run of the backfill did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backfilled {
    /// Rows of `msg_messages` read.
    pub rows: u64,
    /// Transactions of [`search::BACKFILL_BATCH`] rows.
    pub batches: u64,
    /// Every message is in the index now; `false`: the store changed hands
    /// (a wipe, another key) and the run stopped.
    pub whole: bool,
}

/// The backfill of the index (`search::backfill_batch`), one batch after
/// another until all is in, for the owner the store has now; `step` hears
/// the rows read so far after each batch. Between two batches others may
/// write: each one holds the store for writing while it runs.
pub(crate) async fn backfill(store: &Store, mut step: impl FnMut(u64)) -> Result<Backfilled> {
    search::prepare(store).await?;
    let owner = settings::get(store, messenger_store::account::KEY_OWNER).await?.filter(|o| !o.is_empty());
    let mut done = Backfilled::default();
    loop {
        match search::backfill_batch(store, owner.as_deref()).await? {
            search::Step::More(n) => {
                done.rows += n;
                done.batches += 1;
                step(done.rows);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            search::Step::Whole(n) => {
                done.rows += n;
                done.batches += u64::from(n > 0);
                done.whole = true;
                return Ok(done);
            }
            search::Step::Stopped => return Ok(done),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::receipts::tests::with_a_message_of;
    use nostr::key::Keys;

    #[tokio::test]
    async fn a_message_is_found_in_its_chat_and_across_the_chats() {
        let peer = Keys::generate();
        let (_dir, rt, _now) = with_a_message_of(&peer).await;
        let chat = format!("dm:{}", peer.public_key().to_hex());
        let done = rt.search_backfill(|_| {}).await.unwrap();
        assert!(done.whole);

        let page = rt.search_messages(&chat, "HI", None, None).await.unwrap();
        assert_eq!((page.messages.len(), page.total, page.partial), (1, 1, false));
        assert_eq!(page.messages[0].text.as_deref(), Some("hi"));
        let full = rt.search_messages(&chat, "hi", None, Some(1)).await.unwrap();
        assert_eq!((full.messages.len(), full.next_before), (1, None), "a full last page has no page after it");
        let found = rt.search_chats("h", None).await.unwrap();
        assert_eq!(found.hits.iter().map(|h| (h.chat_id.as_str(), h.count)).collect::<Vec<_>>(), [(chat.as_str(), 1)]);
        assert_eq!(found.hits[0].last.id, page.messages[0].id);

        let empty = rt.search_messages(&chat, "   ", None, None).await.unwrap();
        assert_eq!((empty.messages.len(), empty.total), (0, 0), "a query of blanks: nothing, no error");
        assert!(rt.search_chats("", None).await.unwrap().hits.is_empty());
        for long in [rt.search_messages(&chat, &"я".repeat(129), None, None).await.err(), rt.search_chats(&"x".repeat(257), None).await.err()] {
            assert!(long.is_some_and(|e| e.to_string().contains(super::ERR_SEARCH_INVALID)));
        }
        assert!(rt.search_chats(&"x".repeat(256), None).await.is_ok(), "256 bytes is the bound");

        // An edit changes what is found, a deletion takes it out.
        let id = page.messages[0].id.clone();
        messenger_store::messages::set_text(rt.store(), &id, "пока", messenger_core::envelope::TextFormat::Plain, 1).await.unwrap();
        assert_eq!(rt.search_messages(&chat, "hi", None, None).await.unwrap().total, 0);
        assert_eq!(rt.search_messages(&chat, "пок", None, None).await.unwrap().total, 1);
        rt.dm().delete_local(&id).await.unwrap();
        assert_eq!(rt.search_messages(&chat, "пок", None, None).await.unwrap().total, 0);
    }

    /// In a group I moderate (mine: I made it) what waits for a moderator
    /// is found, in a group of others it is not; what a moderator hid is
    /// found in neither.
    #[tokio::test]
    async fn a_moderator_finds_what_waits_a_member_does_not() {
        use messenger_store::messages::{self, NewMessage, MOD_HIDDEN, REVIEW_PENDING};
        let peer = Keys::generate();
        let (_dir, rt, now) = with_a_message_of(&peer).await;
        let mine = rt.group_create(messenger_groups::GroupKind::Private, "Team", "", true, None).await.unwrap().chat_id;
        let theirs = format!("group:{}", "cd".repeat(32));
        sqlx::query("INSERT OR IGNORE INTO msg_chats (id, kind, created_at, updated_at) VALUES (?, 'group', 1, 1)")
            .bind(&theirs)
            .execute(rt.store().pool())
            .await
            .unwrap();
        for chat in [&mine, &theirs] {
            for (n, state) in ["shown", "waits", "hidden"].into_iter().enumerate() {
                let id = format!("{}{n}", &chat[6..12]);
                let row = NewMessage {
                    id: id.clone(),
                    chat_id: chat.to_string(),
                    wire_id: Some(format!("w{id}")),
                    direction: "in".into(),
                    status: "received".into(),
                    content_type: "text".into(),
                    text: Some(format!("собрание {state}")),
                    envelope_json: "{}".into(),
                    sender_pubkey: peer.public_key().to_hex(),
                    reply_to_id: None,
                    target_id: None,
                    created_at: now + n as i64,
                    is_hidden: false,
                    outbox_local_id: None,
                    media_json: None,
                };
                messages::insert(rt.store(), &row).await.unwrap();
                match state {
                    "waits" => assert!(messages::set_review(rt.store(), &id, REVIEW_PENDING).await.unwrap()),
                    "hidden" => assert!(messages::set_mod_state(rt.store(), &id, MOD_HIDDEN).await.unwrap()),
                    _ => {}
                }
            }
        }
        let texts = |p: super::SearchPage| p.messages.into_iter().map(|m| m.text.unwrap_or_default()).collect::<Vec<_>>();
        assert_eq!(texts(rt.search_messages(&mine, "собр", None, None).await.unwrap()), ["собрание waits", "собрание shown"]);
        assert_eq!(texts(rt.search_messages(&theirs, "собр", None, None).await.unwrap()), ["собрание shown"]);
        let counts = rt.search_chats("собрание", None).await.unwrap().hits.into_iter().map(|h| (h.chat_id, h.count)).collect::<Vec<_>>();
        assert_eq!(counts, [(mine.clone(), 2), (theirs.clone(), 1)]);
    }
}

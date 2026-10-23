// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the channels show outside their own pages, through the seams and
//! the services of the shell (internal/communities-stage1.md §6.2):
//!
//! - the rows of the list of chats are `channels_list`, each with its
//!   `chat_id` `channel:<id>`; what the user chose of them (pin, mute,
//!   archive) is the messenger's to keep and to carry to the other
//!   devices, chosen by its commands and read back here (`ChatPrefs`);
//! - the count of what is unread, beside the messenger's on the tray and
//!   the icon of the app (`Shell::set_unread`), muted channels left out as
//!   the messenger leaves its muted chats out;
//! - the network and the identity at work are the messenger's
//!   (`NetworkSession`, `messenger_core::Transport`; see `state`).

use crate::state::Driver;
use crate::views::chat_id;
use communities_core::db;
use std::collections::HashMap;
use std::time::Duration;
use veydan_shell::seams::{self, ChatPref, ChatPrefs};

/// How long the messages of a burst are gathered before the count is read
/// again.
const UNREAD_PAUSE: Duration = Duration::from_millis(500);

/// The prefix of the chats of channels in the messenger's list.
const CHAT_PREFIX: &str = "channel:";

/// What the user chose of the chats of channels, by chat id; none while
/// the messenger is not in the product or does not run.
pub(crate) async fn chat_prefs(d: &Driver) -> HashMap<String, ChatPref> {
    let Some(prefs) = seams::seam::<dyn ChatPrefs, _, _>(&d.app) else { return HashMap::new() };
    match prefs.prefs(CHAT_PREFIX).await {
        Ok(list) => list.into_iter().map(|p| (p.chat_id.clone(), p)).collect(),
        Err(e) => {
            crate::log::clog!("the choices of the chats were not read: {e}");
            HashMap::new()
        }
    }
}

/// The count of the unread is to be told again: once for a burst of
/// messages, after a short pause, and not by the one who asks.
pub(crate) fn unread_changed(d: &Driver) {
    let driver = d.clone();
    d.unread.ask(UNREAD_PAUSE, async move { unread_now(&driver).await });
}

/// The sum of the unread posts of the channels not muted, told to the
/// shell: two queries, whatever the number of channels.
async fn unread_now(d: &Driver) {
    let db = d.service.db();
    let me = d.service.signer().map(|k| k.public_key().to_hex()).unwrap_or_default();
    let (rows, counts) = match (db::channels::list(db).await, db::content::unread_all(db, &me).await) {
        (Ok(rows), Ok(counts)) => (rows, counts),
        (Err(e), _) | (_, Err(e)) => {
            crate::log::clog!("the unread were not counted: {e}");
            return;
        }
    };
    let prefs = chat_prefs(d).await;
    let n: i64 = rows
        .iter()
        .filter(|row| !prefs.get(&chat_id(&row.id)).is_some_and(|p| p.muted))
        .filter_map(|row| counts.get(&row.id))
        .sum();
    set_unread(&d.app, usize::try_from(n).unwrap_or(0));
}

/// Nothing waits once the module is off.
pub(crate) fn clear_unread(app: &tauri::AppHandle) {
    set_unread(app, 0);
}

#[cfg(desktop)]
fn set_unread(app: &tauri::AppHandle, n: usize) {
    use tauri::Manager;
    if let Some(shell) = app.try_state::<veydan_shell::Shell>() {
        shell.set_unread(crate::MODULE_ID, n);
    }
}

/// A phone shows no count of its own: the push of the messenger does.
#[cfg(mobile)]
fn set_unread(_app: &tauri::AppHandle, _n: usize) {}

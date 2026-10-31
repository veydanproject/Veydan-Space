// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! TypeScript types of what the commands of the channels hand to the UI
//! and take from it, written from the Rust types themselves so the two
//! cannot drift apart, as the messenger's are (messenger-runtime/src/bindings.rs).
//!
//! The files live with the UI (`ui/src/lib/communities/generated/`). A test
//! compares them with what the types say now and fails when one is stale;
//! `make msg-types` (or `UPDATE_TS_BINDINGS=1 cargo test -p
//! veydan-communities bindings`) writes them again.

use crate::views::{
    ChannelAction, ChannelDetail, ChannelPreview, ChannelView, FeedCursorView, FeedItemView, FeedPageView, InviteStatusView, InviteView, KeeperStatus, LastPost, LevelInput, TicketsView,
    LevelRequestView, LevelRights, LevelView, LogEntry, LogPage, ModAction, ModSettings, MyLevelRequestView, NewChannelInput, NewScopeInput, OutboxView, ScopeSetInput, QueueItem, QueuePage, ReactionCount, RequestView,
    ReaderView, ReadersPage, ScheduleView, ScopeView, StaffView, StoragePlanView, TransferView, UploadView,
};
use ts_rs::{Config, TS};

/// Relative to this crate.
pub const CHANNELS_FILE: &str = "../../ui/src/lib/communities/generated/channels.ts";
pub const MODERATION_FILE: &str = "../../ui/src/lib/communities/generated/moderation.ts";

const HEADER: &str = "// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (veydan-communities/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.
";

fn export<T: TS>(cfg: &Config, doc: &str) -> String {
    format!("\n/** {doc} */\nexport {}\n", T::decl(cfg))
}

/// The channels, their feed, the files on their way, the invitations.
pub fn channels_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    // A Markdown text is the messenger's blocks, handed on by its seam.
    out += "\nimport type { MdBlock } from '$lib/messenger/seams/shared';\n";
    out += &export::<LastPost>(&cfg, "The newest post of a channel, for its row in the list of chats.");
    out += &export::<ChannelView>(&cfg, "A channel as `channels_list` lists it: the row of the list of chats under `chat_id` (`channel:<id>`), with what the user chose of it in the messenger.");
    out += &export::<ScheduleView>(&cfg, "When readers may post: days 1 (Monday) to 7, `from` and `to` as HH:MM in `tz`; no days takes the schedule away.");
    out += &export::<ScopeView>(&cfg, "A section of the channel: a scope of its own keys (`everyone` is the base one, its title empty).");
    out += &export::<LevelRights>(&cfg, "What a level lets do beyond reading its sections.");
    out += &export::<LevelView>(&cfg, "A level: a name the owner gives, the sections it opens and its rights. Id 0 is the base level of everyone in.");
    out += &export::<LevelRequestView>(&cfg, "A reader asks for a level (`channel.level_request`), for the moderators.");
    out += &export::<NewScopeInput>(&cfg, "`channel_scope_add`: a new section, its look, hidden or not.");
    out += &export::<ScopeSetInput>(&cfg, "`channel_scope_set`: what changes of a section (the owner); a field left out stays.");
    out += &export::<LevelInput>(&cfg, "`channel_level_set`: a level made (no `id`) or changed.");
    out += &export::<StaffView>(&cfg, "Somebody the journal names: staff, or a reader it mutes or trusts.");
    out += &export::<TransferView>(&cfg, "A handover of the channel to a new owner (communities-wire §16): offered to me (`in`) or by me (`out`).");
    out += &export::<OutboxView>(&cfg, "What of mine in a channel still waits for the relays; `channel_outbox_retry` tries it now, `channel.outbox` tells of each row.");
    out += &export::<MyLevelRequestView>(&cfg, "A request for a level I made: waiting, or refused with the moderator's reason until I ask again (communities-wire §14.4).");
    out += &export::<ChannelDetail>(&cfg, "A channel as `channel_get` and the commands that change it answer: the row, the rules, the staff, whether a key is here, the link to hand on.");
    out += &export::<NewChannelInput>(&cfg, "What `channel_create` takes: a name, how readers get in, the relays and the blob hosts of the channel.");
    out += &export::<ChannelAction>(&cfg, "An operation of the journal as `channel_act` takes it, tagged by `op`; `leave` leaves the channel.");
    out += &export::<ReactionCount>(&cfg, "How many set one reaction, and whether I am one of them.");
    out += &export::<FeedItemView>(&cfg, "A post or a comment as a page shows it.");
    out += &export::<FeedCursorView>(&cfg, "Where the next page starts: `next` of the page before, passed back as it came.");
    out += &export::<FeedPageView>(&cfg, "A page of `channel_history` or `channel_thread`; `asked`: the relays were asked for more, which comes as `channel.message`.");
    out += &export::<UploadView>(&cfg, "A file of mine on its way to the hosts of a channel (`channel_media_send`, `channel_uploads`); its progress comes as `channel.media`.");
    out += &export::<crate::commands::ImportedFile>(&cfg, "A file picked to be sent, a file of the device now (`channel_media_import`); `url`: where the page reads a video for its frame; `preview`: a picture of it; `note`: `unsupported_format`, a photo that goes as a file.");
    out += &export::<ChannelPreview>(&cfg, "The card of a channel from its link, before joining (`channel_preview_link`).");
    out += &export::<StoragePlanView>(&cfg, "Where the files of a new channel of mine live (`channel_storage_plan`): the free public media of the messenger without Veydan Cloud.");
    out += &export::<InviteView>(&cfg, "An invitation to a channel, one that came to me (`in`) or one I sent (`out`).");
    out += &export::<RequestView>(&cfg, "A request to be let into a channel, waiting for a moderator.");
    out
}

/// The moderation of a channel and its keeper.
pub fn moderation_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    // The rules name the schedule, a type of the other file.
    out += "\nimport type { ScheduleView } from './channels';\n";
    out += &export::<QueueItem>(&cfg, "A message waiting for a moderator: one line per author, or each of one author's when the queue is asked for theirs.");
    out += &export::<QueuePage>(&cfg, "A page of the queue, as `channel_moderation_queue` answers (`author`: that author's messages alone).");
    out += &export::<ReaderView>(&cfg, "A reader of a channel as its team sees them (`channel_readers`); the journal does not list readers, the keeper's rows or its snapshot do.");
    out += &export::<ReadersPage>(&cfg, "A page of the readers, as `channel_readers` answers: newest first, the staff, the banned and me left out; `partial`: shards of the snapshot still on their way.");
    out += &export::<ModAction>(&cfg, "One act of moderation, as `channel_moderation_mod` takes it; tagged by `t`.");
    out += &export::<LogEntry>(&cfg, "One line of the log of moderation.");
    out += &export::<LogPage>(&cfg, "A page of the log, as `channel_moderation_log` answers.");
    out += &export::<ModSettings>(&cfg, "The rules of a channel, as `channel_moderation_settings` answers and `channel_moderation_settings_set` takes them; a field left out stays as it is.");
    out += &export::<InviteStatusView>(&cfg, "The invitations open now against their limits (`channel_moderation_invite_status`).");
    out += &export::<TicketsView>(&cfg, "The tickets a moderator holds, as `channel_tickets` and `channel_tickets_request` answer.");
    out += &export::<KeeperStatus>(&cfg, "The keeper of a channel, as `channel_keeper_status` and `channel_rotate_now` answer.");
    out
}

/// Every generated file and what it should hold.
pub fn files() -> [(&'static str, String); 2] {
    [(CHANNELS_FILE, channels_ts()), (MODERATION_FILE, moderation_ts())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn bindings_are_current() {
        for (file, want) in files() {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
            if std::env::var_os("UPDATE_TS_BINDINGS").is_some() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &want).unwrap();
                continue;
            }
            let have = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(have == want, "{} is stale: run `make msg-types`", path.display());
        }
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! TypeScript types of what the runtime hands to the UI, written from the
//! Rust types themselves so the two cannot drift apart.
//!
//! The files live with the UI (`ui/src/lib/messenger/generated/`). A test
//! compares them with what the types say now and fails when one is stale;
//! `make msg-types` (or `UPDATE_TS_BINDINGS=1 cargo test -p
//! messenger-runtime bindings`) writes them again.

use crate::avatars::AvatarPreview;
use crate::calls::{
    CallDirection, CallEnded, CallIncoming, CallLimits, CallMedia, CallNodeInput, CallNodeView, CallOutcome, CallPhase, CallState,
    CallStats, CallVia, CallView, CameraInfo, GroupCallAnnounced, GroupCallEnded, GroupCallLevel, GroupCallPhase,
    GroupCallState, GroupCallView, GroupParticipant, ReconnectReason, RelayPolicy, ScreenInfo, VideoInput, VideoQuality,
    VideoSize, VideoTrack,
};
use crate::cards::{ContactPrivateView, OwnPrivateView};
use crate::links::{GroupMembership, LinkGroupKind, LinkView};
use crate::net::{BridgeView, NetCheck, NetMode, NetStatus, Verdict};
use crate::shared::{SharedCounts, SharedSection};
use messenger_avatar::CropRect;
use messenger_media::{Progress, TransferStage, TransferView};
use messenger_contacts::{CardView, ProfileInput, ProfileView, SocialLink, SocialPlatform, SocialView};
use messenger_preview::Preview;
use messenger_richtext::{Color, Span, Style};
use ts_rs::{Config, TS};

/// Relative to this crate.
pub const LINKS_FILE: &str = "../../../ui/src/lib/messenger/generated/links.ts";
pub const SHARED_FILE: &str = "../../../ui/src/lib/messenger/generated/shared.ts";
pub const NET_FILE: &str = "../../../ui/src/lib/messenger/generated/net.ts";
pub const PROFILE_FILE: &str = "../../../ui/src/lib/messenger/generated/profile.ts";
pub const TRANSFER_FILE: &str = "../../../ui/src/lib/messenger/generated/transfer.ts";
pub const CALLS_FILE: &str = "../../../ui/src/lib/messenger/generated/calls.ts";

const HEADER: &str = "// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.
";

fn export<T: TS>(cfg: &Config, doc: &str) -> String {
    format!("\n/** {doc} */\nexport {}\n", T::decl(cfg))
}

/// The whole file, as the types say it should be.
pub fn links_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    out += &export::<GroupMembership>(&cfg, "What I am to a group, as far as this device knows.");
    out += &export::<LinkGroupKind>(&cfg, "Whether a group lets anyone in by its link.");
    out += &export::<LinkView>(&cfg, "What a link inside leads to, as the runtime sees it. The UI never takes a link apart itself.");
    out += &export::<Preview>(&cfg, "What a page outside says about itself.");
    out
}

/// What a chat has shared, as the panel about a chat reads it.
pub fn shared_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    out += &export::<SharedSection>(&cfg, "A section of what a chat has shared: pictures and videos, files, links, voice and round videos.");
    out += &export::<SharedCounts>(&cfg, "How many messages each section holds. Links are counted by message.");
    out
}

/// Which way the project's servers are reached, as the network panel reads it.
pub fn net_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    out += &export::<NetMode>(&cfg, "Whether the project's servers are reached through a bridge: never, always, or when the direct way fails.");
    out += &export::<BridgeView>(&cfg, "A bridge the user added.");
    out += &export::<NetStatus>(&cfg, "The way to the project's servers as it is now.");
    out += &export::<Verdict>(&cfg, "What trying both ways found.");
    out += &export::<NetCheck>(&cfg, "A check of the direct way and of a bridge.");
    out
}

/// A profile, its bio and links, a contact card, the avatar's crop and the
/// phones, as the profile editor and the cards read them.
pub fn profile_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    out += &export::<Color>(&cfg, "A color of the bio's palette; the UI maps each to a token readable in both themes.");
    out += &export::<Style>(&cfg, "How a piece of a bio is written.");
    out += &export::<Span>(&cfg, "A piece of a bio: text, a link (its text is its address) or a line break.");
    out += &export::<SocialLink>(&cfg, "A link to a profile elsewhere as it is stored and sent: a platform id and a handle.");
    out += &export::<SocialView>(&cfg, "A checked link to a profile elsewhere, as the UI shows it; `url` may be empty.");
    out += &export::<SocialPlatform>(&cfg, "A platform the user can pick for a link.");
    out += &export::<ProfileView>(&cfg, "A profile as the UI sees it.");
    out += &export::<ProfileInput>(&cfg, "What the user edits of their own profile; the avatar has commands of its own.");
    out += &export::<CardView>(&cfg, "A contact card as the UI shows it.");
    out += &export::<CropRect>(&cfg, "The part of a picked picture to keep, as fractions 0..1 of its preview.");
    out += &export::<AvatarPreview>(&cfg, "A picked picture, ready to be cropped.");
    out += &export::<OwnPrivateView>(&cfg, "My phone, which never goes into my public profile, and whether my card carries it by default.");
    out += &export::<ContactPrivateView>(&cfg, "The phone a contact sent me in its own card; `null` when none.");
    out
}

/// A transfer of a file, as the bubbles and the list of transfers read it.
pub fn transfer_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    out += &export::<TransferStage>(&cfg, "Where a transfer is: queued, preparing (a photo is made smaller), checking what an earlier attempt kept, uploading, publishing its message, downloading, assembling, verifying.");
    out += &export::<Progress>(&cfg, "The payload of the runtime event `transfer.progress` (`messenger://event`).");
    out += &export::<TransferView>(&cfg, "A transfer as `messenger_media_transfers` and `messenger_media_transfer` read it.");
    out
}

/// A call, as the call screen, the incoming-call screen and the settings
/// of calls read it.
pub fn calls_ts() -> String {
    let cfg = Config::new();
    let mut out = String::from(HEADER);
    out += &export::<CallMedia>(&cfg, "What a call carries.");
    out += &export::<CallDirection>(&cfg, "Who called whom.");
    out += &export::<CallPhase>(&cfg, "Where a call is: ringing on their side, ringing here, ICE looking for a way, talking, restoring a lost way while talking, over.");
    out += &export::<ReconnectReason>(&cfg, "Why a call is `reconnecting`: the way went, my network changed, or the peer lost the way.");
    out += &export::<CallVia>(&cfg, "How the media goes: directly between the two, or through a relay.");
    out += &export::<CallOutcome>(&cfg, "How a call ended; `answered_elsewhere` means another device of mine took it.");
    out += &export::<RelayPolicy>(&cfg, "Which way a call may take: `auto` (directly when it works), `relay_only` (my address stays behind a relay).");
    out += &export::<CallLimits>(&cfg, "What the nearest node allows, as it said.");
    out += &export::<VideoTrack>(&cfg, "Which video of a call: mine as the camera sees it, or the peer's.");
    out += &export::<VideoInput>(&cfg, "What I send as video, as `messenger_call_set_video` takes it: nothing, a camera (by id, or the default; `front`/`back` on a phone), a screen or window (by id, or the first screen; a computer only).");
    out += &export::<VideoQuality>(&cfg, "How big my video is sent, for the next time it goes on: 640×360 or 1280×720 at 30 fps.");
    out += &export::<VideoSize>(&cfg, "The size of the frames of one video as they show.");
    out += &export::<CameraInfo>(&cfg, "A camera the engine can open, as `messenger_call_list_cameras` lists them; `id` is what `VideoInput` and `messenger_call_switch_camera` take.");
    out += &export::<ScreenInfo>(&cfg, "A screen or a window that can be shared, as `messenger_call_list_screens` lists them.");
    out += &export::<CallView>(&cfg, "The call as the screen shows it: the `call` of every `call.*` event and the answer of the call commands.");
    out += &export::<CallIncoming>(&cfg, "The payload of the runtime event `call.incoming`; `busy_with_group` says I sit in the room of a group call as it rings: its `accept` is refused, the screen shows \"busy\" instead of \"answer\".");
    out += &export::<CallEnded>(&cfg, "The payload of the runtime event `call.ended`.");
    out += &export::<CallStats>(&cfg, "The `stats` of the runtime event `call.stats`, as the engine tells them.");
    out += &export::<CallNodeView>(&cfg, "A call node the client may use, as the settings list it.");
    out += &export::<CallNodeInput>(&cfg, "One own node, as `messenger_call_set_nodes` takes it.");
    out += &export::<CallState>(&cfg, "The call under way, the policy and the nodes, as `messenger_call_get_state` answers.");
    out += &export::<GroupCallPhase>(&cfg, "Where I am with the room of a group call: making it, joining it, in it, restoring the way to the node, out of it.");
    out += &export::<GroupParticipant>(&cfg, "One seat of the room of a group call; only a verified seat is a person and is heard. `video_mid` is what `messenger_group_call_video_subscribe` takes.");
    out += &export::<GroupCallView>(&cfg, "The room of a group call I am in: the `call` of `group_call.state` and the answer of the group call commands.");
    out += &export::<GroupCallAnnounced>(&cfg, "A call announced in a group, for the banner of its chat: the `call` of `group_call.started` and `group_call.ended`.");
    out += &export::<GroupCallEnded>(&cfg, "The payload of the runtime event `group_call.ended`.");
    out += &export::<GroupCallLevel>(&cfg, "The payload of the runtime event `group_call.level`: how loud one seat is.");
    out += &export::<GroupCallState>(&cfg, "The room I am in and the call announced in a group, as `messenger_group_call_get_state` answers.");
    out
}

/// Every generated file and what it should hold.
pub fn files() -> [(&'static str, String); 6] {
    [
        (LINKS_FILE, links_ts()),
        (SHARED_FILE, shared_ts()),
        (NET_FILE, net_ts()),
        (PROFILE_FILE, profile_ts()),
        (TRANSFER_FILE, transfer_ts()),
        (CALLS_FILE, calls_ts()),
    ]
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

    #[test]
    fn every_membership_the_store_writes_has_a_name_in_typescript() {
        use messenger_groups::service::*;
        for word in [
            MEMBERSHIP_JOINED, MEMBERSHIP_JOINING, MEMBERSHIP_REQUESTED, MEMBERSHIP_REJECTED, MEMBERSHIP_STALE,
            MEMBERSHIP_LEFT, MEMBERSHIP_REMOVED, MEMBERSHIP_BANNED, MEMBERSHIP_DISBANDED,
        ] {
            let m = GroupMembership::parse(word).unwrap_or_else(|| panic!("{word} is not known"));
            assert_eq!(serde_json::to_value(m).unwrap(), serde_json::json!(word), "the UI gets the same word");
        }
        assert!(GroupMembership::parse("member").is_none());
    }
}

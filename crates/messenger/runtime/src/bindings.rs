// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! TypeScript types of what the runtime hands to the UI, written from the
//! Rust types themselves so the two cannot drift apart.
//!
//! The files live with the UI (`ui/src/lib/messenger/generated/`). A test
//! compares them with what the types say now and fails when one is stale;
//! `make msg-types` (or `UPDATE_TS_BINDINGS=1 cargo test -p
//! messenger-runtime bindings`) writes them again.

use crate::links::{GroupMembership, LinkGroupKind, LinkView};
use crate::net::{BridgeView, NetCheck, NetMode, NetStatus, Verdict};
use crate::shared::{SharedCounts, SharedSection};
use messenger_preview::Preview;
use ts_rs::{Config, TS};

/// Relative to this crate.
pub const LINKS_FILE: &str = "../../../ui/src/lib/messenger/generated/links.ts";
pub const SHARED_FILE: &str = "../../../ui/src/lib/messenger/generated/shared.ts";
pub const NET_FILE: &str = "../../../ui/src/lib/messenger/generated/net.ts";

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

/// Every generated file and what it should hold.
pub fn files() -> [(&'static str, String); 3] {
    [(LINKS_FILE, links_ts()), (SHARED_FILE, shared_ts()), (NET_FILE, net_ts())]
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

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the app keeps its data on an iPhone: the container of its App
//! Group, so that the share extension («Поделиться» into Veydan from
//! another app, internal/messenger-wire.md §5) opens the same data file
//! and sends with the same key. The group is `group.<bundle identifier>`
//! (`group.net.veydan.mobile`, `group.net.veydan.mobile.dev` of a dev
//! build), named in the entitlements of the app and of the extension. An
//! app signed without the group (no entitlement) gets no container and
//! keeps its own folder, as before.
//!
//! The first start with a container moves the folder `data` of the app's
//! own container into the group's (one rename: both lie on the same
//! volume); a move that fails leaves the data where it was and the app
//! goes on from there, saying so on the terminal.

use std::path::{Path, PathBuf};

/// The folder of the group's container the app's data lives under: what
/// `app_data_dir` was inside the app's own container, so the paths below
/// it (`data/`, `data/messenger/`) stay the same.
const ROOT: &str = "veydan";

/// The group of this app: the bundle's own, never another's. A dev build
/// (`net.veydan.mobile.dev`) beside the TestFlight app must not find the
/// data of the other: one group each (scripts/ios/device-env.sh names the
/// dev build's in the entitlements).
fn group_ids() -> Vec<String> {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| vec![format!("group.{}", id)])
        .unwrap_or_default()
}

/// The container of `group`, when the app may use it.
fn container(group: &str) -> Option<PathBuf> {
    use objc2_foundation::{NSFileManager, NSString};
    let url = NSFileManager::defaultManager().containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(group))?;
    let path = url.path()?.to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The folder the app's data lives under: the group's, with the data moved
/// there from `own` (the app's `app_data_dir`) when it still lay there;
/// `own` itself when the app has no group.
pub(crate) fn data_root(own: PathBuf) -> PathBuf {
    for group in group_ids() {
        let Some(container) = container(&group) else { continue };
        let root = container.join(ROOT);
        return match migrate(&own, &root) {
            Ok(()) => root,
            Err(e) => {
                eprintln!("ios: the data stays in the app's own container, not the group's ({group}): {e}");
                own
            }
        };
    }
    own
}

/// `data` of `own` into `root` once: nothing when `root` has it already,
/// nothing when `own` never had it (a fresh install writes to the group
/// from the start).
fn migrate(own: &Path, root: &Path) -> std::io::Result<()> {
    let from = own.join(super::app::DATA_SUBDIR);
    let to = root.join(super::app::DATA_SUBDIR);
    if to.exists() || !from.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(root)?;
    std::fs::rename(&from, &to)?;
    eprintln!("ios: the data moved into the container of the app group");
    Ok(())
}

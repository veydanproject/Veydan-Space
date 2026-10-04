// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Shared on every platform
pub mod demo;

// Desktop only: browser profiles, proxies, SSH/SFTP, backups
#[cfg(desktop)]
pub mod backup;
#[cfg(desktop)]
pub mod camoufox;
#[cfg(desktop)]
pub mod fs;
#[cfg(desktop)]
pub mod profiles;
#[cfg(desktop)]
pub mod proxies;
#[cfg(desktop)]
pub mod sftp;
#[cfg(desktop)]
pub mod ssh;
#[cfg(desktop)]
pub mod ssh_keys;
#[cfg(desktop)]
pub mod transfer;
#[cfg(desktop)]
pub mod workspaces;

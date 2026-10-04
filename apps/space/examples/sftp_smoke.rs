// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Runtime smoke test for the russh + russh-sftp pairing used by
//! `commands::sftp` — same channel flow as `do_connect`, minus Tauri.
//!
//! Usage: cargo run --example sftp_smoke -- <port> <key_path> <list_dir>

use russh::client;
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg};
use russh_sftp::client::SftpSession;
use std::sync::Arc;

struct AcceptAll;

impl client::Handler for AcceptAll {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(true))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().expect("port").parse()?;
    let key_path = args.next().expect("key_path");
    let list_dir = args.next().expect("list_dir");
    let username = std::env::var("USER")?;

    let config = Arc::new(client::Config {
        keepalive_interval: Some(std::time::Duration::from_secs(30)),
        ..Default::default()
    });

    let mut handle = client::connect(config, ("127.0.0.1", port), AcceptAll).await?;

    let pem = std::fs::read_to_string(&key_path)?;
    let key = russh::keys::PrivateKey::from_openssh(pem.trim())
        .or_else(|_| decode_secret_key(pem.trim(), None))?;
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
    let auth = handle
        .authenticate_publickey(&username, key_with_alg)
        .await?;
    anyhow::ensure!(
        matches!(auth, client::AuthResult::Success),
        "auth failed: {auth:?}"
    );
    println!("AUTH OK as {username}");

    // Same flow as commands::sftp::do_connect
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = SftpSession::new(channel.into_stream()).await?;

    let home = sftp.canonicalize(".").await?;
    println!("HOME = {home}");

    let dir = sftp.read_dir(&list_dir).await?;
    for entry in dir {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let md = entry.metadata();
        println!(
            "{:>9} {:>8} dir={} sym={} uid={:?} mtime={:?}  {}",
            md.permissions
                .map(|p| format!("{:04o}", p & 0o7777))
                .unwrap_or_default(),
            md.size.unwrap_or(0),
            md.is_dir(),
            md.is_symlink(),
            md.uid,
            md.mtime,
            name,
        );
        // broken symlinks: stat may fail — must not panic
        if md.is_symlink() {
            let full = format!("{list_dir}/{name}");
            match sftp.metadata(&full).await {
                Ok(t) => println!("          -> target is_dir={}", t.is_dir()),
                Err(e) => println!("          -> broken link (stat failed: {e})"),
            }
        }
    }

    sftp.close().await?;
    println!("SMOKE OK");
    Ok(())
}

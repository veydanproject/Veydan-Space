// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Runtime smoke test for the SSH keys store (`commands::ssh_keys`):
//! generation (all algorithms), import round-trip incl. encrypted keys,
//! `resolve_key_material` against a real sqlite DB, and — when a local sshd
//! is reachable — public-key authentication with the generated keys.
//!
//! Usage: cargo run --example ssh_keys_smoke -- [port] [authorized_keys_path]
//!   Without args only offline checks run. With a port + authorized_keys path
//!   the generated public keys are appended there and real auth is attempted.

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use std::sync::Arc;
use veydan_lib::commands::ssh::{resolve_key_material, SshConnection};
use veydan_lib::commands::ssh_keys::{
    generate_key_material, parse_imported, KeyMaterial, ERR_KEY_ENCRYPTED, ERR_KEY_WRONG_PASSPHRASE,
};

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

fn check(cond: bool, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(cond, "FAILED: {label}");
    println!("  ok: {label}");
    Ok(())
}

async fn auth_with(
    port: u16,
    material: &KeyMaterial,
    passphrase: Option<&str>,
) -> anyhow::Result<()> {
    let username = std::env::var("USER")?;
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect(config, ("127.0.0.1", port), AcceptAll).await?;

    // Same parse + hash-alg selection as commands::ssh::do_authenticate.
    let key =
        parse_imported(&material.private_pem, passphrase).map_err(|e| anyhow::anyhow!("{e}"))?;
    let hash_alg = if key.algorithm().is_rsa() {
        Some(HashAlg::Sha256)
    } else {
        None
    };
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
    let auth = handle
        .authenticate_publickey(&username, key_with_alg)
        .await?;
    anyhow::ensure!(
        matches!(auth, client::AuthResult::Success),
        "auth failed for {}: {auth:?}",
        material.algorithm
    );
    println!("  ok: AUTH {} as {username}", material.algorithm);
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let port: Option<u16> = args.next().map(|p| p.parse()).transpose()?;
    let authorized_keys = args.next();

    // ── Generation ──────────────────────────────────────────────────────────
    println!("generate:");
    let ed = generate_key_material("ed25519", None, "smoke@ed25519".into(), None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    check(
        ed.algorithm == "ed25519" && ed.bits.is_none(),
        "ed25519 meta",
    )?;
    check(
        ed.public_openssh.starts_with("ssh-ed25519 "),
        "ed25519 public key format",
    )?;
    check(ed.fingerprint.starts_with("SHA256:"), "ed25519 fingerprint")?;
    check(
        ed.comment.as_deref() == Some("smoke@ed25519"),
        "ed25519 comment",
    )?;

    let rsa = generate_key_material("rsa", Some(2048), String::new(), Some("s3cret"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    check(
        rsa.algorithm == "rsa" && rsa.bits == Some(2048),
        "rsa 2048 meta",
    )?;
    check(
        rsa.public_openssh.starts_with("ssh-rsa "),
        "rsa public key format",
    )?;

    let ec = generate_key_material("ecdsa", Some(256), String::new(), None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    check(ec.algorithm == "ecdsa-p256", "ecdsa p256 meta")?;
    check(
        ec.public_openssh.starts_with("ecdsa-sha2-nistp256 "),
        "ecdsa public key format",
    )?;

    check(
        generate_key_material("rsa", Some(1024), String::new(), None).is_err(),
        "rsa 1024 rejected",
    )?;
    check(
        generate_key_material("dsa", None, String::new(), None).is_err(),
        "unknown algo rejected",
    )?;

    // ── Import round-trip ───────────────────────────────────────────────────
    println!("import:");
    let re = parse_imported(&ed.private_pem, None).map_err(|e| anyhow::anyhow!("{e}"))?;
    check(
        re.public_key().to_openssh()? == ed.public_openssh,
        "ed25519 re-import public key matches",
    )?;

    // Encrypted RSA: needs the passphrase, distinct errors otherwise.
    let err = parse_imported(&rsa.private_pem, None)
        .unwrap_err()
        .to_string();
    check(
        err.contains(ERR_KEY_ENCRYPTED),
        "encrypted key without passphrase → key_encrypted",
    )?;
    let err = parse_imported(&rsa.private_pem, Some("wrong"))
        .unwrap_err()
        .to_string();
    check(
        err.contains(ERR_KEY_WRONG_PASSPHRASE),
        "wrong passphrase → key_wrong_passphrase",
    )?;
    let re =
        parse_imported(&rsa.private_pem, Some("s3cret")).map_err(|e| anyhow::anyhow!("{e}"))?;
    check(
        re.public_key().to_openssh()? == rsa.public_openssh,
        "encrypted rsa re-import public key matches",
    )?;

    // ── resolve_key_material against a real sqlite DB ───────────────────────
    println!("resolver:");
    let db = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::query("CREATE TABLE ssh_keys (id TEXT PRIMARY KEY, private_key TEXT, passphrase TEXT)")
        .execute(&db)
        .await?;
    sqlx::query("CREATE TABLE ssh_connections (id TEXT PRIMARY KEY, ssh_key_id TEXT)")
        .execute(&db)
        .await?;
    sqlx::query("INSERT INTO ssh_keys (id, private_key, passphrase) VALUES ('k1', ?, 's3cret')")
        .bind(&rsa.private_pem)
        .execute(&db)
        .await?;

    let mut conn = SshConnection {
        id: "c1".into(),
        name: "smoke".into(),
        host: "127.0.0.1".into(),
        port: 22,
        username: "u".into(),
        auth_type: "key".into(), // deliberately wrong — resolver must normalize
        password: None,
        private_key: None,
        key_passphrase: None,
        has_password: false,
        has_private_key: false,
        has_key_passphrase: false,
        ssh_key_id: Some("k1".into()),
        requires_2fa: false,
        totp_entry_id: None,
        proxy_id: None,
        proxy_name: None,
        workspace_ids: vec![],
        profile_ids: vec![],
        connect_timeout_sec: 15,
        keepalive_sec: 30,
        terminal_theme: None,
        default_cols: 120,
        default_rows: 32,
        last_connected_at: None,
        server_fingerprint: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    resolve_key_material(&db, &mut conn)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    check(
        conn.private_key.as_deref() == Some(rsa.private_pem.as_str()),
        "resolver injects key material",
    )?;
    check(
        conn.key_passphrase.as_deref() == Some("s3cret"),
        "resolver injects passphrase",
    )?;
    check(
        conn.auth_type == "key_password",
        "resolver normalizes auth_type",
    )?;

    let mut dangling = conn.clone();
    dangling.ssh_key_id = Some("missing".into());
    dangling.auth_type = "key".into();
    check(
        resolve_key_material(&db, &mut dangling).await.is_err(),
        "dangling ssh_key_id → clean error",
    )?;

    let mut password_conn = conn.clone();
    password_conn.auth_type = "password".into();
    password_conn.private_key = None;
    resolve_key_material(&db, &mut password_conn)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    check(
        password_conn.private_key.is_none(),
        "password auth untouched by resolver",
    )?;

    // ── Live auth against a local sshd (optional) ───────────────────────────
    if let (Some(port), Some(ak_path)) = (port, authorized_keys) {
        println!("live auth on 127.0.0.1:{port}:");
        let mut ak = std::fs::read_to_string(&ak_path).unwrap_or_default();
        for m in [&ed, &rsa, &ec] {
            if !ak.contains(&m.public_openssh) {
                ak.push_str(&m.public_openssh);
                ak.push('\n');
            }
        }
        std::fs::write(&ak_path, ak)?;

        auth_with(port, &ed, None).await?;
        auth_with(port, &rsa, Some("s3cret")).await?;
        auth_with(port, &ec, None).await?;
    } else {
        println!("live auth skipped (no port/authorized_keys args)");
    }

    println!("ALL OK");
    Ok(())
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use russh::client;
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg, PublicKey, PublicKeyBase64};
use std::future::Future;
use std::sync::{Arc, Mutex};

pub enum SshAuth {
    Password(String),
    PrivateKey(String),
}

/// Computes SHA256 fingerprint of a public key, formatted as "SHA256:<base64>".
/// Also used by `commands::ssh::TerminalHandler` so terminal/SFTP pins use the
/// same format as proxy pins.
pub(crate) fn compute_fingerprint(key: &PublicKey) -> String {
    use sha2::{Digest, Sha256};
    let raw = key.public_key_bytes();
    let hash = Sha256::digest(&raw);
    format!(
        "SHA256:{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD_NO_PAD, hash)
    )
}

struct SshHandler {
    /// Fingerprint stored in DB for this proxy (None = first connection).
    known_fingerprint: Option<String>,
    /// Fingerprint received from the server during this handshake.
    received_fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for SshHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        let fp = compute_fingerprint(server_public_key);
        *self.received_fingerprint.lock().unwrap() = Some(fp.clone());

        let result = match &self.known_fingerprint {
            None => {
                // First connection — accept and let the caller save the fingerprint
                Ok(true)
            }
            Some(known) if known == &fp => {
                // Fingerprint matches — accept
                Ok(true)
            }
            Some(_) => {
                // Fingerprint mismatch — possible MITM, reject
                Err(russh::Error::WrongServerSig)
            }
        };

        std::future::ready(result)
    }
}

pub struct SshSession {
    handle: client::Handle<SshHandler>,
}

pub type SharedSession = Arc<SshSession>;

pub struct SshConnectResult {
    pub session: SharedSession,
    /// Fingerprint received from the server during this connection.
    pub fingerprint: String,
    /// True if this was the first connection (no known_fingerprint stored yet).
    pub is_new: bool,
}

impl SshSession {
    pub async fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: SshAuth,
        known_fingerprint: Option<String>,
    ) -> anyhow::Result<SshConnectResult> {
        const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

        let received_fingerprint: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let is_new = known_fingerprint.is_none();

        let config = Arc::new(client::Config {
            // Не даём NAT/файрволам молча убивать простаивающую сессию
            keepalive_interval: Some(std::time::Duration::from_secs(30)),
            ..client::Config::default()
        });
        let handler = SshHandler {
            known_fingerprint,
            received_fingerprint: Arc::clone(&received_fingerprint),
        };
        let mut handle = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            client::connect(config, (host, port), handler),
        )
        .await
        {
            Err(_) => anyhow::bail!("SSH connect to {host}:{port} timed out"),
            Ok(Err(russh::Error::WrongServerSig)) => anyhow::bail!(
                "SSH host key verification failed: the server key fingerprint does not match \
                     the one stored for this proxy. The server key may have changed, or the \
                     connection may be intercepted (possible MITM). If you trust the new key, \
                     re-confirm the fingerprint for this proxy."
            ),
            Ok(Err(e)) => return Err(e.into()),
            Ok(Ok(h)) => h,
        };

        let result = tokio::time::timeout(CONNECT_TIMEOUT, async {
            match auth {
                SshAuth::Password(pass) => {
                    anyhow::Ok(handle.authenticate_password(username, pass).await?)
                }
                SshAuth::PrivateKey(pem) => {
                    let key = decode_secret_key(&pem, None)?;
                    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
                    anyhow::Ok(
                        handle
                            .authenticate_publickey(username, key_with_alg)
                            .await?,
                    )
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("SSH authentication to {host}:{port} timed out"))??;

        anyhow::ensure!(result.success(), "SSH authentication failed");

        let fingerprint = received_fingerprint
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();

        Ok(SshConnectResult {
            session: Arc::new(SshSession { handle }),
            fingerprint,
            is_new,
        })
    }

    pub async fn open_channel(
        &self,
        target_host: &str,
        target_port: u16,
    ) -> anyhow::Result<russh::Channel<client::Msg>> {
        Ok(self
            .handle
            .channel_open_direct_tcpip(target_host, target_port as u32, "127.0.0.1", 0u32)
            .await?)
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Push notifications: keeping a push server told what to watch.
//!
//! The host brings the device's address at the push service (the token);
//! everything else is decided here. The server is told the whole state of
//! the device every time, and only when the state changed or the last
//! telling is a week old.
//!
//! What is wanted is computed from what the messenger knows now: the relays
//! that are read, the groups that are joined. There is no list of "things to
//! tell the server about" to keep in step with the rest: [`push_reconcile`]
//! compares, and tells when there is a difference. The host calls it on a
//! timer ([`MessengerRuntime::push_loop`]) and after what it knows changed.
//!
//! Nothing is said to the server before the user agreed (`push.enabled`),
//! and nothing while the silent mode is on.
//!
//! [`push_reconcile`]: MessengerRuntime::push_reconcile

use crate::{MessengerRuntime, REGION_FALLBACK};
use messenger_core::traits::SystemClock;
use messenger_core::{Clock, MessengerError, Result};
use messenger_push::client::{server_address, PushError};
use messenger_push::{
    Channel, DeviceAnswer, DevicePut, GroupWatch, Prefs, RelayAnswer, RelayWatch, TestAnswer,
    VpushClient,
};
use messenger_store::settings;
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

const KEY_ENABLED: &str = "push.enabled";
const KEY_OFFERED: &str = "push.offered";
const KEY_SERVER: &str = "push.server_url";
const KEY_CHANNEL: &str = "push.channel";
const KEY_PREF_DM: &str = "push.prefs.dm";
const KEY_PREF_GROUPS: &str = "push.prefs.groups";
/// What was told last, and to whom: see [`Told`].
const KEY_TOLD: &str = "push.told";
const KEY_LAST_ERROR: &str = "push.last_error";
const KEY_LAST_TRY: &str = "push.last_try_at";
const KEY_DEVICE_PREFIX: &str = "push.device_id.";

/// The server is told again after this long, changed or not: a registration
/// nobody renews is forgotten by the server.
const RENEW_AFTER: u64 = 7 * 86_400;
/// After a failure, the next attempt is not sooner than this. A success
/// sets no wait: what changes next is told at once.
const RETRY_AFTER: u64 = 60;
/// How often the host's loop compares.
const LOOP_EVERY: Duration = Duration::from_secs(20);

/// The device's address at the push service, as the host brings it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushChannel {
    /// `fcm`.
    pub provider: String,
    pub token: String,
    /// The app as the push server knows it: the Android package.
    pub app_id: String,
    pub app_version: Option<String>,
}

/// What was told to a server, to tell a change from no change, and to know
/// whom to tell when the user turns pushes off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Told {
    server: String,
    pubkey: String,
    device_id: String,
    fingerprint: String,
    at: u64,
    answer: DeviceAnswer,
}

/// Where pushes stand, for the settings screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushStatus {
    /// The user agreed to pushes.
    pub enabled: bool,
    /// The user was asked, whatever the answer.
    pub offered: bool,
    /// The server in use, if there is one to use.
    pub server: Option<String>,
    /// The server was named by the user, not by the manifest.
    pub server_custom: bool,
    pub dm: bool,
    pub groups: bool,
    /// `off`: the user did not agree.
    /// `paused`: the silent mode is on.
    /// `waiting_unlock`: no key to sign with until the vault is opened.
    /// `no_channel`: the phone gave no address at the push service yet.
    /// `no_server`: no server is named.
    /// `pending`: not told yet, or told something older.
    /// `registered`: the server knows what is wanted.
    /// `failed`: the last telling failed; `error` says why.
    pub state: String,
    /// Unix seconds.
    pub last_ok_at: Option<u64>,
    pub expires_at: Option<u64>,
    /// `push_unreachable: …`, `push_refused_<code>: … (request <id>)`.
    pub error: Option<String>,
    /// What the server said about each relay.
    pub relays: Vec<RelayAnswer>,
}

fn now() -> u64 {
    SystemClock.now().secs().max(0) as u64
}

fn random_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(hex::encode(bytes))
}

impl MessengerRuntime {
    async fn push_setting(&self, key: &str) -> Result<Option<String>> {
        settings::get(&self.store, key).await
    }

    async fn push_told(&self) -> Result<Option<Told>> {
        Ok(self
            .push_setting(KEY_TOLD)
            .await?
            .and_then(|json| serde_json::from_str(&json).ok()))
    }

    async fn push_channel(&self) -> Result<Option<PushChannel>> {
        Ok(self
            .push_setting(KEY_CHANNEL)
            .await?
            .and_then(|json| serde_json::from_str(&json).ok()))
    }

    async fn push_prefs(&self) -> Result<Prefs> {
        Ok(Prefs {
            dm: settings::get_bool(&self.store, KEY_PREF_DM, true).await?,
            groups: settings::get_bool(&self.store, KEY_PREF_GROUPS, true).await?,
        })
    }

    /// The server to use: the user's, or else, with the Veydan servers
    /// chosen, the manifest's for the region.
    async fn push_server(&self) -> Result<(Option<String>, bool)> {
        if let Some(custom) = self.push_setting(KEY_SERVER).await? {
            return Ok((Some(custom), true));
        }
        if self.relays.servers_mode().await? != Some(crate::relays::ServersMode::Veydan) {
            return Ok((None, false));
        }
        let (manifest, _) = self.relays.current_manifest().await?;
        let region = self.relays.region().await.unwrap_or_else(|_| REGION_FALLBACK.into());
        let server = manifest
            .push_for_region(&region)
            .first()
            .and_then(|p| server_address(&p.url).ok());
        Ok((server, false))
    }

    /// One id per identity on this device, made when first needed.
    async fn push_device_id(&self, pubkey: &str) -> Result<String> {
        let key = format!("{KEY_DEVICE_PREFIX}{pubkey}");
        if let Some(id) = self.push_setting(&key).await? {
            return Ok(id);
        }
        let id = random_id()?;
        settings::set(&self.store, &key, &id).await?;
        Ok(id)
    }

    /// What the server should know about this device now.
    async fn push_wanted(&self, keys: &Keys, channel: &PushChannel) -> Result<DevicePut> {
        let relays = self
            .relays
            .list()
            .await?
            .into_iter()
            // The relays messages are read from: the same set the user's
            // relay list (kind 10050) names to the world.
            .filter(|r| r.enabled && r.read)
            .map(|r| RelayWatch { url: r.url, dm: true, groups: true })
            .collect();

        let me = messenger_core::PubKey::parse(&keys.public_key().to_hex())
            .ok_or_else(|| MessengerError::Crypto("the key has no hex form".into()))?;
        // A group is named with the keys of its marks, by which the server
        // tells its events from those of strangers. One this device holds
        // no key of yet is left out: the server could tell nothing about
        // its events, and this device could open none of them.
        let mut groups = Vec::new();
        for group in self.groups().list(&me).await?.into_iter().filter(|g| g.membership == "joined") {
            let keys = self.groups().push_keys(&group.id).await?;
            if !keys.is_empty() {
                groups.push(GroupWatch { id: group.id, keys });
            }
        }
        groups.sort_by(|a, b| a.id.cmp(&b.id));

        let channel_dto = match channel.provider.as_str() {
            "fcm" => Channel::Fcm { token: channel.token.clone() },
            other => {
                return Err(MessengerError::Invalid(format!(
                    "push_provider_unknown: {other}"
                )))
            }
        };
        Ok(DevicePut {
            app_id: channel.app_id.clone(),
            channel: channel_dto,
            app_version: channel.app_version.clone(),
            prefs: self.push_prefs().await?,
            author_key: Some(messenger_dm::pushtags::author_key(keys)),
            relays,
            groups,
        })
    }

    fn push_fingerprint(server: &str, pubkey: &str, device_id: &str, wanted: &DevicePut) -> String {
        let mut hash = Sha256::new();
        for part in [server, pubkey, device_id] {
            hash.update(part.as_bytes());
            hash.update([0]);
        }
        hash.update(serde_json::to_vec(wanted).unwrap_or_default());
        hex::encode(hash.finalize())
    }

    async fn push_note_failure(&self, error: &str) -> Result<()> {
        // A server that cannot be reached may be one that a bridge reaches.
        if error.contains("push_unreachable") {
            if let Err(e) = self.net_trouble().await {
                eprintln!("messenger net: check after a push failure: {e}");
            }
        }
        settings::set(&self.store, KEY_LAST_ERROR, error).await?;
        settings::set(&self.store, KEY_LAST_TRY, &now().to_string()).await
    }

    /// Compares what the server was told with what is wanted now, and tells
    /// it when they differ. `force` tells it anyway, and at once: for the
    /// moment the user presses a button.
    ///
    /// A failure to reach the server is not an error of this call: it is
    /// kept, and shown in the status.
    pub async fn push_reconcile(&self, force: bool) -> Result<PushStatus> {
        if !settings::get_bool(&self.store, KEY_ENABLED, false).await?
            || self.relays.is_silent().await?
        {
            return self.push_status().await;
        }
        let (Ok(keys), Some(channel), (Some(server), _)) = (
            self.session_keys().await,
            self.push_channel().await?,
            self.push_server().await?,
        ) else {
            return self.push_status().await;
        };

        let pubkey = keys.public_key().to_hex();
        let device_id = self.push_device_id(&pubkey).await?;
        let wanted = self.push_wanted(&keys, &channel).await?;
        let fingerprint = Self::push_fingerprint(&server, &pubkey, &device_id, &wanted);
        let told = self.push_told().await?;

        if !force {
            let fresh = told.as_ref().is_some_and(|t| {
                t.fingerprint == fingerprint && now().saturating_sub(t.at) < RENEW_AFTER
            });
            if fresh {
                return self.push_status().await;
            }
            let last_try: u64 = self
                .push_setting(KEY_LAST_TRY)
                .await?
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if now().saturating_sub(last_try) < RETRY_AFTER {
                return self.push_status().await;
            }
        }

        // What was registered elsewhere, or as somebody else, is taken back
        // first: the old server must not go on watching.
        if let Some(old) = &told {
            if old.server != server || old.pubkey != pubkey {
                self.push_forget(old, &keys).await;
            }
        }

        let sent = async {
            VpushClient::new(&server)?
                .put_device(&keys, &device_id, &wanted)
                .await
        }
        .await;
        match sent {
            Ok(answer) => {
                let told = Told { server, pubkey, device_id, fingerprint, at: now(), answer };
                settings::set(&self.store, KEY_TOLD, &serde_json::to_string(&told)?).await?;
                settings::delete(&self.store, KEY_LAST_ERROR).await?;
                // The wait between attempts is for failures.
                settings::delete(&self.store, KEY_LAST_TRY).await?;
            }
            Err(e) => self.push_note_failure(&e.to_string()).await?,
        }
        self.push_status().await
    }

    /// Tells a server to forget the device. It can be told only by the key
    /// the device was registered with; a registration made as somebody else
    /// is left to run out.
    async fn push_forget(&self, told: &Told, keys: &Keys) {
        if told.pubkey != keys.public_key().to_hex() {
            return;
        }
        let done = async {
            VpushClient::new(&told.server)?
                .delete_device(keys, &told.device_id)
                .await
        }
        .await;
        if let Err(e) = done {
            eprintln!("messenger push: the server was not told to forget the device: {e}");
        }
    }

    /// Takes the registration back. Called when the user turns pushes off,
    /// and by the host before an identity is deleted: afterwards there is no
    /// key to sign the request with.
    pub async fn push_unregister(&self) -> Result<()> {
        if let (Some(told), Ok(keys)) = (self.push_told().await?, self.session_keys().await) {
            self.push_forget(&told, &keys).await;
        }
        settings::delete(&self.store, KEY_TOLD).await?;
        settings::delete(&self.store, KEY_LAST_ERROR).await?;
        settings::delete(&self.store, KEY_LAST_TRY).await
    }

    /// The user leaves the project's servers: a push server the manifest
    /// gave is told to forget this device. One the user named stays.
    pub(crate) async fn push_leave_manifest_server(&self) -> Result<()> {
        if self.push_setting(KEY_SERVER).await?.is_some() {
            return Ok(());
        }
        self.push_unregister().await
    }

    pub async fn push_status(&self) -> Result<PushStatus> {
        let enabled = settings::get_bool(&self.store, KEY_ENABLED, false).await?;
        let (server, server_custom) = self.push_server().await?;
        let prefs = self.push_prefs().await?;
        let told = self.push_told().await?;
        let error = self.push_setting(KEY_LAST_ERROR).await?;
        let keys = self.session_keys().await.ok();

        // What was told counts only while it is about this server and this key.
        let current = told.filter(|t| {
            Some(&t.server) == server.as_ref()
                && keys.as_ref().is_none_or(|k| k.public_key().to_hex() == t.pubkey)
        });

        let state = if !enabled {
            "off"
        } else if self.relays.is_silent().await? {
            "paused"
        } else if keys.is_none() {
            "waiting_unlock"
        } else if self.push_channel().await?.is_none() {
            "no_channel"
        } else if server.is_none() {
            "no_server"
        } else if error.is_some() {
            "failed"
        } else if current.is_some() {
            "registered"
        } else {
            "pending"
        };

        Ok(PushStatus {
            enabled,
            offered: settings::get_bool(&self.store, KEY_OFFERED, false).await?,
            server,
            server_custom,
            dm: prefs.dm,
            groups: prefs.groups,
            state: state.to_string(),
            last_ok_at: current.as_ref().map(|t| t.at),
            expires_at: current.as_ref().map(|t| t.answer.expires_at),
            error: if enabled { error } else { None },
            relays: current.map(|t| t.answer.relays).unwrap_or_default(),
        })
    }

    /// The user answered the question about pushes, one way or the other.
    pub async fn push_mark_offered(&self) -> Result<()> {
        settings::set_bool(&self.store, KEY_OFFERED, true).await
    }

    /// The address the phone has at the push service, or `None` when it has
    /// none any more. Nothing is sent by this call.
    pub async fn push_set_channel(&self, channel: Option<PushChannel>) -> Result<()> {
        match channel {
            Some(c) => settings::set(&self.store, KEY_CHANNEL, &serde_json::to_string(&c)?).await,
            None => settings::delete(&self.store, KEY_CHANNEL).await,
        }
    }

    pub async fn push_set_enabled(&self, enabled: bool) -> Result<PushStatus> {
        settings::set_bool(&self.store, KEY_OFFERED, true).await?;
        if enabled {
            settings::set_bool(&self.store, KEY_ENABLED, true).await?;
            return self.push_reconcile(true).await;
        }
        // Told to forget first, while pushes still count as on: the flag is
        // what the user sees, and it must not say "off" with a server still
        // watching.
        self.push_unregister().await?;
        settings::set_bool(&self.store, KEY_ENABLED, false).await?;
        self.push_status().await
    }

    /// `None` goes back to the server of the manifest.
    pub async fn push_set_server(&self, url: Option<String>) -> Result<PushStatus> {
        match url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            Some(url) => {
                let url = server_address(url).map_err(MessengerError::from)?;
                settings::set(&self.store, KEY_SERVER, &url).await?;
            }
            None => settings::delete(&self.store, KEY_SERVER).await?,
        }
        self.push_reconcile(true).await
    }

    pub async fn push_set_prefs(&self, dm: bool, groups: bool) -> Result<PushStatus> {
        settings::set_bool(&self.store, KEY_PREF_DM, dm).await?;
        settings::set_bool(&self.store, KEY_PREF_GROUPS, groups).await?;
        self.push_reconcile(true).await
    }

    /// Asks the server for a test push to this device.
    pub async fn push_test(&self) -> Result<TestAnswer> {
        let keys = self.session_keys().await?;
        let told = self
            .push_told()
            .await?
            .filter(|t| t.pubkey == keys.public_key().to_hex())
            .ok_or_else(|| MessengerError::Invalid("push_not_registered".into()))?;
        let answer = async {
            VpushClient::new(&told.server)?
                .test(&keys, &told.device_id)
                .await
        }
        .await
        .map_err(|e: PushError| MessengerError::from(e))?;
        Ok(answer)
    }

    /// Compares for as long as the runtime lives. The host starts it once.
    pub async fn push_loop(self: Arc<Self>) {
        loop {
            if let Err(e) = self.push_reconcile(false).await {
                eprintln!("messenger push: {e}");
            }
            tokio::time::sleep(LOOP_EVERY).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::MessengerConfig;
    use messenger_testkit::MemorySecretStore;
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A relay nothing listens on: it is in the list, and costs nothing.
    const RELAY: &str = "ws://127.0.0.1:9";

    struct Setup {
        rt: MessengerRuntime,
        secrets: Arc<MemorySecretStore>,
        server: MockServer,
        _dir: tempfile::TempDir,
    }

    async fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start(cfg, secrets.clone()).await.unwrap();

        // Off the network: the relays of the manifest are turned off, and
        // the one left is not there.
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        for relay in rt.relays().list().await.unwrap() {
            rt.relays().set_enabled(&relay.url, false).await.unwrap();
        }
        rt.relays().add_user(RELAY, None).await.unwrap();
        rt.relays().set_silent(false).await.unwrap();

        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();

        let server = MockServer::start().await;
        Setup { rt, secrets, server, _dir: dir }
    }

    fn channel() -> PushChannel {
        PushChannel {
            provider: "fcm".into(),
            token: "token-of-the-phone".into(),
            app_id: "net.veydan.mobile".into(),
            app_version: Some("4.0.1".into()),
        }
    }

    async fn server_accepts(server: &MockServer) {
        Mock::given(method("PUT"))
            .and(path_regex("^/v1/devices/[0-9a-f]{32}$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_id": "x", "expires_at": 1_800_000_000u64,
                "relays": [{ "url": RELAY, "status": "not_allowed", "detail": "no" }]
            })))
            .mount(server)
            .await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(204))
            .mount(server)
            .await;
    }

    async fn asked(server: &MockServer, verb: &str) -> Vec<wiremock::Request> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == verb)
            .collect()
    }

    /// Pushes on, with a phone and a server of the test.
    async fn registered() -> Setup {
        let s = setup().await;
        server_accepts(&s.server).await;
        s.rt.push_set_channel(Some(channel())).await.unwrap();
        s.rt.push_set_server(Some(s.server.uri())).await.unwrap();
        let status = s.rt.push_set_enabled(true).await.unwrap();
        assert_eq!(status.state, "registered", "{status:?}");
        s
    }

    #[tokio::test]
    async fn nothing_is_said_before_the_user_agreed() {
        let s = setup().await;
        server_accepts(&s.server).await;
        s.rt.push_set_channel(Some(channel())).await.unwrap();
        s.rt.push_set_server(Some(s.server.uri())).await.unwrap();

        let status = s.rt.push_reconcile(true).await.unwrap();
        assert_eq!(status.state, "off");
        assert!(!status.enabled && !status.offered);
        assert!(s.server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_server_of_the_manifest_is_used_unless_the_user_names_one() {
        let s = setup().await;
        let manifest = messenger_transport::Manifest::parse_content(messenger_transport::EMBEDDED_MANIFEST_JSON).unwrap();
        let of_manifest = manifest.push_for_region("default")[0].url.clone();
        let status = s.rt.push_status().await.unwrap();
        assert_eq!(status.server.as_deref(), Some(of_manifest.as_str()));
        assert!(!status.server_custom);

        let status = s.rt.push_set_server(Some(format!("{}/", s.server.uri()))).await.unwrap();
        assert_eq!(status.server, Some(s.server.uri()));
        assert!(status.server_custom);

        let status = s.rt.push_set_server(None).await.unwrap();
        assert_eq!(status.server.as_deref(), Some(of_manifest.as_str()));

        let e = s.rt.push_set_server(Some("http://elsewhere.example.org".into())).await;
        assert!(matches!(e, Err(MessengerError::Invalid(_))), "{e:?}");
    }

    #[tokio::test]
    async fn the_server_is_told_what_to_watch() {
        let s = registered().await;
        let keys = s.rt.session_keys().await.unwrap();

        let puts = asked(&s.server, "PUT").await;
        assert_eq!(puts.len(), 1);
        let body: Value = serde_json::from_slice(&puts[0].body).unwrap();
        assert_eq!(
            body,
            json!({
                "app_id": "net.veydan.mobile",
                "channel": { "provider": "fcm", "token": "token-of-the-phone" },
                "app_version": "4.0.1",
                "prefs": { "dm": true, "groups": true },
                "author_key": messenger_dm::pushtags::author_key(&keys),
                "relays": [{ "url": RELAY, "dm": true, "groups": true }],
                "groups": [],
            })
        );
        // Nothing of the user's key, and no key of a relay.
        let text = String::from_utf8_lossy(&puts[0].body).to_string();
        assert!(!text.contains(&keys.secret_key().to_secret_hex()));

        let status = s.rt.push_status().await.unwrap();
        assert_eq!(status.expires_at, Some(1_800_000_000));
        assert_eq!(status.relays[0].url, RELAY);
        assert!(!status.relays[0].status.watched());
        assert!(status.last_ok_at.is_some());
        assert!(status.offered);
    }

    #[tokio::test]
    async fn the_server_is_told_again_only_when_something_changed() {
        let s = registered().await;
        for _ in 0..3 {
            s.rt.push_reconcile(false).await.unwrap();
        }
        assert_eq!(asked(&s.server, "PUT").await.len(), 1);

        s.rt.push_set_prefs(true, false).await.unwrap();
        let puts = asked(&s.server, "PUT").await;
        assert_eq!(puts.len(), 2);
        let body: Value = serde_json::from_slice(&puts[1].body).unwrap();
        assert_eq!(body["prefs"], json!({ "dm": true, "groups": false }));

        // A new address at the push service, as after a reinstall.
        let mut renewed = channel();
        renewed.token = "a-new-token".into();
        s.rt.push_set_channel(Some(renewed)).await.unwrap();
        s.rt.push_reconcile(false).await.unwrap();
        assert_eq!(asked(&s.server, "PUT").await.len(), 3);

        // A relay turned off is a relay not to watch.
        s.rt.relays().set_enabled(RELAY, false).await.unwrap();
        s.rt.push_reconcile(false).await.unwrap();
        let puts = asked(&s.server, "PUT").await;
        assert_eq!(puts.len(), 4);
        let body: Value = serde_json::from_slice(&puts[3].body).unwrap();
        assert_eq!(body["relays"], json!([]));
    }

    #[tokio::test]
    async fn a_group_is_named_with_the_keys_of_its_marks() {
        use messenger_groups::{GroupKind, OpBody};

        let s = registered().await;
        let group = s.rt.group_create(GroupKind::Private, "Team", "", true).await.unwrap();
        s.rt.push_reconcile(false).await.unwrap();
        let puts = asked(&s.server, "PUT").await;
        assert_eq!(puts.len(), 2, "a group joined is a reason to tell the server");
        let body: Value = serde_json::from_slice(&puts[1].body).unwrap();
        let first = s.rt.groups().push_keys(&group.id).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(body["groups"], json!([{ "id": group.id, "keys": first }]));
        // The key of the marks is made from the group's key, and is not it.
        let text = String::from_utf8_lossy(&puts[1].body).to_string();
        for (_, key) in s.rt.groups().export_keys().await.unwrap() {
            assert!(!text.contains(&hex::encode(key.as_bytes())));
        }

        // The group's key changes: the server is told the new one, and
        // for a while the one before, for what is sent by those who have
        // not heard of the change.
        s.rt.group_act(&group.id, OpBody::RotateKey).await.unwrap();
        s.rt.push_reconcile(false).await.unwrap();
        let puts = asked(&s.server, "PUT").await;
        assert_eq!(puts.len(), 3);
        let body: Value = serde_json::from_slice(&puts[2].body).unwrap();
        let keys = body["groups"][0]["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], json!(first[0]), "the new key first");
        assert_eq!(keys[1], json!(first[0]));
    }

    #[tokio::test]
    async fn the_device_keeps_its_id() {
        let s = registered().await;
        s.rt.push_set_prefs(false, true).await.unwrap();
        let puts = asked(&s.server, "PUT").await;
        assert_eq!(puts[0].url.path(), puts[1].url.path());
    }

    #[tokio::test]
    async fn what_is_missing_is_named() {
        let s = setup().await;
        server_accepts(&s.server).await;
        s.rt.push_set_server(Some(s.server.uri())).await.unwrap();

        let status = s.rt.push_set_enabled(true).await.unwrap();
        assert_eq!(status.state, "no_channel");

        s.rt.push_set_channel(Some(channel())).await.unwrap();
        s.secrets.set_unlocked(false);
        s.rt.refresh_signer().await.unwrap();
        assert_eq!(s.rt.push_reconcile(true).await.unwrap().state, "waiting_unlock");
        assert!(asked(&s.server, "PUT").await.is_empty());

        s.secrets.set_unlocked(true);
        s.rt.refresh_signer().await.unwrap();
        assert_eq!(s.rt.push_reconcile(false).await.unwrap().state, "registered");
    }

    #[tokio::test]
    async fn the_silent_mode_silences_pushes_too() {
        let s = registered().await;
        s.rt.relays().set_silent(true).await.unwrap();
        // Something changed that the server would be told about.
        settings::set_bool(&s.rt.store, KEY_PREF_GROUPS, false).await.unwrap();

        let status = s.rt.push_reconcile(true).await.unwrap();
        assert_eq!(status.state, "paused");
        assert_eq!(asked(&s.server, "PUT").await.len(), 1, "nothing new was said");
    }

    #[tokio::test]
    async fn a_refusal_is_kept_and_shown_and_not_repeated_at_once() {
        let s = setup().await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(422).set_body_json(json!({
                "error": { "code": "unknown_app", "message": "not served" },
                "request_id": "5f3a9c1e"
            })))
            .mount(&s.server)
            .await;
        s.rt.push_set_channel(Some(channel())).await.unwrap();
        s.rt.push_set_server(Some(s.server.uri())).await.unwrap();

        let status = s.rt.push_set_enabled(true).await.unwrap();
        assert_eq!(status.state, "failed");
        assert_eq!(
            status.error.as_deref(),
            Some("push_refused_unknown_app: not served (request 5f3a9c1e)")
        );
        assert!(status.enabled, "the user's wish stands");
        assert_eq!(asked(&s.server, "PUT").await.len(), 1);

        for _ in 0..3 {
            s.rt.push_reconcile(false).await.unwrap();
        }
        assert_eq!(asked(&s.server, "PUT").await.len(), 1, "not sooner than in a minute");

        s.rt.push_reconcile(true).await.unwrap();
        assert_eq!(asked(&s.server, "PUT").await.len(), 2, "the button asks at once");
    }

    #[tokio::test]
    async fn a_server_that_is_not_there() {
        let s = setup().await;
        s.rt.push_set_channel(Some(channel())).await.unwrap();
        s.rt.push_set_server(Some("http://127.0.0.1:1".into())).await.unwrap();
        let status = s.rt.push_set_enabled(true).await.unwrap();
        assert_eq!(status.state, "failed");
        assert!(status.error.unwrap().starts_with("push_unreachable"));
    }

    #[tokio::test]
    async fn turning_pushes_off_takes_the_registration_back() {
        let s = registered().await;
        let put = &asked(&s.server, "PUT").await[0];

        let status = s.rt.push_set_enabled(false).await.unwrap();
        assert_eq!(status.state, "off");
        assert!(status.relays.is_empty() && status.error.is_none());

        let deletes = asked(&s.server, "DELETE").await;
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].url.path(), put.url.path());

        s.rt.push_reconcile(true).await.unwrap();
        assert_eq!(asked(&s.server, "PUT").await.len(), 1, "off is off");
    }

    #[tokio::test]
    async fn another_server_is_told_and_the_former_one_is_told_to_forget() {
        let s = registered().await;
        let other = MockServer::start().await;
        server_accepts(&other).await;

        let status = s.rt.push_set_server(Some(other.uri())).await.unwrap();
        assert_eq!(status.state, "registered");
        assert_eq!(asked(&s.server, "DELETE").await.len(), 1);
        assert_eq!(asked(&other, "PUT").await.len(), 1);
    }

    #[tokio::test]
    async fn a_test_push_is_asked_of_the_server_the_device_is_registered_at() {
        let s = registered().await;
        Mock::given(method("POST"))
            .and(path_regex("^/v1/devices/[0-9a-f]{32}/test$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "outcome": "delivered", "trace": "9583cb58"
            })))
            .mount(&s.server)
            .await;
        let answer = s.rt.push_test().await.unwrap();
        assert_eq!(answer.outcome, "delivered");

        s.rt.push_set_enabled(false).await.unwrap();
        let e = s.rt.push_test().await;
        assert!(matches!(e, Err(MessengerError::Invalid(ref c)) if c == "push_not_registered"), "{e:?}");
    }
}

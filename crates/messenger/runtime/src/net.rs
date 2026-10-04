// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whether the project's servers are reached directly or through a bridge.
//!
//! The user chooses one of three:
//!
//! - **off**: directly. When that stops working and a bridge would work,
//!   the app says so and offers to switch;
//! - **on**: through a bridge, always;
//! - **auto**: directly while that works, through a bridge when it does not.
//!
//! Only the project's servers are ever reached through a bridge, and only
//! with the project's servers chosen: a hub carries nothing else.
//!
//! What "does not work" means is found by trying, not by guessing at the
//! kind of trouble: a few hundred kilobytes are pushed the direct way and
//! through a bridge (see [`decide`]). A way that passes small requests and
//! stalls on large ones is the common case, and only moving bytes shows it.

use crate::relays::{RelayService, ServersMode};
use messenger_core::traits::SystemClock;
use messenger_core::{Clock, MessengerError, Result};
use messenger_store::{settings, Store};
use messenger_transport::manifest::ManifestSource;
use messenger_transport::Manifest;
use messenger_vlink::proto::list::SignedList;
use messenger_vlink::{probe, trust, BridgeRef, Net, NetConfig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use ts_rs::TS;

pub const KEY_MODE: &str = "net.mode";
/// The last list the registry gave, as it came: signed, checked again
/// every time it is read.
const KEY_LIST: &str = "net.list";
const KEY_LIST_CHECKED: &str = "net.list.checked_at";
/// Bridges the user added, as references.
const KEY_PRIVATE: &str = "net.private";
/// The bridge that carried last: tried first next time.
const KEY_LAST_GOOD: &str = "net.last_good";
/// In `auto`: until when the bridge is used before the direct way is
/// tried again.
const KEY_AUTO_UNTIL: &str = "net.auto_until";
/// In `off`: a bridge would help, and the user has not answered yet.
const KEY_OFFER: &str = "net.offer";
const KEY_OFFER_DISMISSED: &str = "net.offer.dismissed_at";
const KEY_CHECKED: &str = "net.checked_at";

/// What a check moves each way: past the size at which a throttled way stalls.
const CHECK_BYTES: usize = 256 * 1024;
const DIRECT_PATIENCE: Duration = Duration::from_secs(8);
const BRIDGE_PATIENCE: Duration = Duration::from_secs(15);
/// A list older than this is asked for again when the app starts.
const LIST_REFRESH_SECS: i64 = 6 * 60 * 60;
/// `auto` goes back to trying the direct way after this long.
const AUTO_HOLD_SECS: i64 = 24 * 60 * 60;
/// "Not now" is respected for this long.
const OFFER_QUIET_SECS: i64 = 7 * 24 * 60 * 60;
/// Checks cost bytes; signs of trouble come in bursts.
const CHECK_EVERY_SECS: i64 = 10 * 60;
/// With nothing wrong in sight the direct way is still tried once a day: a
/// way that passes small requests and stalls on large ones shows no sign
/// until a file is sent.
const CHECK_QUIET_SECS: i64 = 24 * 60 * 60;
/// No relay connected for this long, with a session running, is trouble.
const RELAYS_PATIENCE: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NetMode {
    Off,
    On,
    Auto,
}

impl NetMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Auto => "auto",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// A bridge the user added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BridgeView {
    pub id: String,
    pub addr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct NetStatus {
    pub mode: NetMode,
    /// The project's servers are reached through a bridge right now.
    pub active: bool,
    /// The address of the bridge in use, while a connection to one is up.
    pub bridge: Option<String>,
    /// Bridges known: added, listed, built in.
    pub bridges: u32,
    pub private: Vec<BridgeView>,
    /// When the registry was last asked, unix seconds.
    #[ts(type = "number | null")]
    pub list_checked_at: Option<i64>,
    /// A bridge would help and the user has not answered: show the offer.
    pub offer: bool,
    /// False with the user's own servers chosen: bridges carry only the
    /// project's servers.
    pub available: bool,
}

/// What a check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NetCheck {
    pub direct: bool,
    pub bridge: bool,
    pub verdict: Verdict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The direct way carries bytes.
    Direct,
    /// The direct way does not, a bridge does.
    Restricted,
    /// Neither does: there is no network, or no bridge that works.
    Offline,
}

pub fn decide(direct: bool, bridge: bool) -> Verdict {
    match (direct, bridge) {
        (true, _) => Verdict::Direct,
        (false, true) => Verdict::Restricted,
        (false, false) => Verdict::Offline,
    }
}

/// What to do about a verdict, given the mode. Kept apart from the doing
/// so that it can be tested without a network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// `off`: tell the user a bridge would help.
    Offer,
    /// `auto`: use the bridge until `until`.
    Hold { until: i64 },
    /// `auto`: the direct way works again.
    Release,
    /// `off`: the trouble is gone, the offer with it.
    Withdraw,
}

pub fn step(mode: NetMode, verdict: Verdict, now: i64, dismissed_at: Option<i64>) -> Step {
    match (mode, verdict) {
        (NetMode::On, _) => Step::Nothing,
        (NetMode::Off, Verdict::Restricted) => match dismissed_at {
            Some(at) if now - at < OFFER_QUIET_SECS => Step::Nothing,
            _ => Step::Offer,
        },
        (NetMode::Off, Verdict::Direct) => Step::Withdraw,
        (NetMode::Auto, Verdict::Restricted) => Step::Hold { until: now + AUTO_HOLD_SECS },
        (NetMode::Auto, Verdict::Direct) => Step::Release,
        // No network at all says nothing about which way is better.
        (_, Verdict::Offline) => Step::Nothing,
    }
}

/// The host of `wss://host…` or `https://host…`.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', ':', '?']).next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The servers of the project: everything the manifest names, and the
/// registries of bridges (which stand behind servers of the manifest, but
/// need not).
pub fn project_hosts(manifest: &Manifest) -> BTreeSet<String> {
    let sources = manifest.sources.iter().filter_map(|s| match s {
        ManifestSource::Http { url, .. } => Some(url.as_str()),
        ManifestSource::Nostr { .. } => None,
    });
    manifest
        .relays
        .iter()
        .map(|r| r.url.as_str())
        .chain(manifest.media.iter().map(|m| m.url.as_str()))
        .chain(manifest.push.iter().map(|p| p.url.as_str()))
        .chain(sources)
        .chain(trust::REGISTRIES.iter().copied())
        .filter_map(host_of)
        .collect()
}

fn now() -> i64 {
    SystemClock.now().secs()
}

fn transport(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Transport(e.to_string())
}

pub struct NetService {
    store: Store,
    net: Net,
    /// Since when no relay has been connected, while one should be.
    relays_down_since: std::sync::Mutex<Option<Instant>>,
}

impl NetService {
    /// Puts in force what the app was using when it last ran, before any
    /// connection is made. [`NetService::apply`] then brings it up to date.
    pub async fn init(store: Store) -> Result<Self> {
        let net = Net::global().clone();
        // A door that did not open fails what goes through a bridge, and
        // nothing else: the messenger starts all the same.
        if let Err(e) = net.configure(messenger_notify::net::saved(&store).await?).await {
            eprintln!("messenger net: {e}");
        }
        Ok(Self { store, net, relays_down_since: std::sync::Mutex::new(None) })
    }

    /// The runtime stops: the door of the bridges closes with it, so a
    /// messenger that is off holds no listening port and no bridge.
    pub fn shutdown(&self) {
        self.net.close();
    }

    pub async fn mode(&self) -> Result<NetMode> {
        Ok(settings::get(&self.store, KEY_MODE).await?.as_deref().and_then(NetMode::parse).unwrap_or(NetMode::Off))
    }

    async fn stamp(&self, key: &str) -> Result<Option<i64>> {
        Ok(settings::get(&self.store, key).await?.and_then(|s| s.parse().ok()))
    }

    async fn private(&self) -> Result<Vec<BridgeRef>> {
        let text = settings::get(&self.store, KEY_PRIVATE).await?.unwrap_or_else(|| "[]".into());
        let refs: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
        Ok(refs.iter().filter_map(|r| r.parse().ok()).collect())
    }

    async fn set_private(&self, bridges: &[BridgeRef]) -> Result<()> {
        let refs: Vec<String> = bridges.iter().map(BridgeRef::to_string).collect();
        settings::set(&self.store, KEY_PRIVATE, &serde_json::to_string(&refs)?).await
    }

    /// The bridges of the last list, when its signatures still hold.
    async fn listed(&self) -> Result<Vec<BridgeRef>> {
        let Some(text) = settings::get(&self.store, KEY_LIST).await? else { return Ok(Vec::new()) };
        let Ok(signed) = serde_json::from_str::<SignedList>(&text) else { return Ok(Vec::new()) };
        Ok(signed.verify(trust::ROOT_PUB, now().max(0) as u64).map(|l| l.bridges).unwrap_or_default())
    }

    /// Every bridge known, the ones to try first at the front: the one
    /// that carried last, the user's own, the registry's, the built-in.
    pub async fn bridges(&self) -> Result<Vec<BridgeRef>> {
        let last_good: Option<BridgeRef> =
            settings::get(&self.store, KEY_LAST_GOOD).await?.and_then(|r| r.parse().ok());
        let mut known: Vec<BridgeRef> = self.private().await?;
        known.extend(self.listed().await?);
        known.extend(trust::SEEDS.iter().filter_map(|s| s.parse().ok()));
        let mut out: Vec<BridgeRef> = Vec::new();
        // The last good one leads only while it is still among the known:
        // a bridge that left the list is not held on to.
        let first = last_good.filter(|b| known.contains(b));
        for bridge in first.into_iter().chain(known) {
            if !out.contains(&bridge) {
                out.push(bridge);
            }
        }
        Ok(out)
    }

    /// Works out what is in force now and puts it there. Returns whether
    /// the way of the project's servers changed, so that connections made
    /// the old way can be made anew.
    pub async fn apply(&self, relays: &RelayService) -> Result<bool> {
        let was = self.net.is_active();
        let hosts = match relays.servers_mode().await? {
            Some(ServersMode::Veydan) => project_hosts(&relays.current_manifest().await?.0),
            _ => BTreeSet::new(),
        };
        let wanted = match self.mode().await? {
            NetMode::On => true,
            NetMode::Off => false,
            NetMode::Auto => self.stamp(KEY_AUTO_UNTIL).await?.is_some_and(|until| now() < until),
        };
        let config = NetConfig { active: wanted && !hosts.is_empty(), hosts, bridges: self.bridges().await? };
        messenger_notify::net::save(&self.store, &config).await?;
        self.net.configure(config).await?;
        Ok(self.net.is_active() != was)
    }

    pub async fn set_mode(&self, mode: NetMode) -> Result<()> {
        settings::set(&self.store, KEY_MODE, mode.as_str()).await?;
        // The choice answers the offer, whatever the choice is.
        settings::delete(&self.store, KEY_OFFER).await?;
        if mode != NetMode::Auto {
            settings::delete(&self.store, KEY_AUTO_UNTIL).await?;
        }
        Ok(())
    }

    pub async fn status(&self, relays: &RelayService) -> Result<NetStatus> {
        let private = self.private().await?;
        Ok(NetStatus {
            mode: self.mode().await?,
            active: self.net.is_active(),
            bridge: self.net.current().await.map(|b| b.addr.to_string()),
            bridges: self.bridges().await?.len() as u32,
            private: private.iter().map(|b| BridgeView { id: b.id.to_string(), addr: b.addr.to_string() }).collect(),
            list_checked_at: self.stamp(KEY_LIST_CHECKED).await?,
            offer: settings::get_bool(&self.store, KEY_OFFER, false).await?,
            available: relays.servers_mode().await? == Some(ServersMode::Veydan),
        })
    }

    /// Adds a bridge the user was given: its link (`veydan://vlink/…`), or
    /// the reference its own tools print (`address:port#id`).
    pub async fn add_bridge(&self, text: &str) -> Result<()> {
        let bridge = bridge_of(text).map_err(|code| MessengerError::Invalid(code.into()))?;
        let mut private = self.private().await?;
        if !private.contains(&bridge) {
            private.insert(0, bridge);
            self.set_private(&private).await?;
        }
        Ok(())
    }

    pub async fn remove_bridge(&self, id: &str) -> Result<()> {
        let mut private = self.private().await?;
        private.retain(|b| b.id.to_string() != id);
        self.set_private(&private).await
    }

    /// Asks a registry for bridges and keeps the answer when its
    /// signatures hold. Returns how many bridges it named.
    pub async fn refresh_list(&self) -> Result<usize> {
        let client = messenger_http::client(Duration::from_secs(6), Duration::from_secs(12))?;
        let mut last = MessengerError::Transport("no registry is known".into());
        for registry in trust::REGISTRIES {
            let asked = async {
                let response = client.get(format!("{registry}/v1/list")).send().await.map_err(transport)?;
                if !response.status().is_success() {
                    return Err(MessengerError::Transport(format!("the registry answered {}", response.status())));
                }
                let text = response.text().await.map_err(transport)?;
                let signed: SignedList = serde_json::from_str(&text)?;
                let list = signed
                    .verify(trust::ROOT_PUB, now().max(0) as u64)
                    .map_err(|e| MessengerError::Invalid(format!("the list of bridges: {e}")))?;
                Ok((text, list.bridges.len()))
            };
            match asked.await {
                Ok((text, count)) => {
                    settings::set(&self.store, KEY_LIST, &text).await?;
                    settings::set(&self.store, KEY_LIST_CHECKED, &now().to_string()).await?;
                    return Ok(count);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Whether the list is due: never asked, or asked long ago.
    pub async fn list_is_stale(&self) -> Result<bool> {
        Ok(self.stamp(KEY_LIST_CHECKED).await?.is_none_or(|at| now() - at >= LIST_REFRESH_SECS))
    }

    /// Pushes bytes the direct way, whatever the rule says: a client of
    /// its own, bound to a rule that sends nothing to a bridge.
    async fn direct_carries(&self) -> bool {
        let Ok(builder) = messenger_http::builder_on(Net::new(), Duration::from_secs(5), DIRECT_PATIENCE) else {
            return false;
        };
        let Ok(client) = builder.build() else { return false };
        for registry in trust::REGISTRIES {
            let sent = client.post(format!("{registry}/v1/canary")).body(vec![0x5au8; CHECK_BYTES]).send().await;
            if sent.is_ok_and(|r| r.status().is_success()) {
                return true;
            }
        }
        false
    }

    /// Pushes bytes through a bridge, in use or not. The bridge that
    /// carried is remembered.
    async fn bridge_carries(&self) -> Result<bool> {
        let Some(client) = self.net.client() else { return Ok(false) };
        let carried = tokio::time::timeout(BRIDGE_PATIENCE, probe::run(&client, CHECK_BYTES as u64)).await;
        if !matches!(carried, Ok(Ok(_))) {
            return Ok(false);
        }
        if let Some(bridge) = client.current().await {
            settings::set(&self.store, KEY_LAST_GOOD, &bridge.to_string()).await?;
        }
        Ok(true)
    }

    /// Tries both ways and acts on what was found, as the mode says.
    pub async fn check(&self) -> Result<NetCheck> {
        settings::set(&self.store, KEY_CHECKED, &now().to_string()).await?;
        let direct = self.direct_carries().await;
        // The bridge is asked only when it matters.
        let bridge = if direct { false } else { self.bridge_carries().await? };
        let verdict = decide(direct, bridge);
        match step(self.mode().await?, verdict, now(), self.stamp(KEY_OFFER_DISMISSED).await?) {
            Step::Nothing => {}
            Step::Offer => settings::set_bool(&self.store, KEY_OFFER, true).await?,
            Step::Withdraw => settings::delete(&self.store, KEY_OFFER).await?,
            Step::Hold { until } => settings::set(&self.store, KEY_AUTO_UNTIL, &until.to_string()).await?,
            Step::Release => settings::delete(&self.store, KEY_AUTO_UNTIL).await?,
        }
        Ok(NetCheck { direct, bridge, verdict })
    }

    /// A sign of trouble was seen. Says whether a check is worth it now:
    /// not with bridges always on, and not more often than every ten minutes.
    pub async fn check_is_due(&self) -> Result<bool> {
        if self.mode().await? == NetMode::On {
            return Ok(false);
        }
        Ok(self.stamp(KEY_CHECKED).await?.is_none_or(|at| now() - at >= CHECK_EVERY_SECS))
    }

    /// Told every few seconds whether the relays are down while they should
    /// be up. Says true once, when that has lasted long enough to be trouble.
    pub fn relays_stuck(&self, down: bool) -> bool {
        let mut since = self.relays_down_since.lock().expect("relays_down_since");
        match (down, *since) {
            (false, _) => {
                *since = None;
                false
            }
            (true, None) => {
                *since = Some(Instant::now());
                false
            }
            (true, Some(at)) if at.elapsed() >= RELAYS_PATIENCE => {
                // Counted from now again: trouble that stays is reported
                // anew, and the pause between checks holds it back.
                *since = Some(Instant::now());
                true
            }
            (true, Some(_)) => false,
        }
    }

    /// With bridges not always on: was the direct way last tried long ago?
    pub async fn quiet_check_is_due(&self) -> Result<bool> {
        if self.mode().await? == NetMode::On {
            return Ok(false);
        }
        Ok(self.stamp(KEY_CHECKED).await?.is_none_or(|at| now() - at >= CHECK_QUIET_SECS))
    }

    /// In `auto`, with the bridge in use: is it time to try the direct way?
    pub async fn hold_is_over(&self) -> Result<bool> {
        Ok(self.mode().await? == NetMode::Auto && self.stamp(KEY_AUTO_UNTIL).await?.is_some_and(|until| now() >= until))
    }

    pub async fn dismiss_offer(&self) -> Result<()> {
        settings::delete(&self.store, KEY_OFFER).await?;
        settings::set(&self.store, KEY_OFFER_DISMISSED, &now().to_string()).await
    }
}

// ─── The runtime's side ──────────────────────────────────────────────────────

use crate::MessengerRuntime;
use messenger_core::traits::UiEvent;

impl MessengerRuntime {
    pub async fn net_status(&self) -> Result<NetStatus> {
        self.net.status(&self.relays).await
    }

    /// Puts the choice, the manifest and the bridges known in force. When
    /// the way of the project's servers changed, the relays are connected
    /// anew: a connection made the old way would stay on it.
    pub async fn net_apply(&self) -> Result<()> {
        if self.net.apply(&self.relays).await? {
            let pool = self.relays.pool().await;
            pool.disconnect().await;
            pool.connect().await;
        }
        let _ = self.ui.send(UiEvent { name: "net".into(), payload: serde_json::json!({}) });
        Ok(())
    }

    pub async fn net_set_mode(&self, mode: NetMode) -> Result<NetStatus> {
        self.net.set_mode(mode).await?;
        if mode != NetMode::Off {
            // Fresh bridges are worth a moment; the ones known do otherwise.
            if let Err(e) = self.net.refresh_list().await {
                eprintln!("messenger net: the list of bridges was not refreshed: {e}");
            }
        }
        self.net_apply().await?;
        if mode != NetMode::Off && self.net_status().await?.active {
            // Through the bridge now: a newer list may be had this way even
            // when the direct way gave none.
            if self.net.list_is_stale().await? && self.net.refresh_list().await.is_ok() {
                self.net_apply().await?;
            }
        }
        self.net_status().await
    }

    /// Tries the direct way and a bridge now, and acts on it as the mode says.
    pub async fn net_check(&self) -> Result<NetCheck> {
        let check = self.net.check().await?;
        self.net_apply().await?;
        Ok(check)
    }

    /// Something failed that a throttled way would explain. Checks, unless
    /// it did so a moment ago.
    pub async fn net_trouble(&self) -> Result<Option<NetCheck>> {
        if !self.net.check_is_due().await? {
            return Ok(None);
        }
        self.net_check().await.map(Some)
    }

    /// Called every few seconds while the app runs: notices relays that
    /// stay down, and tries the direct way once a day when nothing else
    /// gave a reason to.
    pub async fn net_watch(&self) -> Result<()> {
        if self.relays.servers_mode().await? != Some(ServersMode::Veydan) {
            return Ok(());
        }
        let status = self.status().await?;
        let down = status.session_active
            && !status.silent_mode
            && status.relays_total > 0
            && status.relays_connected == 0;
        if self.net.relays_stuck(down) {
            self.net_trouble().await?;
        } else if status.session_active && self.net.quiet_check_is_due().await? {
            self.net_check().await?;
        }
        Ok(())
    }

    /// The housekeeping of an hour: a list that grew old, a hold that ran out.
    pub async fn net_tick(&self) -> Result<()> {
        let status = self.net_status().await?;
        if !status.available {
            return Ok(());
        }
        if status.mode != NetMode::Off && self.net.list_is_stale().await? && self.net.refresh_list().await.is_ok() {
            self.net_apply().await?;
        }
        if self.net.hold_is_over().await? {
            self.net_check().await?;
        }
        Ok(())
    }

    pub async fn net_bridge_add(&self, reference: &str) -> Result<NetStatus> {
        self.net.add_bridge(reference).await?;
        self.net_apply().await?;
        self.net_status().await
    }

    pub async fn net_bridge_remove(&self, id: &str) -> Result<NetStatus> {
        self.net.remove_bridge(id).await?;
        self.net_apply().await?;
        self.net_status().await
    }

    /// Looks at what the runtime tells the UI. A transfer that failed on the
    /// network (not on the file, not on what the server answered) is a sign
    /// that the direct way may be restricted: uploads run apart from the
    /// runtime, and this event is how their failures are heard of.
    pub async fn net_heard(&self, event: &UiEvent) {
        let media = event.name == "error" && event.payload["family"] == "media";
        if media && event.payload["error"].as_str().is_some_and(|e| e.contains("err.network")) {
            if let Err(e) = self.net_trouble().await {
                eprintln!("messenger net: check after a failed transfer: {e}");
            }
        }
    }

    pub async fn net_offer_dismiss(&self) -> Result<()> {
        self.net.dismiss_offer().await?;
        let _ = self.ui.send(UiEvent { name: "net".into(), payload: serde_json::json!({}) });
        Ok(())
    }
}

/// The bridge a text names: a link (`veydan://vlink/…`) or a reference
/// (`address:port#id`). A well-formed link of another type, such as
/// `veydan://bridge/…` (the word is kept for bridges to other networks),
/// is `net_bridge_link_type`; anything else is `net_bridge_invalid`.
fn bridge_of(text: &str) -> std::result::Result<BridgeRef, &'static str> {
    const INVALID: &str = "net_bridge_invalid";
    let text = text.trim();
    let reference = if text.starts_with(messenger_links::SCHEME) {
        match messenger_links::BridgeLink::parse(text) {
            Ok(link) => link.reference(),
            Err(messenger_links::LinkError::Type) => return Err("net_bridge_link_type"),
            Err(_) => return Err(INVALID),
        }
    } else {
        text.to_string()
    };
    reference.parse().map_err(|_| INVALID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_transport::EMBEDDED_MANIFEST_JSON;

    const ID: &str = "9c5f434def40ebc2879201642bab6c69063578c4a302aa92de5918b3663ffbfc";

    #[test]
    fn a_bridge_is_added_by_its_vlink_link_or_its_reference() {
        let by_link = bridge_of(&format!(" veydan://vlink/{ID}?a=203.0.113.7%3A443 ")).unwrap();
        let by_reference = bridge_of(&format!("203.0.113.7:443#{ID}")).unwrap();
        assert_eq!(by_link, by_reference);
        assert_eq!(by_link.to_string(), format!("203.0.113.7:443#{ID}"));
        let v6 = bridge_of(&format!("veydan://vlink/{}?a=%5B2001%3Adb8%3A%3A1%5D%3A8443", ID.to_uppercase())).unwrap();
        assert_eq!(v6.addr.to_string(), "[2001:db8::1]:8443");
        assert_eq!(v6.id.to_string(), ID);
    }

    #[test]
    fn a_link_of_another_type_is_told_from_a_broken_one() {
        // `bridge` is kept for bridges to other networks: no alias.
        assert_eq!(bridge_of(&format!("veydan://bridge/{ID}?a=203.0.113.7%3A443")), Err("net_bridge_link_type"));
        assert_eq!(bridge_of(&format!("veydan://group/{ID}")), Err("net_bridge_link_type"));
        assert_eq!(bridge_of("veydan://vlink/abc?a=203.0.113.7%3A443"), Err("net_bridge_invalid"));
        assert_eq!(bridge_of("not a bridge"), Err("net_bridge_invalid"));
        assert_eq!(bridge_of("veydan:/"), Err("net_bridge_invalid"));
    }

    #[test]
    fn the_projects_hosts_are_those_of_the_manifest_and_the_registries() {
        let manifest = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        let hosts = project_hosts(&manifest);
        for url in manifest.relays.iter().map(|r| &r.url).chain(manifest.media.iter().map(|m| &m.url)) {
            assert!(hosts.contains(&host_of(url).unwrap()), "{url}");
        }
        for registry in trust::REGISTRIES {
            assert!(hosts.contains(&host_of(registry).unwrap()), "{registry}");
        }
        assert!(hosts.iter().all(|h| !h.contains('/') && !h.contains(':')));
    }

    #[test]
    fn hosts_are_read_out_of_addresses() {
        assert_eq!(host_of("wss://Relay.Example.org").as_deref(), Some("relay.example.org"));
        assert_eq!(host_of("https://m.example.org:8443/vlink?x=1").as_deref(), Some("m.example.org"));
        assert_eq!(host_of("https://m.example.org?key=1").as_deref(), Some("m.example.org"));
        assert_eq!(host_of("not an address"), None);
    }

    #[test]
    fn a_verdict_is_what_carried() {
        assert_eq!(decide(true, false), Verdict::Direct);
        assert_eq!(decide(true, true), Verdict::Direct);
        assert_eq!(decide(false, true), Verdict::Restricted);
        assert_eq!(decide(false, false), Verdict::Offline);
    }

    #[test]
    fn off_offers_once_and_respects_not_now() {
        let now = 1_000_000;
        assert_eq!(step(NetMode::Off, Verdict::Restricted, now, None), Step::Offer);
        // Declined an hour ago: quiet.
        assert_eq!(step(NetMode::Off, Verdict::Restricted, now, Some(now - 3600)), Step::Nothing);
        // Declined long ago: asked again.
        assert_eq!(step(NetMode::Off, Verdict::Restricted, now, Some(now - OFFER_QUIET_SECS)), Step::Offer);
        // The trouble went away: so does the offer.
        assert_eq!(step(NetMode::Off, Verdict::Direct, now, None), Step::Withdraw);
    }

    #[test]
    fn auto_holds_the_bridge_and_lets_go() {
        let now = 1_000_000;
        assert_eq!(step(NetMode::Auto, Verdict::Restricted, now, None), Step::Hold { until: now + AUTO_HOLD_SECS });
        assert_eq!(step(NetMode::Auto, Verdict::Direct, now, None), Step::Release);
    }

    #[test]
    fn no_network_changes_nothing_and_on_is_left_alone() {
        for mode in [NetMode::Off, NetMode::On, NetMode::Auto] {
            assert_eq!(step(mode, Verdict::Offline, 1, None), Step::Nothing);
        }
        for verdict in [Verdict::Direct, Verdict::Restricted, Verdict::Offline] {
            assert_eq!(step(NetMode::On, verdict, 1, None), Step::Nothing);
        }
    }
}

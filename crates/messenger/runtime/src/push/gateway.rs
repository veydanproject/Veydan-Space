// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The push of my private servers, through the gateway of the Veydan
//! network (services/push/spec/protocol.md, «The gateway»;
//! internal/messenger-wire.md, «Push через шлюз»).
//!
//! A private server whose push is on (`push_mode = veydan`, and the address
//! of its push server, `push_url`) runs a push server of its own. It cannot
//! wake the phone by itself: the token of FCM is the app's, whose key only
//! the push servers of the network hold. So the device takes a **ticket**
//! from the gateway that server names (a push server of the network, the
//! home one or another of the manifest), which stands for its token without
//! naming it, and registers at the private push server with the channel
//! `gateway`: the ticket, and the **push key** of the device, a key made for
//! pushes alone that every push of the server is sealed to.
//!
//! - **The push key**: x-only secp256k1, one per identity on this device,
//!   made when first needed, its secret in the `SecretStore`
//!   (`push.key.<pubkey>`), its public half in `push.key_pub.<pubkey>`;
//!   never published. The push handler gets it in its bundle.
//! - **Tickets**: one per private server (a ticket is the right to wake, and
//!   one taken back stops one server alone), asked of the gateway after the
//!   gateway knows the device, kept in `push.private` with `expires_at`,
//!   asked anew when fewer than seven days are left, the old one taken back
//!   once the new one is registered. A gateway keeps eight live tickets of
//!   a device; seven private servers per gateway are told, the eighth
//!   ticket is the room a renewal needs.
//! - **A gateway that is not the home push server** is told of the device
//!   too, with its token and nothing to watch, for as long as a server of
//!   mine wakes through it.
//! - **The seal**: what the private push server says of its gateway and its
//!   key in `/v1/info` is kept on its row (`push_gateway`, `push_sender`);
//!   a sealed push from any other key is dropped by the handler.

use super::{now, random_id, PushChannel, Told, RENEW_AFTER, RETRY_AFTER};
use crate::MessengerRuntime;
use messenger_core::{MessengerError, Result};
use messenger_push::client::{server_address, PushError};
use messenger_push::{Channel, DevicePut, GroupWatch, RelayAnswer, RelayWatch, TicketIssued, VpushClient};
use messenger_store::servers::{self, ServerRow};
use messenger_store::settings;
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The state of the private targets: one JSON value.
const KEY_PRIVATE: &str = "push.private";
/// Tickets still to take back at their gateways ([`TakeBack`]).
const KEY_TAKE_BACK: &str = "push.take_back";
/// Tickets kept to take back at most; a gateway gives eight per device.
const TAKE_BACK_MAX: usize = 64;
/// The `SecretStore` key of the push key of identity `<pubkey>`.
const SECRET_PUSH_KEY_PREFIX: &str = "push.key.";
/// Its public half, in the settings: the fingerprint of the handler's
/// bundle reads it without a secret.
const KEY_PUSH_KEY_PUB_PREFIX: &str = "push.key_pub.";
/// Said when a push key was made: the host gives the push handler its
/// bundle anew at once (`notify_bundle` carries the key), before the first
/// registration that names the key goes out.
pub const UI_EVENT_PUSH_KEY: &str = "push.key";
/// A ticket with fewer seconds than this left is asked anew.
pub const TICKET_RENEW_BEFORE: u64 = 7 * 86_400;
/// Private servers told through one gateway at most: of the eight live
/// tickets a gateway keeps for a device, one is left for a renewal.
pub const SERVERS_PER_GATEWAY: usize = 7;

/// What is kept of the private targets.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PrivateState {
    /// By server id.
    #[serde(default)]
    servers: BTreeMap<String, Target>,
    /// Gateways that are not the home push server, by their address: told
    /// of the device (its token, nothing to watch) so that they issue it
    /// tickets.
    #[serde(default)]
    gateways: BTreeMap<String, Aux>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Target {
    /// The device at this push server: an id of its own, so that two
    /// private servers do not see the same one.
    device_id: String,
    #[serde(default)]
    told: Option<Told>,
    #[serde(default)]
    ticket: Option<Held>,
    /// Tickets replaced and not taken back yet: taken back once the new
    /// one is registered.
    #[serde(default)]
    retired: Vec<Held>,
    #[serde(default)]
    error: Option<String>,
    /// When the last attempt failed; 0 after a success.
    #[serde(default)]
    last_try: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Held {
    /// The gateway that issued it.
    gateway: String,
    /// The identity it was asked as.
    pubkey: String,
    ticket: String,
    expires_at: u64,
}

impl Held {
    fn good_for(&self, gateway: &str, pubkey: &str) -> bool {
        self.gateway == gateway && self.pubkey == pubkey && self.expires_at > now() + TICKET_RENEW_BEFORE
    }
}

/// Tickets the gateway could not be asked to take back (it was out of
/// reach, or said no for a while): kept apart from `PrivateState`, which
/// goes with the server and with pushes off, and asked again every round
/// until the gateway takes them back, says it has none, or they run out.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TakeBack {
    #[serde(default)]
    tickets: Vec<Held>,
    /// When they were last asked.
    #[serde(default)]
    last_try: u64,
}

impl TakeBack {
    /// Tickets past their end are dead at the gateway anyway; of the rest,
    /// the ones that live longest are kept, `TAKE_BACK_MAX` at most.
    fn trim(&mut self, now: u64) {
        self.tickets.retain(|h| h.expires_at > now);
        self.tickets.sort_by_key(|h| std::cmp::Reverse(h.expires_at));
        self.tickets.truncate(TAKE_BACK_MAX);
    }
}

/// Whether a take-back that failed is worth asking again: not when the
/// gateway has no such ticket (it is gone) nor when its address is no
/// address at all.
fn worth_asking_again(e: &PushError) -> bool {
    !matches!(e, PushError::Refused { status: 404, .. } | PushError::Address(_))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Aux {
    #[serde(default)]
    told: Option<Told>,
}

/// Where the push of one private server stands, for the settings screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTarget {
    pub server_id: String,
    pub name: Option<String>,
    /// The push server of the server.
    pub server: String,
    /// The gateway it wakes this device through, as it said.
    pub gateway: Option<String>,
    /// `pending`: not told yet, or told something older.
    /// `registered`: the server knows what is wanted.
    /// `failed`: the last telling failed; `error` says why
    /// (`push_gateway_unknown`: the gateway it names is no push server of
    /// the network; `push_ticket_limit`: more servers wake through one
    /// gateway than it gives tickets for; `push_no_gateway`: the server
    /// wakes no phone through a gateway).
    pub state: String,
    pub error: Option<String>,
    pub last_ok_at: Option<u64>,
    pub expires_at: Option<u64>,
    /// When the ticket it holds runs out; it is renewed a week before.
    pub ticket_expires_at: Option<u64>,
    pub relays: Vec<RelayAnswer>,
}

/// A live ticket of mine at a gateway, as the gateway shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTicket {
    pub gateway: String,
    /// The first 8 hex and `…`.
    pub ticket: String,
    pub device_id: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub last_wake_at: Option<u64>,
    /// The private server of this device that holds it; `None` for a
    /// ticket this device does not use (another install, one replaced).
    pub server_id: Option<String>,
}

fn fail(word: &str, why: impl std::fmt::Display) -> MessengerError {
    MessengerError::Transport(format!("{word}: {why}"))
}

/// The words of a failure as the status shows them, the same as the home
/// server's (`push_unreachable: …`, `push_refused_<code>: …`), without the
/// kind of error in front.
fn said(e: &MessengerError) -> String {
    match e {
        MessengerError::Transport(s) | MessengerError::Invalid(s) | MessengerError::Crypto(s) => s.clone(),
        other => other.to_string(),
    }
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl MessengerRuntime {
    async fn push_private_state(&self) -> Result<PrivateState> {
        Ok(settings::get(&self.store, KEY_PRIVATE)
            .await?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    async fn push_private_save(&self, state: &PrivateState) -> Result<()> {
        if state.servers.is_empty() && state.gateways.is_empty() {
            return settings::delete(&self.store, KEY_PRIVATE).await;
        }
        settings::set(&self.store, KEY_PRIVATE, &serde_json::to_string(state)?).await
    }

    // ─── The push key ───────────────────────────────────────────────────────

    /// The push key of identity `pubkey` on this device; made and kept
    /// when `create` and there is none. Never published: the private push
    /// servers learn its public half, the handler its secret.
    pub(crate) async fn push_key(&self, pubkey: &str, create: bool) -> Result<Option<Keys>> {
        let name = format!("{SECRET_PUSH_KEY_PREFIX}{pubkey}");
        if let Some(secret) = self.secrets.get(&name).await? {
            let hex = String::from_utf8(secret.to_vec()).map_err(|_| MessengerError::Crypto("push key".into()))?;
            let keys = Keys::parse(&hex).map_err(|_| MessengerError::Crypto("push key".into()))?;
            let public = keys.public_key().to_hex();
            let key_pub = format!("{KEY_PUSH_KEY_PUB_PREFIX}{pubkey}");
            if settings::get(&self.store, &key_pub).await?.as_deref() != Some(public.as_str()) {
                settings::set(&self.store, &key_pub, &public).await?;
            }
            return Ok(Some(keys));
        }
        if !create {
            return Ok(None);
        }
        let keys = Keys::generate();
        self.secrets.put(&name, keys.secret_key().to_secret_hex().as_bytes()).await?;
        // The public half is in the fingerprint of the handler's bundle
        // (`notify_fingerprint`): the host hands the handler the key.
        settings::set(&self.store, &format!("{KEY_PUSH_KEY_PUB_PREFIX}{pubkey}"), &keys.public_key().to_hex()).await?;
        // At once, not at the host's next look: the server this key is
        // made for asks with a sealed push right after the registration,
        // and a handler without the key would show it as «something came».
        let _ = self.ui.send(messenger_core::traits::UiEvent { name: UI_EVENT_PUSH_KEY.into(), payload: serde_json::json!({}) });
        Ok(Some(keys))
    }

    /// The public half of the push key of `pubkey`, without a secret.
    pub(crate) async fn push_key_public(&self, pubkey: &str) -> Result<Option<String>> {
        settings::get(&self.store, &format!("{KEY_PUSH_KEY_PUB_PREFIX}{pubkey}")).await
    }

    // ─── The servers of mine ────────────────────────────────────────────────

    /// The push server of private server `id`: `Some(url)` turns its push
    /// on, through the gateway it names; `None` turns it off, and the next
    /// round tells that server to forget the device. The card names it
    /// (`pu=`); my other devices take it with the server from the snapshot
    /// of my relations. `server_unknown` for an id that is no private
    /// server of this device.
    pub async fn server_set_push(&self, id: &str, push_url: Option<String>) -> Result<()> {
        let url = match push_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => Some(server_address(u).map_err(MessengerError::from)?),
            None => None,
        };
        if !servers::set_push(&self.store, id, url.as_deref()).await? {
            return Err(MessengerError::Invalid("server_unknown".into()));
        }
        self.dm.mark_relations_owed().await?;
        Ok(())
    }

    /// The private servers whose push servers are to be told, in the order
    /// they were added, with their push server.
    async fn push_private_wanted(&self) -> Result<Vec<(ServerRow, servers::ServerPush, String)>> {
        let mut out = Vec::new();
        for row in servers::private(&self.store).await? {
            let Some(push) = servers::push_of(&self.store, &row.id).await? else { continue };
            if push.push_mode != servers::PUSH_VEYDAN {
                continue;
            }
            let Some(url) = push.push_url.as_deref().and_then(|u| server_address(u).ok()) else { continue };
            out.push((row, push, url));
        }
        Ok(out)
    }

    /// A gateway this device may hand its token to: the home push server,
    /// or a push server of the manifest. Any other a private server names
    /// is refused: the token and my key are not for its choice of hosts.
    async fn push_gateway_known(&self, gateway: &str, home: Option<&str>) -> Result<bool> {
        if home == Some(gateway) {
            return Ok(true);
        }
        let (manifest, _) = self.relays.current_manifest().await?;
        Ok(manifest
            .push
            .iter()
            .filter_map(|p| server_address(&p.url).ok())
            .any(|u| u == gateway))
    }

    /// What the push server of private server `row` should know: the
    /// gateway's ticket and my push key, its relay alone, its groups.
    async fn push_private_put(
        &self,
        keys: &Keys,
        channel: &PushChannel,
        row: &ServerRow,
        ticket: &str,
        push_key: &str,
    ) -> Result<DevicePut> {
        let me = messenger_core::PubKey::parse(&keys.public_key().to_hex())
            .ok_or_else(|| MessengerError::Crypto("the key has no hex form".into()))?;
        // The groups that live on this server: left out of the home
        // registration (internal/messenger-wire.md, «Группы на сервере»),
        // told here, by the keys of their marks.
        let mut groups = Vec::new();
        for group in self
            .groups()
            .list(&me)
            .await?
            .into_iter()
            .filter(|g| g.membership == "joined" && g.server_id.as_deref() == Some(row.id.as_str()))
        {
            let keys = self.groups().push_keys(&group.id).await?;
            if !keys.is_empty() {
                groups.push(GroupWatch { id: group.id, keys });
            }
        }
        groups.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(DevicePut {
            app_id: channel.app_id.clone(),
            channel: Channel::Gateway { ticket: ticket.to_string(), pubkey: push_key.to_string() },
            app_version: channel.app_version.clone(),
            prefs: self.push_prefs().await?,
            author_key: Some(messenger_dm::pushtags::author_key(keys)),
            relays: vec![RelayWatch { url: servers::normalize(&row.relay_url), dm: true, groups: true }],
            groups,
        })
    }

    /// Tells the push servers of my private servers what to watch, through
    /// the gateway each names; forgets the device at those whose push went
    /// off or whose server went. `home`: the home push server, if any.
    pub(super) async fn push_reconcile_private(
        &self,
        force: bool,
        keys: &Keys,
        channel: &PushChannel,
        home: Option<&str>,
    ) -> Result<()> {
        let mut state = self.push_private_state().await?;
        let wanted = self.push_private_wanted().await?;
        if wanted.is_empty() && state.servers.is_empty() && state.gateways.is_empty() {
            return Ok(());
        }
        // The provider of the phone carries the gateway's wakes: FCM alone.
        if channel.provider != "fcm" {
            return Ok(());
        }

        let ids: HashSet<&str> = wanted.iter().map(|(r, _, _)| r.id.as_str()).collect();
        let gone: Vec<String> = state.servers.keys().filter(|id| !ids.contains(id.as_str())).cloned().collect();
        for id in gone {
            if let Some(target) = state.servers.remove(&id) {
                self.push_forget_target(&target, keys).await;
            }
            servers::set_push_seal(&self.store, &id, None, None).await?;
        }

        let mut per_gateway: HashMap<String, usize> = HashMap::new();
        for (row, push, url) in &wanted {
            let target = state.servers.entry(row.id.clone()).or_insert_with(|| Target {
                device_id: random_id().unwrap_or_default(),
                ..Default::default()
            });
            if target.device_id.is_empty() {
                target.device_id = random_id()?;
            }
            // Counted by the gateway it named last; a server beyond the
            // tickets one gateway gives is not told.
            if let Some(gateway) = &push.push_gateway {
                let n = per_gateway.entry(gateway.clone()).or_default();
                *n += 1;
                if *n > SERVERS_PER_GATEWAY {
                    let mut target = std::mem::take(target);
                    self.push_forget_target(&target, keys).await;
                    target.told = None;
                    target.ticket = None;
                    target.error = Some(format!(
                        "push_ticket_limit: no more than {SERVERS_PER_GATEWAY} servers wake this device through one gateway"
                    ));
                    state.servers.insert(row.id.clone(), target);
                    continue;
                }
            }
            if !force {
                if let Some(fresh) = self.push_private_fresh(keys, channel, row, push, url, target).await? {
                    if fresh {
                        continue;
                    }
                }
                if target.last_try > 0 && now().saturating_sub(target.last_try) < RETRY_AFTER {
                    continue;
                }
            }
            let mut target_now = std::mem::take(target);
            // A gateway not heard of yet is counted once it is.
            let uncounted = if push.push_gateway.is_none() { Some(&mut per_gateway) } else { None };
            let done = self
                .push_register_private(keys, channel, home, row, url, &mut target_now, &mut state.gateways, uncounted)
                .await;
            match done {
                Ok(()) => {
                    target_now.error = None;
                    target_now.last_try = 0;
                }
                Err(e) => {
                    target_now.error = Some(said(&e));
                    target_now.last_try = now();
                }
            }
            state.servers.insert(row.id.clone(), target_now);
        }

        // A gateway other than the home server is told of the device for
        // as long as a ticket of it is held.
        let used: HashSet<String> = state
            .servers
            .values()
            .filter_map(|t| t.ticket.as_ref().map(|h| h.gateway.clone()))
            .collect();
        let idle: Vec<String> = state.gateways.keys().filter(|g| !used.contains(*g)).cloned().collect();
        for gateway in idle {
            if let Some(Aux { told: Some(told) }) = state.gateways.remove(&gateway) {
                self.push_forget(&told, keys).await;
            }
        }
        self.push_private_save(&state).await
    }

    /// Whether the push server of `row` knows what is wanted, judged
    /// without the network; `None` when that cannot be said here (no
    /// ticket, one near its end, no push key yet).
    async fn push_private_fresh(
        &self,
        keys: &Keys,
        channel: &PushChannel,
        row: &ServerRow,
        push: &servers::ServerPush,
        url: &str,
        target: &Target,
    ) -> Result<Option<bool>> {
        let pubkey = keys.public_key().to_hex();
        let (Some(told), Some(held), Some(gateway), Some(push_key)) = (
            target.told.as_ref(),
            target.ticket.as_ref(),
            push.push_gateway.as_deref(),
            self.push_key_public(&pubkey).await?,
        ) else {
            return Ok(None);
        };
        if !held.good_for(gateway, &pubkey) {
            return Ok(None);
        }
        let wanted = self.push_private_put(keys, channel, row, &held.ticket, &push_key).await?;
        let fingerprint = Self::push_fingerprint(url, &pubkey, &target.device_id, &wanted);
        Ok(Some(
            told.fingerprint == fingerprint && told.pubkey == pubkey && now().saturating_sub(told.at) < RENEW_AFTER,
        ))
    }

    /// One private server told: its push server asked about itself, the
    /// gateway it names told of the device when it is not the home one, a
    /// ticket asked when there is none good for a week more, the
    /// registration sent; a ticket the push server finds unknown at the
    /// gateway (`token_invalid`) is asked anew, and sent once more.
    #[allow(clippy::too_many_arguments)]
    async fn push_register_private(
        &self,
        keys: &Keys,
        channel: &PushChannel,
        home: Option<&str>,
        row: &ServerRow,
        url: &str,
        target: &mut Target,
        gateways: &mut BTreeMap<String, Aux>,
        uncounted: Option<&mut HashMap<String, usize>>,
    ) -> Result<()> {
        let pubkey = keys.public_key().to_hex();
        let client = VpushClient::new(url)?;
        let info = client.info().await?;
        let Some(seal) = info.gateway.as_ref().filter(|_| info.serves(&channel.app_id, "gateway")) else {
            return Err(fail("push_no_gateway", "the push server of this server wakes no phone of this app through a gateway"));
        };
        let gateway = server_address(&seal.url).map_err(MessengerError::from)?;
        if !hex64(&seal.sender) {
            return Err(fail("push_unreadable", "the key of the seal is not 64 hex"));
        }
        if !self.push_gateway_known(&gateway, home).await? {
            return Err(fail("push_gateway_unknown", format!("{gateway} is no push server of the Veydan network")));
        }
        if let Some(counts) = uncounted {
            let n = counts.entry(gateway.clone()).or_default();
            *n += 1;
            if *n > SERVERS_PER_GATEWAY {
                return Err(fail(
                    "push_ticket_limit",
                    format!("no more than {SERVERS_PER_GATEWAY} servers wake this device through one gateway"),
                ));
            }
        }
        // Kept before the registration: the server checks the ticket with
        // a sealed push, which this device must know to show nothing of.
        servers::set_push_seal(&self.store, &row.id, Some(&gateway), Some(&seal.sender)).await?;

        // The gateway must know the device before it gives a ticket.
        let gateway_device = self.push_device_id(&pubkey).await?;
        if home == Some(gateway.as_str()) {
            let known = self.push_told().await?.is_some_and(|t| t.server == gateway && t.pubkey == pubkey);
            if !known {
                return Err(fail("push_gateway_pending", "the push server of the network does not know this device yet"));
            }
        } else {
            self.push_aux_ensure(keys, channel, &gateway, &gateway_device, gateways).await?;
        }

        let push_key = self
            .push_key(&pubkey, true)
            .await?
            .ok_or_else(|| MessengerError::Crypto("no push key".into()))?
            .public_key()
            .to_hex();
        if !target.ticket.as_ref().is_some_and(|h| h.good_for(&gateway, &pubkey)) {
            let fresh = Self::push_ticket_ask(keys, &gateway, &gateway_device, &pubkey).await?;
            if let Some(old) = target.ticket.replace(fresh) {
                target.retired.push(old);
            }
        }
        let mut attempt = 0;
        let answer = loop {
            let held = target.ticket.clone().expect("a ticket was just made sure of");
            let wanted = self.push_private_put(keys, channel, row, &held.ticket, &push_key).await?;
            let fingerprint = Self::push_fingerprint(url, &pubkey, &target.device_id, &wanted);
            match client.put_device(keys, &target.device_id, &wanted).await {
                Ok(answer) => {
                    break Told {
                        server: url.to_string(),
                        pubkey: pubkey.clone(),
                        device_id: target.device_id.clone(),
                        fingerprint,
                        at: now(),
                        answer,
                    }
                }
                // The gateway did not know the ticket: expired, taken
                // back, gone with a dead token. A new one, once.
                Err(e) if attempt == 0 && e.code() == Some("token_invalid") => {
                    attempt += 1;
                    let fresh = Self::push_ticket_ask(keys, &gateway, &gateway_device, &pubkey).await?;
                    if let Some(old) = target.ticket.replace(fresh) {
                        target.retired.push(old);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        };
        target.told = Some(answer);
        // The tickets replaced are taken back; one the gateway no longer
        // has is gone already.
        let mut kept = Vec::new();
        for old in std::mem::take(&mut target.retired) {
            if old.pubkey != pubkey {
                continue;
            }
            if let Err(PushError::Unreachable(_)) = Self::push_ticket_take_back(keys, &old).await {
                kept.push(old);
            }
        }
        target.retired = kept;
        Ok(())
    }

    async fn push_ticket_ask(keys: &Keys, gateway: &str, device_id: &str, pubkey: &str) -> Result<Held> {
        let TicketIssued { ticket, expires_at } = VpushClient::new(gateway)?.ticket_issue(keys, device_id).await?;
        if !hex64(&ticket) {
            return Err(fail("push_unreadable", "the ticket is not 64 hex"));
        }
        Ok(Held { gateway: gateway.to_string(), pubkey: pubkey.to_string(), ticket, expires_at })
    }

    async fn push_ticket_take_back(keys: &Keys, held: &Held) -> std::result::Result<(), PushError> {
        VpushClient::new(&held.gateway)?.ticket_delete(keys, &held.ticket).await
    }

    /// A gateway that is not the home push server, told of the device: its
    /// token, nothing to watch.
    async fn push_aux_ensure(
        &self,
        keys: &Keys,
        channel: &PushChannel,
        gateway: &str,
        device_id: &str,
        gateways: &mut BTreeMap<String, Aux>,
    ) -> Result<()> {
        let pubkey = keys.public_key().to_hex();
        let wanted = DevicePut {
            app_id: channel.app_id.clone(),
            channel: Channel::Fcm { token: channel.token.clone() },
            app_version: channel.app_version.clone(),
            prefs: self.push_prefs().await?,
            author_key: None,
            relays: Vec::new(),
            groups: Vec::new(),
        };
        let fingerprint = Self::push_fingerprint(gateway, &pubkey, device_id, &wanted);
        let aux = gateways.entry(gateway.to_string()).or_default();
        if aux
            .told
            .as_ref()
            .is_some_and(|t| t.fingerprint == fingerprint && now().saturating_sub(t.at) < RENEW_AFTER)
        {
            return Ok(());
        }
        let answer = VpushClient::new(gateway)?.put_device(keys, device_id, &wanted).await?;
        aux.told = Some(Told {
            server: gateway.to_string(),
            pubkey,
            device_id: device_id.to_string(),
            fingerprint,
            at: now(),
            answer,
        });
        Ok(())
    }

    /// The push server of a private server forgets the device, and its
    /// tickets are taken back. A ticket the gateway could not be asked to
    /// take back is kept to be asked again (`push_take_back_later`): until
    /// it is, the server it was given to can still wake the phone.
    async fn push_forget_target(&self, target: &Target, keys: &Keys) {
        if let Some(told) = &target.told {
            self.push_forget(told, keys).await;
        }
        let pubkey = keys.public_key().to_hex();
        let mut later = Vec::new();
        for held in target.ticket.iter().chain(target.retired.iter()).filter(|h| h.pubkey == pubkey) {
            if let Err(e) = Self::push_ticket_take_back(keys, held).await {
                eprintln!("messenger push: a ticket was not taken back, asked again later: {e}");
                if worth_asking_again(&e) {
                    later.push(held.clone());
                }
            }
        }
        if let Err(e) = self.push_take_back_later(later).await {
            eprintln!("messenger push: the tickets to take back were not kept: {e}");
        }
    }

    async fn push_take_back_state(&self) -> Result<TakeBack> {
        Ok(settings::get(&self.store, KEY_TAKE_BACK)
            .await?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    async fn push_take_back_save(&self, state: &TakeBack) -> Result<()> {
        if state.tickets.is_empty() {
            return settings::delete(&self.store, KEY_TAKE_BACK).await;
        }
        settings::set(&self.store, KEY_TAKE_BACK, &serde_json::to_string(state)?).await
    }

    /// Keeps `held` to be taken back at the next rounds: dead ones (past
    /// their end) go, the newest `TAKE_BACK_MAX` stay.
    async fn push_take_back_later(&self, held: Vec<Held>) -> Result<()> {
        if held.is_empty() {
            return Ok(());
        }
        let mut state = self.push_take_back_state().await?;
        for h in held {
            if !state.tickets.iter().any(|k| k.gateway == h.gateway && k.ticket == h.ticket) {
                state.tickets.push(h);
            }
        }
        // Asked just now: the next try waits as any other does.
        state.last_try = now();
        state.trim(now());
        self.push_take_back_save(&state).await
    }

    /// The tickets kept to take back, asked again: every round of
    /// `push_reconcile` (not sooner than `RETRY_AFTER` after the last try
    /// unless `force`), whether pushes are on or not and whether the server
    /// they were given to is still mine or long gone. One the gateway no
    /// longer has is done with; one of another identity waits for its key.
    pub(super) async fn push_take_back_retry(&self, keys: &Keys, force: bool) -> Result<()> {
        let mut state = self.push_take_back_state().await?;
        if state.tickets.is_empty() {
            return Ok(());
        }
        if !force && now().saturating_sub(state.last_try) < RETRY_AFTER {
            return Ok(());
        }
        let pubkey = keys.public_key().to_hex();
        let mut kept = Vec::new();
        for held in std::mem::take(&mut state.tickets) {
            if held.pubkey != pubkey {
                kept.push(held);
                continue;
            }
            match Self::push_ticket_take_back(keys, &held).await {
                Ok(()) => {}
                Err(e) if worth_asking_again(&e) => kept.push(held),
                Err(e) => eprintln!("messenger push: a ticket kept to take back is given up: {e}"),
            }
        }
        state.tickets = kept;
        state.last_try = now();
        state.trim(now());
        self.push_take_back_save(&state).await
    }

    /// The device was forgotten at `gateway`: the tickets it issued are
    /// dead, and the servers that hold them are told anew with new ones.
    pub(super) async fn push_tickets_void(&self, gateway: &str) -> Result<()> {
        let mut back = self.push_take_back_state().await?;
        if back.tickets.iter().any(|h| h.gateway == gateway) {
            back.tickets.retain(|h| h.gateway != gateway);
            self.push_take_back_save(&back).await?;
        }
        let mut state = self.push_private_state().await?;
        let mut changed = false;
        for target in state.servers.values_mut() {
            if target.ticket.as_ref().is_some_and(|h| h.gateway == gateway) {
                target.ticket = None;
                changed = true;
            }
            target.retired.retain(|h| h.gateway != gateway);
        }
        if changed {
            self.push_private_save(&state).await?;
        }
        Ok(())
    }

    /// Pushes off, or the identity leaves: every private push server and
    /// every other gateway forgets the device, and the tickets are taken
    /// back.
    pub(super) async fn push_unregister_private(&self, keys: Option<&Keys>) -> Result<()> {
        let state = self.push_private_state().await?;
        if let Some(keys) = keys {
            for target in state.servers.values() {
                self.push_forget_target(target, keys).await;
            }
            for aux in state.gateways.values() {
                if let Some(told) = &aux.told {
                    self.push_forget(told, keys).await;
                }
            }
        }
        for id in state.servers.keys() {
            servers::set_push_seal(&self.store, id, None, None).await?;
        }
        settings::delete(&self.store, KEY_PRIVATE).await
    }

    /// The push of every private server whose push is on.
    pub(super) async fn push_private_status(&self) -> Result<Vec<PushTarget>> {
        let state = self.push_private_state().await?;
        let pubkey = self.session_keys().await.ok().map(|k| k.public_key().to_hex());
        let mut out = Vec::new();
        for (row, push, url) in self.push_private_wanted().await? {
            let target = state.servers.get(&row.id);
            let current = target
                .and_then(|t| t.told.as_ref())
                .filter(|t| t.server == url && pubkey.as_ref().is_none_or(|p| *p == t.pubkey));
            let error = target.and_then(|t| t.error.clone());
            let state = if error.is_some() {
                "failed"
            } else if current.is_some() {
                "registered"
            } else {
                "pending"
            };
            out.push(PushTarget {
                server_id: row.id.clone(),
                name: row.name.clone(),
                server: url,
                gateway: push.push_gateway.clone(),
                state: state.into(),
                error,
                last_ok_at: current.map(|t| t.at),
                expires_at: current.map(|t| t.answer.expires_at),
                ticket_expires_at: target.and_then(|t| t.ticket.as_ref()).map(|h| h.expires_at),
                relays: current.map(|t| t.answer.relays.clone()).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    // ─── The tickets, for the settings screen ──────────────────────────────

    /// The gateways this device takes tickets of: the home push server and
    /// every other one a private server of mine wakes through.
    async fn push_gateways(&self) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        if let (Some(home), _) = self.push_server().await? {
            out.push(home);
        }
        let state = self.push_private_state().await?;
        for gateway in state.gateways.keys() {
            if !out.contains(gateway) {
                out.push(gateway.clone());
            }
        }
        Ok(out)
    }

    /// Every live ticket of this identity at the gateways this device
    /// takes tickets of, each with the private server of this device that
    /// holds it. A gateway that cannot be asked is left out.
    pub async fn push_tickets_list(&self) -> Result<Vec<PushTicket>> {
        let keys = self.session_keys().await?;
        let state = self.push_private_state().await?;
        let mut out = Vec::new();
        for gateway in self.push_gateways().await? {
            let list = match VpushClient::new(&gateway)?.tickets(&keys).await {
                Ok(list) => list,
                // A server that issues no tickets answers 404.
                Err(PushError::Refused { status: 404, .. }) => continue,
                Err(e) => {
                    eprintln!("messenger push: the tickets of {gateway} were not read: {e}");
                    continue;
                }
            };
            for t in list.tickets {
                let server_id = state
                    .servers
                    .iter()
                    .find(|(_, s)| {
                        s.ticket
                            .as_ref()
                            .is_some_and(|h| h.gateway == gateway && h.ticket.starts_with(t.prefix()) && !t.prefix().is_empty())
                    })
                    .map(|(id, _)| id.clone());
                out.push(PushTicket {
                    gateway: gateway.clone(),
                    ticket: t.ticket,
                    device_id: t.device_id,
                    issued_at: t.issued_at,
                    expires_at: t.expires_at,
                    last_wake_at: t.last_wake_at,
                    server_id,
                });
            }
        }
        Ok(out)
    }

    /// Takes back the ticket that begins with `prefix` (its first 8 hex, or
    /// the whole of it) at every gateway this device takes tickets of. A
    /// private server of mine that held it is told anew with a new one at
    /// the next round. How many gateways took one back.
    pub async fn push_ticket_revoke(&self, prefix: &str) -> Result<usize> {
        let prefix = prefix.trim().trim_end_matches('…').to_ascii_lowercase();
        if prefix.len() < 8 || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(MessengerError::Invalid("a ticket is named by 8 hex at least".into()));
        }
        let keys = self.session_keys().await?;
        let mut taken = 0;
        for gateway in self.push_gateways().await? {
            match VpushClient::new(&gateway)?.ticket_delete(&keys, &prefix).await {
                Ok(()) => taken += 1,
                Err(PushError::Refused { status: 404, .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut state = self.push_private_state().await?;
        for target in state.servers.values_mut() {
            if target.ticket.as_ref().is_some_and(|h| h.ticket.starts_with(&prefix)) {
                target.ticket = None;
            }
        }
        self.push_private_save(&state).await?;
        Ok(taken)
    }
}

#[cfg(test)]
mod tests;

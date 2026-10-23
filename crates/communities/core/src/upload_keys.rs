// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys a write to a channel's own server needs (internal/communities-wire.md
//! §9.1): a server of the Veydan-Server kit with `MEDIA_UPLOAD_KEY=required`
//! takes an upload only with the app key of its card, or with an upload key
//! (`vu1.…`) the owner's device made from that app key for one member of the
//! channel's staff: upload-only, bound to the member's nostr key, revocable.
//!
//! `Book` holds both on this device, by host, and what each host said it
//! needs (`Gate`): the app keys come from the messenger's cards of servers
//! (`ChannelService::set_app_keys`), the upload keys from the keeper's notes
//! (`channel.upload_keys`). A host that needs a key this device has not is
//! never written to: it drops out of the upload of that blob.

use hmac_sha256::hmac;
use std::collections::HashMap;
use std::sync::RwLock;

/// The life of an upload key the keeper makes.
pub const TTL_SECS: i64 = 30 * 24 * 3600;
/// A key with less life than this left is made again.
pub const RENEW_SECS: i64 = 15 * 24 * 3600;
/// The longest life a server takes (the kit: 31 days).
pub const MAX_LIFE_SECS: i64 = 31 * 24 * 3600;
/// How long what a host said of its gate is believed.
pub const GATE_SECS: i64 = 600;
/// What a host that did not answer is believed to be, and for how long.
pub const GATE_UNKNOWN_SECS: i64 = 60;
/// Where a host of the kit says what it needs.
pub const INFO_PATH: &str = "/.well-known/veydan-media";
/// Where its owner revokes the upload keys of one member.
pub const REVOKE_PATH: &str = "/.well-known/veydan-media/revoke";
/// What a write says when no host of it takes a writer without the key of
/// its server, and this device has not the key.
pub const NEEDS_KEY: &str = "err.storage_needs_key";

pub(crate) mod hmac_sha256 {
    use sha2::{Digest, Sha256};

    /// HMAC-SHA256 (RFC 2104).
    pub(crate) fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..32].copy_from_slice(&Sha256::digest(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        inner.update(k.map(|b| b ^ 0x36));
        inner.update(msg);
        let mut outer = Sha256::new();
        outer.update(k.map(|b| b ^ 0x5c));
        outer.update(inner.finalize());
        outer.finalize().into()
    }
}

/// The name of the host of a base address, lower case, without the scheme,
/// the port and the path: what nginx calls `$host`.
pub fn host_name(base: &str) -> String {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let name = if authority.starts_with('[') {
        authority.split_once(']').map_or(authority, |(h, _)| h).trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    name.to_ascii_lowercase()
}

/// The label of a channel's keys: the first 32 hex of its id.
pub fn label_of(community_id: &str) -> String {
    community_id.chars().take(32).collect::<String>().to_ascii_lowercase()
}

/// An upload key for `pubkey` (hex) on `host` (a base address or a name),
/// valid from `iat` to `exp`, made with the server's app key.
pub fn mint(app_key: &str, host: &str, label: &str, pubkey: &str, iat: i64, exp: i64) -> String {
    format!("vu1.{label}.{pubkey}.{iat}.{exp}.{}", mac(app_key, &host_name(host), label, pubkey, iat, exp))
}

fn mac(app_key: &str, host: &str, label: &str, pubkey: &str, iat: i64, exp: i64) -> String {
    let msg = format!("veydan.media.upload.v1\n{host}\n{label}\n{pubkey}\n{iat}\n{exp}");
    hex::encode(hmac(app_key.as_bytes(), msg.as_bytes()))[..32].to_string()
}

/// The parts of an upload key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub label: String,
    pub pubkey: String,
    pub iat: i64,
    pub exp: i64,
    pub mac: String,
}

fn hex_of(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// An upload key read, its form checked (not its mac).
pub fn parse(key: &str) -> Option<Parsed> {
    let p: Vec<&str> = key.split('.').collect();
    let [v, label, pubkey, iat, exp, mac] = p.as_slice() else { return None };
    let num = |s: &str| (1..=10).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if *v != "vu1" || !hex_of(label, 1, 64) || !hex_of(pubkey, 64, 64) || !num(iat) || !num(exp) || !hex_of(mac, 32, 32) {
        return None;
    }
    Some(Parsed { label: label.to_string(), pubkey: pubkey.to_string(), iat: iat.parse().ok()?, exp: exp.parse().ok()?, mac: mac.to_string() })
}

/// What the server checks of an upload key (the kit's `upload.js`), but the
/// revocation: made with `app_key` for `host` and the signer `pubkey`,
/// alive at `now`. For the hosts of the tests.
pub fn verify(app_key: &str, host: &str, key: &str, pubkey: &str, now: i64) -> bool {
    let Some(p) = parse(key) else { return false };
    p.pubkey == pubkey
        && p.mac == mac(app_key, &host_name(host), &p.label, &p.pubkey, p.iat, p.exp)
        && now < p.exp
        && p.iat < p.exp
        && p.exp - p.iat <= MAX_LIFE_SECS
        && p.iat <= now + 300
}

/// What a host needs of a writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Any nostr key (a host of Veydan Cloud, a free host, a kit with `open`).
    Open,
    /// The server's key; `scoped`: it takes upload keys too (kit 0.1.8+).
    Key { scoped: bool },
}

impl Gate {
    /// What a host answered at `INFO_PATH`: the JSON of the kit, 401 of an
    /// older kit that needs the key; anything else is a host that needs none.
    pub fn from_info(body: &[u8]) -> Self {
        let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        match v.get("upload").and_then(|u| u.as_str()) {
            Some("key") => Self::Key { scoped: v.get("scoped").and_then(serde_json::Value::as_i64) == Some(1) },
            _ => Self::Open,
        }
    }
}

/// An upload key this device was given, for its user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given {
    pub community_id: String,
    pub host: String,
    pub key: String,
    pub expires_at: i64,
}

/// The keys of this device for the hosts, and what the hosts need.
#[derive(Default)]
pub struct Book {
    /// App keys by host name (the cards of servers of this device's user).
    app: RwLock<HashMap<String, String>>,
    /// Upload keys given to this device's user, by host name.
    given: RwLock<HashMap<String, Vec<Given>>>,
    /// What each host said it needs, and until when that is believed.
    gates: RwLock<HashMap<String, (Gate, i64)>>,
    /// The keys of channels given to this device's user for the owner's
    /// Veydan Cloud (wire §9.2), by channel, with their end.
    cloud: RwLock<HashMap<String, (nostr::key::Keys, i64)>>,
}

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

impl Book {
    /// The app keys of the cards of servers, `(base address, key)`: all of them.
    pub fn set_app_keys(&self, keys: &[(String, String)]) {
        let map = keys.iter().filter(|(_, k)| !k.trim().is_empty()).map(|(h, k)| (host_name(h), k.trim().to_string())).collect();
        *write(&self.app) = map;
    }

    /// The app key of `host`, when this device has its card.
    pub fn app_key(&self, host: &str) -> Option<String> {
        read(&self.app).get(&host_name(host)).cloned()
    }

    /// The upload keys given to this device's user: all of them.
    pub fn set_given(&self, keys: Vec<Given>) {
        let mut map: HashMap<String, Vec<Given>> = HashMap::new();
        for g in keys {
            map.entry(host_name(&g.host)).or_default().push(g);
        }
        *write(&self.given) = map;
    }

    /// The key a write by `signer` (hex) to `host` carries at `now`: the app
    /// key of its card, else an upload key given for this signer, the one
    /// that lives longest (of the channel named first, when there is one).
    pub fn key_for(&self, host: &str, signer: &str, community_id: Option<&str>, now: i64) -> Option<String> {
        if let Some(k) = self.app_key(host) {
            return Some(k);
        }
        let given = read(&self.given);
        let alive = given.get(&host_name(host))?.iter().filter(|g| g.expires_at > now && parse(&g.key).is_some_and(|p| p.pubkey == signer));
        let mut best: Option<&Given> = None;
        for g in alive {
            let rank = |x: &Given| (community_id == Some(x.community_id.as_str()), x.expires_at);
            if best.is_none_or(|b| rank(g) > rank(b)) {
                best = Some(g);
            }
        }
        best.map(|g| g.key.clone())
    }

    /// The keys of channels for the owners' Cloud given to this device's
    /// user (wire §9.2): all of them.
    pub fn set_cloud(&self, keys: HashMap<String, (nostr::key::Keys, i64)>) {
        *write(&self.cloud) = keys;
    }

    /// The key of `community_id` for its owner's Cloud, alive at `now`.
    pub fn cloud_key(&self, community_id: &str, now: i64) -> Option<nostr::key::Keys> {
        read(&self.cloud).get(community_id).filter(|(_, until)| *until > now).map(|(k, _)| k.clone())
    }

    /// Every key of a channel for its owner's Cloud alive at `now`.
    pub fn cloud_keys(&self, now: i64) -> Vec<nostr::key::Keys> {
        read(&self.cloud).values().filter(|(_, until)| *until > now).map(|(k, _)| k.clone()).collect()
    }

    /// The key of `community_id` no longer uploads (the server said so).
    pub fn drop_cloud(&self, community_id: &str) {
        write(&self.cloud).remove(community_id);
    }

    /// What `host` needs, when it said so lately.
    pub fn gate(&self, host: &str, now: i64) -> Option<Gate> {
        read(&self.gates).get(&host_name(host)).filter(|(_, until)| *until > now).map(|(g, _)| *g)
    }

    pub fn set_gate(&self, host: &str, gate: Gate, until: i64) {
        write(&self.gates).insert(host_name(host), (gate, until));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PK: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    /// The vector of wire §9.1, the same the kit checks (scripts/tests/server.tests.mjs).
    #[test]
    fn the_key_of_the_vector_is_the_kits() {
        let key = mint("test-app-key", "https://Media.Example.org:443/", "00112233445566778899aabbccddeeff", PK, 1_760_000_000, 1_762_592_000);
        assert_eq!(key, format!("vu1.00112233445566778899aabbccddeeff.{PK}.1760000000.1762592000.5053c7a51071f325ff14242cabc90eda"));
        assert!(verify("test-app-key", "media.example.org", &key, PK, 1_760_000_100));
        assert!(!verify("test-app-key", "other.example.org", &key, PK, 1_760_000_100), "another host");
        assert!(!verify("other", "media.example.org", &key, PK, 1_760_000_100), "another app key");
        assert!(!verify("test-app-key", "media.example.org", &key, &"0".repeat(64), 1_760_000_100), "another signer");
        assert!(!verify("test-app-key", "media.example.org", &key, PK, 1_762_592_000), "expired");
        // RFC 4231, case 2.
        assert_eq!(hex::encode(hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn a_host_is_named_as_nginx_names_it() {
        assert_eq!(host_name("https://User-Media.veydan.net"), "user-media.veydan.net");
        assert_eq!(host_name("https://m.example.org:8443/x"), "m.example.org");
        assert_eq!(host_name("m.example.org"), "m.example.org");
        assert_eq!(label_of(&"AB".repeat(32)), "ab".repeat(16));
    }

    #[test]
    fn a_kit_says_what_it_needs() {
        assert_eq!(Gate::from_info(br#"{"v":1,"upload":"key","scoped":1}"#), Gate::Key { scoped: true });
        assert_eq!(Gate::from_info(br#"{"v":1,"upload":"key"}"#), Gate::Key { scoped: false });
        assert_eq!(Gate::from_info(br#"{"v":1,"upload":"open"}"#), Gate::Open);
        assert_eq!(Gate::from_info(b"<html>"), Gate::Open);
    }

    #[test]
    fn the_book_gives_the_app_key_first_then_the_given_key_of_the_signer() {
        let b = Book::default();
        let k1 = mint("app", "https://m.example.org", "aa", PK, 100, 1000);
        let k2 = mint("app", "https://m.example.org", "bb", PK, 100, 2000);
        b.set_given(vec![
            Given { community_id: "aa".into(), host: "https://m.example.org".into(), key: k1.clone(), expires_at: 1000 },
            Given { community_id: "bb".into(), host: "https://m.example.org".into(), key: k2.clone(), expires_at: 2000 },
        ]);
        assert_eq!(b.key_for("https://m.example.org", PK, None, 200), Some(k2.clone()), "the longest alive");
        assert_eq!(b.key_for("https://m.example.org", PK, Some("aa"), 200), Some(k1), "the channel's own first");
        assert_eq!(b.key_for("https://m.example.org", PK, Some("aa"), 1500), Some(k2), "an expired one never");
        assert_eq!(b.key_for("https://m.example.org", &"0".repeat(64), None, 200), None, "another signer has none");
        b.set_app_keys(&[("https://M.example.org".into(), "app".into())]);
        assert_eq!(b.key_for("https://m.example.org", &"0".repeat(64), None, 200), Some("app".into()));
        assert_eq!(b.gate("https://m.example.org", 0), None);
        b.set_gate("https://m.example.org", Gate::Key { scoped: true }, 10);
        assert_eq!(b.gate("m.example.org", 5), Some(Gate::Key { scoped: true }));
        assert_eq!(b.gate("m.example.org", 10), None, "said too long ago");
    }
}

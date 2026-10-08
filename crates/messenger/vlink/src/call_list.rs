// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The list of call nodes a registry gives a client (`GET
//! <registry>/v1/calls`, the registry of the project), and the check of its
//! signatures.
//!
//! The envelope is the one of the lists of bridges ([`SignedList`]): the
//! root delegates a list key, the list key signs. The context differs,
//! `vcall-list/1` instead of `vlink-list/1`, so a signature made for a
//! list of bridges is never one for a list of nodes and the other way
//! round; and the list says what it is (`kind: "call"`).
//!
//! Two versions are read: 1 (the first registries: active nodes, the
//! load in percent) and 2 (degraded nodes too, each with its `state` and
//! `class`, the load as a share of 1). [`ListedNode::load_percent`]
//! gives the load the same way for both.
//!
//! The types are spelled again here rather than taken from the hub or
//! the node: the messenger takes no crate from `services/` but the client
//! of VLink (scripts/boundaries.sh). A client only checks; the signing is
//! here for the fakes of the tests (`messenger-testkit`), with the same
//! rules the hub signs by.

use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use vlink_proto::list::{hex, unhex, Delegation, Error, SignedList};
use vlink_proto::BridgeRef;

const LIST_CONTEXT: &str = "vcall-list/1\n";

/// The versions of [`CallList`] this client reads.
pub const VERSION_MIN: u32 = 1;
pub const VERSION: u32 = 2;
/// What a [`CallList`] lists.
pub const KIND: &str = "call";
/// The class of every node of the registry: anybody may run one.
pub const CLASS_VOLUNTEER: &str = "volunteer";
/// The states a list names (version 2); a node of version 1 is active.
pub const STATE_ACTIVE: &str = "active";
pub const STATE_DEGRADED: &str = "degraded";

/// One node of the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListedNode {
    /// `address:control_port#id`: what the client pins.
    #[serde(rename = "ref")]
    pub node: BridgeRef,
    pub turn_port: u16,
    /// 0: no SFU.
    #[serde(default)]
    pub sfu_port: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub region: String,
    /// What the node reported, as far as the registry's dial-back agreed.
    #[serde(default)]
    pub caps: Vec<String>,
    /// [`CLASS_VOLUNTEER`] in a list of the registry.
    #[serde(default = "volunteer")]
    pub class: String,
    /// `active`, or `degraded` (the last check failed). Absent in
    /// version 1: active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// How full the node said it was when the list was made: a share of
    /// 1 in version 2, percent in version 1.
    #[serde(default)]
    pub load: f64,
}

fn volunteer() -> String {
    CLASS_VOLUNTEER.to_string()
}

impl ListedNode {
    /// The load in percent, whatever the version wrote.
    pub fn load_percent(&self, version: u32) -> u8 {
        let share = if version == 1 { self.load / 100.0 } else { self.load };
        (share.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// The state as named, `active` when the list named none.
    pub fn state(&self) -> &str {
        self.state.as_deref().unwrap_or(STATE_ACTIVE)
    }
}

/// The call nodes a client is given, the least loaded first, and until
/// when the list may be used. Times are unix seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallList {
    pub v: u32,
    pub kind: String,
    /// Every node the registry gives out, rather than a few for a client.
    #[serde(default)]
    pub complete: bool,
    pub issued_at: u64,
    pub expires_at: u64,
    pub nodes: Vec<ListedNode>,
}

/// The bytes the list key signs.
pub fn list_message(list: &str) -> Vec<u8> {
    format!("{LIST_CONTEXT}{list}").into_bytes()
}

/// As a client: the list, when the root delegated the key that signed
/// it, it is a list of call nodes of a version known, and nothing ran
/// out at `now`.
pub fn verify(signed: &SignedList, root_hex: &str, now: u64) -> Result<CallList, Error> {
    signed.delegation.verify(root_hex, now)?;
    let key = unhex(&signed.delegation.key)?;
    UnparsedPublicKey::new(&ED25519, &key)
        .verify(&list_message(&signed.list), &unhex(&signed.sig)?)
        .map_err(|_| Error::Signature)?;
    let list: CallList = serde_json::from_str(&signed.list).map_err(|e| Error::Malformed(e.to_string()))?;
    if !(VERSION_MIN..=VERSION).contains(&list.v) {
        return Err(Error::Version(list.v));
    }
    if list.kind != KIND {
        return Err(Error::Malformed(format!("a list of {}, not of call nodes", list.kind)));
    }
    if now >= list.expires_at {
        return Err(Error::Expired);
    }
    Ok(list)
}

/// As a registry: signs `list` with the list key `pkcs8` under
/// `delegation`. For the fakes of the tests; the real registry is the hub.
pub fn sign(pkcs8: &[u8], list: &CallList, delegation: &Delegation) -> Result<SignedList, Error> {
    let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(pkcs8).map_err(|e| Error::Malformed(format!("not an Ed25519 key: {e}")))?;
    let text = serde_json::to_string(list).expect("a list is plain data");
    let sig = key.sign(&list_message(&text));
    Ok(SignedList { list: text, sig: hex(sig.as_ref()), delegation: delegation.clone() })
}

/// A fresh Ed25519 key as PKCS#8 DER, for a fake root or list key of a
/// test; its public half as hex comes from [`public_hex`].
pub fn generate_pkcs8() -> Vec<u8> {
    Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).expect("the system gives random bytes").as_ref().to_vec()
}

/// The public half of the key `pkcs8`, 32 bytes as hex.
pub fn public_hex(pkcs8: &[u8]) -> Result<String, Error> {
    let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(pkcs8).map_err(|e| Error::Malformed(format!("not an Ed25519 key: {e}")))?;
    Ok(hex(key.public_key().as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlink_proto::list::{BridgeList, VERSION as BRIDGE_LIST_VERSION};
    use vlink_proto::sign::Signer;
    use vlink_proto::BridgeId;

    fn list() -> CallList {
        let id = BridgeId::of_key(b"n");
        CallList {
            v: VERSION,
            kind: KIND.into(),
            complete: false,
            issued_at: 100,
            expires_at: 5000,
            nodes: vec![ListedNode {
                node: format!("203.0.113.7:8443#{id}").parse().unwrap(),
                turn_port: 3478,
                sfu_port: 3479,
                region: "eu".into(),
                caps: vec!["stun".into(), "sfu".into()],
                class: CLASS_VOLUNTEER.into(),
                state: Some(STATE_DEGRADED.into()),
                load: 0.3,
            }],
        }
    }

    #[test]
    fn a_list_is_taken_under_the_root_and_never_for_a_bridge_list() {
        let (root_der, list_der) = (generate_pkcs8(), generate_pkcs8());
        let root = Signer::from_pkcs8(&root_der).unwrap();
        let delegation = root.delegate(&public_hex(&list_der).unwrap(), 10_000);
        let signed = sign(&list_der, &list(), &delegation).unwrap();
        // It survives the trip as JSON.
        let wire: SignedList = serde_json::from_str(&serde_json::to_string(&signed).unwrap()).unwrap();
        let read = verify(&wire, &root.public_hex(), 200).unwrap();
        assert_eq!(read, list());
        assert_eq!((read.nodes[0].load_percent(read.v), read.nodes[0].state()), (30, STATE_DEGRADED));
        assert_eq!(verify(&wire, &root.public_hex(), 5000), Err(Error::Expired));
        assert_eq!(verify(&wire, &"00".repeat(32), 200), Err(Error::Delegation));

        // The same key and delegation signing a list of bridges: no list
        // of nodes; and the list of nodes is no list of bridges.
        let bridges = BridgeList { v: BRIDGE_LIST_VERSION, issued_at: 100, expires_at: 5000, bridges: vec![] };
        let as_bridges = Signer::from_pkcs8(&list_der).unwrap().sign_list(&bridges, &delegation);
        assert_eq!(verify(&as_bridges, &root.public_hex(), 200), Err(Error::Signature));
        assert_eq!(signed.verify(&root.public_hex(), 200), Err(Error::Signature));

        // A changed list, a list of another kind, a version not known.
        let mut changed = signed.clone();
        changed.list = changed.list.replace("3478", "3479");
        assert_eq!(verify(&changed, &root.public_hex(), 200), Err(Error::Signature));
        let mut other = list();
        other.kind = "bridge".into();
        let signed = sign(&list_der, &other, &delegation).unwrap();
        assert!(matches!(verify(&signed, &root.public_hex(), 200), Err(Error::Malformed(_))));
        let mut newer = list();
        newer.v = VERSION + 1;
        assert_eq!(verify(&sign(&list_der, &newer, &delegation).unwrap(), &root.public_hex(), 200), Err(Error::Version(VERSION + 1)));
    }

    #[test]
    fn a_list_of_the_first_version_is_read_as_the_first_registries_wrote_it() {
        let (root_der, list_der) = (generate_pkcs8(), generate_pkcs8());
        let root = Signer::from_pkcs8(&root_der).unwrap();
        let delegation = root.delegate(&public_hex(&list_der).unwrap(), 10_000);
        let id = BridgeId::of_key(b"n");
        // Version 1: no class, no state, the load in percent.
        let text = format!(
            r#"{{"v":1,"kind":"call","issued_at":100,"expires_at":5000,"nodes":[{{"ref":"203.0.113.7:8443#{id}","turn_port":3478,"caps":["stun"],"load":30}}]}}"#
        );
        let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&list_der).unwrap();
        let signed = SignedList { sig: hex(key.sign(&list_message(&text)).as_ref()), list: text, delegation };
        let read = verify(&signed, &root.public_hex(), 200).unwrap();
        let n = &read.nodes[0];
        assert_eq!((n.load_percent(read.v), n.state(), n.class.as_str(), n.sfu_port, read.complete), (30, STATE_ACTIVE, CLASS_VOLUNTEER, 0, false));
    }
}

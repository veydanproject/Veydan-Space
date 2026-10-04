// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The link of a VLink bridge: `veydan://vlink/<id>?a=<address:port>`.
//!
//! A bridge is a machine through which the project's servers are reached
//! where the direct way is throttled. Its id is the hash of the
//! certificate it shows; the address is where it listens. Whoever runs a
//! bridge of their own gives this link to the people it is for.
//!
//! The type is `vlink`, not `bridge`: that word is kept for bridges to
//! other networks, and `veydan://bridge/…` is a type this version does
//! not know (`LinkError::Type` here, an `unknown` link to a card).

use crate::error::LinkError;
use crate::kind::LinkType;
use crate::uri::{Uri, UriBuilder};
use std::net::SocketAddr;

const ID_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeLink {
    /// 64 hex characters, lower case.
    pub id: String,
    pub addr: SocketAddr,
}

impl BridgeLink {
    pub fn encode(&self) -> String {
        UriBuilder::new(&LinkType::Bridge, &self.id).param("a", &self.addr.to_string()).build()
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::Bridge {
            return Err(LinkError::Type);
        }
        let id = uri.id().to_ascii_lowercase();
        if id.len() != ID_LEN || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(LinkError::Id);
        }
        // A bridge is called by address: a name here would be looked up,
        // and the lookup is one more thing that can be tampered with.
        let addr = uri.text("a")?.and_then(|a| a.parse().ok()).ok_or(LinkError::Param)?;
        Ok(Self { id, addr })
    }

    /// As the bridge's own tools write it: `address:port#id`.
    pub fn reference(&self) -> String {
        format!("{}#{}", self.addr, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "9c5f434def40ebc2879201642bab6c69063578c4a302aa92de5918b3663ffbfc";

    #[test]
    fn a_link_as_the_bridge_prints_it_is_read() {
        // What `vlink link --addr …` writes.
        let link = format!("veydan://vlink/{ID}?a=45.93.201.244%3A443");
        let bridge = BridgeLink::parse(&link).unwrap();
        assert_eq!(bridge.addr.to_string(), "45.93.201.244:443");
        assert_eq!(bridge.reference(), format!("45.93.201.244:443#{ID}"));
        assert_eq!(bridge.encode(), link);

        let v6 = format!("veydan://vlink/{ID}?a=%5B2001%3Adb8%3A%3A1%5D%3A8443");
        assert_eq!(BridgeLink::parse(&v6).unwrap().addr.to_string(), "[2001:db8::1]:8443");
        assert_eq!(BridgeLink::parse(&v6).unwrap().encode(), v6);
    }

    #[test]
    fn what_is_not_a_bridge_is_refused() {
        // Another type.
        assert_eq!(BridgeLink::parse(&format!("veydan://contact/{ID}?a=1.2.3.4%3A443")), Err(LinkError::Type));
        // An id that is no hash.
        assert_eq!(BridgeLink::parse("veydan://vlink/abc?a=1.2.3.4%3A443"), Err(LinkError::Id));
        assert_eq!(BridgeLink::parse(&format!("veydan://vlink/{}?a=1.2.3.4%3A443", "z".repeat(64))), Err(LinkError::Id));
        // No address, a name instead of an address, no port.
        assert_eq!(BridgeLink::parse(&format!("veydan://vlink/{ID}")), Err(LinkError::Param));
        assert_eq!(BridgeLink::parse(&format!("veydan://vlink/{ID}?a=example.org%3A443")), Err(LinkError::Param));
        assert_eq!(BridgeLink::parse(&format!("veydan://vlink/{ID}?a=1.2.3.4")), Err(LinkError::Param));
    }

    #[test]
    fn an_upper_case_id_is_the_same_bridge() {
        let link = format!("veydan://vlink/{}?a=1.2.3.4%3A443", ID.to_uppercase());
        assert_eq!(BridgeLink::parse(&link).unwrap().id, ID);
        assert_eq!(BridgeLink::parse(&link).unwrap().encode(), format!("veydan://vlink/{ID}?a=1.2.3.4%3A443"));
    }

    #[test]
    fn a_bridge_is_written_as_a_vlink_link() {
        let v4 = BridgeLink { id: ID.into(), addr: "203.0.113.7:443".parse().unwrap() };
        assert_eq!(v4.encode(), format!("veydan://vlink/{ID}?a=203.0.113.7%3A443"));
        let v6 = BridgeLink { id: ID.into(), addr: "[2001:db8::1]:8443".parse().unwrap() };
        assert_eq!(v6.encode(), format!("veydan://vlink/{ID}?a=%5B2001%3Adb8%3A%3A1%5D%3A8443"));
        assert_eq!(BridgeLink::parse(&v6.encode()).unwrap(), v6);
    }

    #[test]
    fn the_word_bridge_is_no_vlink_link() {
        // Kept for bridges to other networks; never released, so no alias.
        let old = format!("veydan://bridge/{ID}?a=45.93.201.244%3A443");
        assert_eq!(BridgeLink::parse(&old), Err(LinkError::Type));
        assert_eq!(Uri::parse(&old).unwrap().link_type(), &LinkType::Unknown("bridge".into()));
    }
}

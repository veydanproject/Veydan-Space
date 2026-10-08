// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The link of a private call node:
//! `veydan://call-node/<id>?a=<address:port>&t=<token>`.
//!
//! Whoever runs a private node (`VCALL_PRIVATE=true`) makes an
//! invitation on it (`vcall ctl invite`) and gives the link to the
//! people the node is for. The id is the hash of the node's key, what
//! TLS to it is pinned to; the address is where its control channel
//! listens; the token is the invitation, good a limited number of times
//! for a limited while. The app exchanges the token at the node for the
//! credentials of this device (`messenger-calls`, `redeem_invite`) and
//! keeps those; the link itself is kept nowhere.
//!
//! Without a token the link names the node alone (`a` only): what an
//! owner writes for a node that takes no invitations, for instance one
//! whose access key is given by hand. The token is then `None`.

use crate::error::LinkError;
use crate::kind::LinkType;
use crate::uri::{Uri, UriBuilder};
use std::net::SocketAddr;

const ID_LEN: usize = 64;
/// A token is a short word the node made; anything longer is not one.
const MAX_TOKEN_LEN: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallNodeLink {
    /// 64 hex characters, lower case.
    pub id: String,
    pub addr: SocketAddr,
    /// The invitation, when the link carries one.
    pub token: Option<String>,
}

impl CallNodeLink {
    pub fn encode(&self) -> String {
        let mut b = UriBuilder::new(&LinkType::CallNode, &self.id).param("a", &self.addr.to_string());
        if let Some(t) = &self.token {
            b = b.param("t", t);
        }
        b.build()
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::CallNode {
            return Err(LinkError::Type);
        }
        let id = uri.id().to_ascii_lowercase();
        if id.len() != ID_LEN || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(LinkError::Id);
        }
        // A node is called by address, as a bridge is: a name would be
        // looked up, and the lookup is one more thing to tamper with.
        let addr: SocketAddr = uri.text("a")?.and_then(|a| a.parse().ok()).ok_or(LinkError::Param)?;
        if addr.port() == 0 {
            return Err(LinkError::Param);
        }
        let token = match uri.text("t")? {
            Some(t) if t.is_empty() || t.len() > MAX_TOKEN_LEN || t.chars().any(|c| c.is_control() || c.is_whitespace()) => {
                return Err(LinkError::Param)
            }
            other => other,
        };
        Ok(Self { id, addr, token })
    }

    /// As the node's own tools write it: `address:port#id`.
    pub fn reference(&self) -> String {
        format!("{}#{}", self.addr, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca";

    #[test]
    fn a_link_as_the_node_prints_it_is_read() {
        // What `vcall ctl invite` writes.
        let link = format!("veydan://call-node/{ID}?a=203.0.113.7%3A8443&t=inv-7f3a9c");
        let node = CallNodeLink::parse(&link).unwrap();
        assert_eq!(node.addr.to_string(), "203.0.113.7:8443");
        assert_eq!(node.token.as_deref(), Some("inv-7f3a9c"));
        assert_eq!(node.reference(), format!("203.0.113.7:8443#{ID}"));
        assert_eq!(node.encode(), link);

        let v6 = format!("veydan://call-node/{ID}?a=%5B2001%3Adb8%3A%3A1%5D%3A443&t=x");
        assert_eq!(CallNodeLink::parse(&v6).unwrap().addr.to_string(), "[2001:db8::1]:443");
        assert_eq!(CallNodeLink::parse(&v6).unwrap().encode(), v6);

        // Without a token: the node alone.
        let bare = format!("veydan://call-node/{}?a=203.0.113.7%3A8443", ID.to_uppercase());
        let node = CallNodeLink::parse(&bare).unwrap();
        assert_eq!(node.token, None);
        assert_eq!(node.id, ID, "an upper-case id is the same node");
    }

    #[test]
    fn what_is_not_a_call_node_is_refused() {
        assert_eq!(CallNodeLink::parse(&format!("veydan://vlink/{ID}?a=1.2.3.4%3A443")), Err(LinkError::Type));
        assert_eq!(CallNodeLink::parse("veydan://call-node/abc?a=1.2.3.4%3A443"), Err(LinkError::Id));
        assert_eq!(CallNodeLink::parse(&format!("veydan://call-node/{ID}")), Err(LinkError::Param));
        assert_eq!(CallNodeLink::parse(&format!("veydan://call-node/{ID}?a=example.org%3A443")), Err(LinkError::Param));
        assert_eq!(CallNodeLink::parse(&format!("veydan://call-node/{ID}?a=1.2.3.4%3A0&t=x")), Err(LinkError::Param));
        assert_eq!(CallNodeLink::parse(&format!("veydan://call-node/{ID}?a=1.2.3.4%3A443&t=")), Err(LinkError::Param));
        assert_eq!(CallNodeLink::parse(&format!("veydan://call-node/{ID}?a=1.2.3.4%3A443&t={}", "x".repeat(200))), Err(LinkError::Param));
        // The grammar: a hyphen joins two words and nothing else.
        assert_eq!(Uri::parse(&format!("veydan://call--node/{ID}")).unwrap_err(), LinkError::Type);
        assert_eq!(Uri::parse(&format!("veydan://-call/{ID}")).unwrap_err(), LinkError::Type);
        assert_eq!(Uri::parse(&format!("veydan://call-/{ID}")).unwrap_err(), LinkError::Type);
    }
}

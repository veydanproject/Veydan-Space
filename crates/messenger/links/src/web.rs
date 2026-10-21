// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The link of a sign-in to the web cabinet of Veydan Cloud
//! (services/cloud/spec/cloud-api.md §16.2): `veydan://web/<id>`.
//!
//! The cabinet draws it as a QR code on its sign-in page; the app reads it
//! with its own scanner or from a paste, shows where the request comes from
//! and approves it with the identity. The id is the 16 hex of the pending
//! sign-in and nothing else: no parameter, so a page cannot write a word of
//! its own onto the screen of approval. The app takes such a link only from
//! its scanner or a paste (no deep link of the OS opens it).

use crate::error::LinkError;
use crate::kind::LinkType;
use crate::uri::{Uri, UriBuilder};

/// The characters of the id.
const ID_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebLink {
    /// 16 hex characters, lower case: the sign-in waiting for approval.
    pub id: String,
}

/// Whether `id` is the id of a sign-in: 16 hex in lower case, as vcloud makes it.
pub fn is_web_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl WebLink {
    pub fn encode(&self) -> String {
        UriBuilder::new(&LinkType::Web, &self.id).build()
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    /// Parameters are skipped as the grammar says (a newer cabinet may add
    /// one); none of them is read or shown.
    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::Web {
            return Err(LinkError::Type);
        }
        if !is_web_id(uri.id()) {
            return Err(LinkError::Id);
        }
        Ok(Self {
            id: uri.id().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef";

    #[test]
    fn a_link_is_written_and_read() {
        let link = WebLink { id: ID.into() };
        assert_eq!(link.encode(), format!("veydan://web/{ID}"));
        assert_eq!(WebLink::parse(&link.encode()).unwrap(), link);
        // A parameter of a newer cabinet is skipped, not shown.
        assert_eq!(
            WebLink::parse(&format!("veydan://web/{ID}?n=Pay%20here")).unwrap(),
            link
        );
    }

    #[test]
    fn what_is_not_a_web_link_is_refused() {
        assert_eq!(
            WebLink::parse(&format!("veydan://contact/{ID}")),
            Err(LinkError::Type)
        );
        for bad in [
            "0123456789ABCDEF",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789abcdeg",
            "my.veydan.net",
        ] {
            assert_eq!(
                WebLink::parse(&format!("veydan://web/{bad}")),
                Err(LinkError::Id),
                "{bad}"
            );
        }
        assert_eq!(
            WebLink::parse(&format!("https://my.veydan.net/{ID}")),
            Err(LinkError::Scheme)
        );
    }
}

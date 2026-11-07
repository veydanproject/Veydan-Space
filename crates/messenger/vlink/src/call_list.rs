// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The list of call nodes a registry gives a client (`GET
//! <registry>/v1/calls`, the registry of the project), and the check of its
//! signatures.
//!
//! The form is not spelled here: it is `registry_proto::call` (Veydan
//! Common, `services/common`), the one copy the registry and the node read
//! too, given under the names the messenger uses. The envelope is the one
//! of the lists of bridges ([`SignedList`]): the root delegates a list
//! key, the list key signs, under a context of its own (`vcall-list/1`),
//! and the list says what it is (`kind: "call"`). Versions 1 and 2 are
//! read; [`ListedNode::load_percent`] gives the load the same way for both.
//!
//! A client only checks; the signing is here for the fakes of the tests
//! (`messenger-testkit`), by the same rules the hub signs by.

pub use registry_proto::call::{
    list_message, verify_list as verify, CallList, CallNode as ListedNode, State, CLASS_VOLUNTEER, KIND,
    LIST_VERSION as VERSION, LIST_VERSION_MIN as VERSION_MIN,
};

use registry_proto::sign::{Signer, SignerExt};
use vlink_proto::list::{Delegation, Error, SignedList};

/// As a registry: signs `list` with the list key `pkcs8` under
/// `delegation`. For the fakes of the tests; the real registry is the hub.
pub fn sign(pkcs8: &[u8], list: &CallList, delegation: &Delegation) -> Result<SignedList, Error> {
    Ok(Signer::from_pkcs8(pkcs8)?.sign_call_list(list, delegation))
}

/// A fresh Ed25519 key as PKCS#8 DER, for a fake root or list key of a
/// test; its public half as hex comes from [`public_hex`].
pub fn generate_pkcs8() -> Vec<u8> {
    Signer::generate_pkcs8()
}

/// The public half of the key `pkcs8`, 32 bytes as hex.
pub fn public_hex(pkcs8: &[u8]) -> Result<String, Error> {
    Ok(Signer::from_pkcs8(pkcs8)?.public_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fakes_sign_what_a_client_takes() {
        let (root_der, list_der) = (generate_pkcs8(), generate_pkcs8());
        let root = Signer::from_pkcs8(&root_der).unwrap();
        let delegation = root.delegate(&public_hex(&list_der).unwrap(), 10_000);
        let list = CallList { v: VERSION, kind: KIND.into(), complete: false, issued_at: 100, expires_at: 5000, nodes: vec![] };
        let signed = sign(&list_der, &list, &delegation).unwrap();
        assert_eq!(verify(&signed, &root.public_hex(), 200), Ok(list));
        assert!(sign(b"not a key", &CallList { v: VERSION, kind: KIND.into(), complete: false, issued_at: 0, expires_at: 0, nodes: vec![] }, &delegation).is_err());
    }
}

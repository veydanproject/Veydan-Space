// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! NIP-98 as the requests of the cloud carry it (cloud-api.md 2.3): the
//! header of an event, and the event of a temporary key T. Requests of the
//! identity are signed by the messenger's `Signer`; those of a procedure of
//! recovery by its key T, which the module holds itself.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use secp256k1::{Keypair, Secp256k1};
use sha2::{Digest, Sha256};
use veydan_shell::seams::{code, CloudError, TempKey, WireEvent};
use zeroize::Zeroizing;

/// The kind of an HTTP auth event (NIP-98).
pub const KIND_HTTP_AUTH: u16 = 27235;

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `Authorization: Nostr <base64 of the event's JSON>`.
pub fn authorization(event: &WireEvent) -> Result<String, CloudError> {
    let json = serde_json::to_vec(event).map_err(crypto)?;
    Ok(format!("Nostr {}", STANDARD.encode(json)))
}

/// The sha256 of a body, for the tag `payload`; none for an empty body.
pub fn payload_hash(body: &[u8]) -> Option<[u8; 32]> {
    (!body.is_empty()).then(|| Sha256::digest(body).into())
}

fn keypair(t: &TempKey) -> Result<Keypair, CloudError> {
    let secp = Secp256k1::signing_only();
    Keypair::from_seckey_slice(&secp, t.secret_bytes())
        .map_err(|_| crypto("the temporary key is not a key of secp256k1"))
}

/// The public key of T as the contract writes it: x-only, 64 hex.
pub fn temp_pub(t: &TempKey) -> Result<String, CloudError> {
    let mut pair = keypair(t)?;
    let (xonly, _) = pair.x_only_public_key();
    pair.non_secure_erase();
    Ok(hex(&xonly.serialize()))
}

/// The event of NIP-98 signed by T at `created_at`: kind 27235, `u`
/// exactly `url`, `method` in capitals, `payload` when there is a body.
pub fn event_by(
    t: &TempKey,
    method: &str,
    url: &str,
    payload_sha256: Option<[u8; 32]>,
    created_at: u64,
) -> Result<WireEvent, CloudError> {
    let mut tags = vec![
        vec!["u".to_string(), url.to_string()],
        vec!["method".to_string(), method.to_ascii_uppercase()],
    ];
    if let Some(hash) = payload_sha256 {
        tags.push(vec!["payload".to_string(), hex(&hash)]);
    }
    let pubkey = temp_pub(t)?;
    // NIP-01: the id is the sha256 of [0, pubkey, created_at, kind, tags, content].
    let serial = serde_json::json!([0, pubkey, created_at, KIND_HTTP_AUTH, tags, ""]);
    let id: [u8; 32] = Sha256::digest(serial.to_string().as_bytes()).into();
    let mut aux = Zeroizing::new([0u8; 32]);
    getrandom::fill(aux.as_mut_slice()).map_err(crypto)?;
    let mut pair = keypair(t)?;
    let secp = Secp256k1::signing_only();
    let sig = secp.sign_schnorr_with_aux_rand(&id, &pair, &aux);
    pair.non_secure_erase();
    Ok(WireEvent {
        id: hex(&id),
        pubkey,
        created_at,
        kind: KIND_HTTP_AUTH,
        tags,
        content: String::new(),
        sig: hex(sig.as_ref()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{schnorr::Signature, XOnlyPublicKey};

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
    }

    /// The event verifies as a server verifies it: the id from the NIP-01
    /// serialisation, the Schnorr signature by the x-only key.
    #[test]
    fn an_event_of_t_is_signed_and_verifies() {
        let t = TempKey::generate().expect("key");
        let url = "https://eu-1-media.example.com/cloud/v1/recovery/request";
        let body = br#"{"npub":"3f"}"#;
        let event = event_by(&t, "post", url, payload_hash(body), 1_791_012_345).expect("event");
        assert_eq!(event.kind, KIND_HTTP_AUTH);
        assert_eq!(event.pubkey, temp_pub(&t).expect("pub"));
        assert_eq!(event.tags[0], vec!["u", url]);
        assert_eq!(event.tags[1], vec!["method", "POST"]);
        assert_eq!(event.tags[2][1], hex(&Sha256::digest(body)));
        let serial = serde_json::json!([0, event.pubkey, event.created_at, 27235, event.tags, ""]);
        let id: [u8; 32] = Sha256::digest(serial.to_string().as_bytes()).into();
        assert_eq!(event.id, hex(&id));
        let secp = Secp256k1::verification_only();
        let key = XOnlyPublicKey::from_slice(&unhex(&event.pubkey)).expect("x-only");
        let sig = Signature::from_slice(&unhex(&event.sig)).expect("sig");
        secp.verify_schnorr(&sig, &id, &key).expect("verifies");
        // A GET has no body and no tag `payload`.
        let get = event_by(&t, "GET", url, payload_hash(b""), 1).expect("event");
        assert_eq!(get.tags.len(), 2);
    }

    #[test]
    fn the_header_is_the_base64_of_the_json_of_the_event() {
        let t = TempKey::generate().expect("key");
        let event = event_by(
            &t,
            "GET",
            "https://eu-1-media.example.com/cloud/v1/account",
            None,
            7,
        )
        .expect("event");
        let header = authorization(&event).expect("header");
        let encoded = header.strip_prefix("Nostr ").expect("scheme");
        let back: WireEvent =
            serde_json::from_slice(&STANDARD.decode(encoded).expect("base64")).expect("json");
        assert_eq!(back, event);
    }
}

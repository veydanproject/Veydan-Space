// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys of a group call and the word of identity on the control
//! channel (the plan of calls, «Шифрование → Группы»; wire §10,
//! "Групповые звонки").
//!
//! The group shares a **secret of the epoch** (32 bytes, in `call.start`
//! and `call.epoch` under the group key). From it every participant
//! derives the key it sends with — HKDF-SHA256 of the secret with the
//! call, the seat and the slot in the info — and the keys of everybody
//! else it listens to. The slot of the frames is the epoch itself (one
//! byte on the wire, the engine keeps a ring of 256); the node sees
//! nothing but ciphertext under a key it never has.
//!
//! The **word of identity** ties a seat of the node to a person: a
//! statement of `npub`, seat, call, room, epoch and the DTLS fingerprint
//! of the participant's own description, signed with the person's Nostr
//! key (BIP-340 over a tagged hash, as the proof of a presence key) and
//! sealed with AES-GCM under a key of the epoch, so that only the holders
//! of the epoch's secret read it and only a member of the group can have
//! written it. The receivers check the signature, that the signer is a
//! member, that the seat is the one the node put in front of the frame;
//! a seat not confirmed so is neither shown nor listened to (no key is
//! set for its m-lines).

use crate::group::signal::Secret;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use messenger_core::PubKey;
use nostr::key::Keys;
use nostr::prelude::PublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The salt of every derivation of a call: names the purpose, not a secret.
pub const MEDIA_INFO: &[u8] = b"veydan-media";
const HELLO_INFO: &[u8] = b"veydan-hello";
const HELLO_CONTEXT: &[u8] = b"veydan.call.hello.v1\0";
/// The first byte of a word of identity on the wire.
const HELLO_TAG: u8 = 0x01;
const NONCE_LEN: usize = 12;
/// A word of identity is a few hundred bytes; more is not one.
const MAX_HELLO: usize = 4096;

/// HKDF-SHA256 (RFC 5869): extract with `salt`, expand `info` to `len` bytes.
pub fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut extract = HmacSha256::new_from_slice(salt).expect("hmac takes any key length");
    extract.update(ikm);
    let prk = extract.finalize().into_bytes();
    let mut out = Vec::with_capacity(len);
    let mut previous: Vec<u8> = Vec::new();
    let mut counter = 1u8;
    while out.len() < len {
        let mut expand = HmacSha256::new_from_slice(&prk).expect("hmac takes any key length");
        expand.update(&previous);
        expand.update(info);
        expand.update(&[counter]);
        previous = expand.finalize().into_bytes().to_vec();
        out.extend_from_slice(&previous);
        counter += 1;
    }
    out.truncate(len);
    out
}

/// The key material the participant `seat` sends with in the epoch
/// `epoch` of the call: 32 bytes, the same for everybody who derives it
/// from the secret of that epoch.
pub fn sender_key(secret: &Secret, call_id: &str, seat: u32, epoch: u32) -> Vec<u8> {
    let mut info = Vec::with_capacity(MEDIA_INFO.len() + call_id.len() + 8);
    info.extend_from_slice(MEDIA_INFO);
    info.extend_from_slice(call_id.as_bytes());
    info.extend_from_slice(&seat.to_be_bytes());
    info.extend_from_slice(&epoch.to_be_bytes());
    hkdf(call_id.as_bytes(), secret, &info, 32)
}

/// The slot of the engine's ring an epoch's keys live in.
pub fn slot(epoch: u32) -> u8 {
    (epoch % 256) as u8
}

/// The key the words of identity of an epoch are sealed with.
fn hello_key(secret: &Secret, call_id: &str, epoch: u32) -> [u8; 32] {
    let mut info = Vec::with_capacity(HELLO_INFO.len() + call_id.len() + 4);
    info.extend_from_slice(HELLO_INFO);
    info.extend_from_slice(call_id.as_bytes());
    info.extend_from_slice(&epoch.to_be_bytes());
    hkdf(call_id.as_bytes(), secret, &info, 32).try_into().expect("32 bytes asked")
}

/// What a participant says of itself, as signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub npub: String,
    pub participant: u32,
    pub call_id: String,
    pub room_id: String,
    pub epoch: u32,
    /// The fingerprint of the DTLS certificate of the participant's own
    /// description (`a=fingerprint` of its offer), so that the seat the
    /// node sees and the person are one.
    pub dtls_fp: String,
}

impl Hello {
    fn digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(HELLO_CONTEXT);
        for part in [self.npub.as_str(), self.call_id.as_str(), self.room_id.as_str(), self.dtls_fp.as_str()] {
            h.update((part.len() as u32).to_be_bytes());
            h.update(part.as_bytes());
        }
        h.update(self.participant.to_be_bytes());
        h.update(self.epoch.to_be_bytes());
        h.finalize().into()
    }
}

#[derive(Serialize, Deserialize)]
struct Signed {
    #[serde(flatten)]
    hello: Hello,
    sig: String,
}

/// The word of identity of `keys` as it goes on the channel: signed,
/// sealed under the key of the epoch it names, tagged.
pub fn seal_hello(keys: &Keys, hello: &Hello, secret: &Secret) -> Vec<u8> {
    let sig = keys.sign_schnorr(hello.digest()).to_hex();
    let plain = serde_json::to_vec(&Signed { hello: hello.clone(), sig }).expect("plain data");
    let key = hello_key(secret, &hello.call_id, hello.epoch);
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("the system has random bytes");
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes");
    let ct = cipher.encrypt(&Nonce::from(nonce), plain.as_slice()).expect("sealing plain data");
    let mut out = Vec::with_capacity(1 + 4 + NONCE_LEN + ct.len());
    out.push(HELLO_TAG);
    out.extend_from_slice(&hello.epoch.to_be_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// The epoch a sealed word names on the outside, so that the reader
/// knows which secret opens it. `None` for a frame of another shape.
pub fn hello_epoch(frame: &[u8]) -> Option<u32> {
    if frame.len() < 1 + 4 + NONCE_LEN + 16 || frame.len() > MAX_HELLO || frame[0] != HELLO_TAG {
        return None;
    }
    Some(u32::from_be_bytes(frame[1..5].try_into().ok()?))
}

/// Why a word of identity is not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelloError {
    /// Not a word of identity, or not under this secret.
    Unreadable,
    /// The signature is not the npub's.
    BadSignature,
    /// It speaks of another call, room or epoch than the one it came in.
    WrongCall,
    /// The seat it names is not the seat the node put in front of it.
    WrongSeat,
}

/// Open a word of identity that came from the seat `from` in the call
/// `call_id`, room `room_id`, under the secret of the epoch it names.
/// The membership of the npub is the caller's to check.
pub fn open_hello(frame: &[u8], from: u32, call_id: &str, room_id: &str, secret: &Secret) -> Result<Hello, HelloError> {
    let epoch = hello_epoch(frame).ok_or(HelloError::Unreadable)?;
    let nonce: [u8; NONCE_LEN] = frame[5..5 + NONCE_LEN].try_into().map_err(|_| HelloError::Unreadable)?;
    let key = hello_key(secret, call_id, epoch);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes");
    let plain = cipher.decrypt(&Nonce::from(nonce), &frame[5 + NONCE_LEN..]).map_err(|_| HelloError::Unreadable)?;
    let signed: Signed = serde_json::from_slice(&plain).map_err(|_| HelloError::Unreadable)?;
    let hello = signed.hello;
    if hello.call_id != call_id || hello.room_id != room_id || hello.epoch != epoch {
        return Err(HelloError::WrongCall);
    }
    if hello.participant != from {
        return Err(HelloError::WrongSeat);
    }
    let key = PublicKey::from_hex(&hello.npub).ok().and_then(|p| p.xonly().ok()).ok_or(HelloError::BadSignature)?;
    let sig = hex::decode(&signed.sig).ok().and_then(|b| <[u8; 64]>::try_from(b).ok()).ok_or(HelloError::BadSignature)?;
    let sig = secp256k1::schnorr::Signature::from_byte_array(sig);
    secp256k1::Secp256k1::verification_only()
        .verify_schnorr(&sig, &hello.digest(), &key)
        .map_err(|_| HelloError::BadSignature)?;
    Ok(hello)
}

/// The DTLS fingerprint in an SDP (`a=fingerprint:sha-256 AB:CD…`), or
/// when there is none (an engine without DTLS, the fake of the tests)
/// the hash of the description itself: what matters is that the far
/// end's word names the description the node took.
pub fn dtls_fingerprint(sdp: &str) -> String {
    for line in sdp.lines() {
        if let Some(rest) = line.trim_end().strip_prefix("a=fingerprint:") {
            return rest.trim().to_string();
        }
    }
    format!("sdp-sha256 {}", hex::encode(Sha256::digest(sdp.as_bytes())))
}

/// The hex of a Nostr public key, as `Hello::npub` carries it.
pub fn npub_of(keys: &Keys) -> String {
    keys.public_key().to_hex()
}

pub fn pubkey_of(hello: &Hello) -> Option<PubKey> {
    PubKey::parse(&hello.npub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hkdf_matches_rfc_5869_test_case_1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0u8..=12).collect();
        let info: Vec<u8> = (0xf0u8..=0xf9).collect();
        let okm = hkdf(&salt, &ikm, &info, 42);
        assert_eq!(
            hex::encode(okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
    }

    #[test]
    fn keys_differ_by_seat_and_epoch_and_everybody_derives_the_same() {
        let secret = [3u8; 32];
        let a = sender_key(&secret, "call", 1, 1);
        assert_eq!(a.len(), 32);
        assert_eq!(a, sender_key(&secret, "call", 1, 1));
        assert_ne!(a, sender_key(&secret, "call", 2, 1));
        assert_ne!(a, sender_key(&secret, "call", 1, 2));
        assert_ne!(a, sender_key(&[4u8; 32], "call", 1, 1));
        assert_ne!(a, sender_key(&secret, "other", 1, 1));
        assert_eq!(slot(1), 1);
        assert_eq!(slot(257), 1);
        assert_ne!(hello_key(&secret, "call", 1), sender_key(&secret, "call", 1, 1).as_slice());
    }

    #[test]
    fn a_word_of_identity_opens_for_the_holders_of_the_secret_and_names_its_seat() {
        let alice = Keys::generate();
        let secret = new_secret();
        let hello = Hello {
            npub: npub_of(&alice),
            participant: 2,
            call_id: "c".into(),
            room_id: "r".into(),
            epoch: 1,
            dtls_fp: "sha-256 AB:CD".into(),
        };
        let frame = seal_hello(&alice, &hello, &secret);
        assert_eq!(hello_epoch(&frame), Some(1));
        assert!(!String::from_utf8_lossy(&frame).contains(&hello.npub), "the npub is sealed");
        assert_eq!(open_hello(&frame, 2, "c", "r", &secret), Ok(hello.clone()));
        assert_eq!(open_hello(&frame, 3, "c", "r", &secret), Err(HelloError::WrongSeat), "the node says seat 3 sent it");
        assert_eq!(open_hello(&frame, 2, "c", "other", &secret), Err(HelloError::WrongCall));
        assert_eq!(open_hello(&frame, 2, "c", "r", &new_secret()), Err(HelloError::Unreadable), "another secret");
        assert_eq!(open_hello(b"\x01\0\0\0\x01short", 2, "c", "r", &secret), Err(HelloError::Unreadable));
        assert_eq!(open_hello(b"\x02bytes of something else that is long enough", 2, "c", "r", &secret), Err(HelloError::Unreadable));

        // Mallory seals a word naming Alice with his own key: the signature is not hers.
        let mallory = Keys::generate();
        let forged = seal_hello(&mallory, &hello, &secret);
        assert_eq!(open_hello(&forged, 2, "c", "r", &secret), Err(HelloError::BadSignature));
        // And a word of another epoch opens under that epoch's key alone.
        let later = Hello { epoch: 2, ..hello };
        let frame = seal_hello(&alice, &later, &secret);
        assert_eq!(hello_epoch(&frame), Some(2));
        assert_eq!(open_hello(&frame, 2, "c", "r", &secret).unwrap().epoch, 2);
    }

    #[test]
    fn the_fingerprint_is_read_from_the_sdp_or_made_of_it() {
        let sdp = "v=0\r\na=fingerprint:sha-256 AB:CD:EF\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n";
        assert_eq!(dtls_fingerprint(sdp), "sha-256 AB:CD:EF");
        let fake = dtls_fingerprint("fake-offer:1:0");
        assert!(fake.starts_with("sdp-sha256 "));
        assert_ne!(fake, dtls_fingerprint("fake-offer:2:0"));
    }

    use crate::group::signal::new_secret;
}

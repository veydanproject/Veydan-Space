// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Encryption of frames, for the calls that cross an SFU (stage 7b).
//!
//! AES-GCM over the payload of every frame, under a key the SFU never
//! gets (the FrameCryptor of libwebrtc; tmp/calls-spike/REPORT.md,
//! section 5). The keys live in a [`FrameKeys`] ring of up to 256 slots:
//! an epoch of the plan is a slot, the index travels in the last byte of
//! every frame, so a late frame of the old epoch is still read while the
//! old key is in the ring. A key may be ratcheted forward (HKDF); a
//! receiver that fails to decrypt tries a few ratchets ahead by itself.
//! The nonce of a frame is `ssrc ‖ timestamp ‖ counter`, so the key of a
//! slot must change whenever the SSRC may repeat under it (a new session,
//! a restart on another node).
//!
//! Two shapes of a ring:
//!
//! - [`FrameKeys::shared`]: one key per slot for everybody, both ways.
//!   The tests of a pair, and a call whose participants all derive one
//!   secret.
//! - [`FrameKeys::per_sender`]: a key per slot *per sender* (the plan,
//!   "Шифрование → группы": every participant has a sender key of the
//!   epoch, derived from the epoch's secret and its id). This side's own
//!   sender key is set with [`crate::Session::set_sender_key`]; the key
//!   of every other participant goes with the m-line its stream comes on
//!   ([`crate::Session::set_receiver_key`]), because the node tells whose
//!   a stream is by its mid. Inside the ring the sender is the
//!   participant [`SENDER`] and a receiver is [`receiver_id`] of its mid;
//!   nothing outside the engine sees these names.
//!
//! How a key is derived from the secret of a group, who confirms whose
//! identity and when the epoch moves are the caller's (`messenger-calls`);
//! this module holds the bytes and the switch.

use libwebrtc::native::frame_cryptor::{self as fc, KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions};

/// The key ring of a call, shared by every session of that call in this
/// process. Cheap to clone: a handle on one ring.
#[derive(Clone)]
pub struct FrameKeys {
    pub(crate) provider: KeyProvider,
    /// One key per slot for everybody (`true`), or one per sender.
    shared: bool,
}

/// How many ratchets ahead a receiver tries when a frame does not
/// decrypt, before it reports a missing key.
const RATCHET_WINDOW: i32 = 4;
/// Slots of the ring. The index of a frame is one byte; the whole range is
/// kept so that an epoch counter wraps without a collision within 256
/// epochs.
const KEY_RING: i32 = 256;

/// The name of this side's sender inside a per-sender ring.
pub(crate) const SENDER: &str = "tx";

/// The name of the receiver of the m-line `mid` inside a per-sender ring.
pub(crate) fn receiver_id(mid: &str) -> String {
    format!("rx:{mid}")
}

impl FrameKeys {
    /// A ring with one key per slot for everybody, derived with `salt`
    /// (the same for everybody in the call: it names the call, not a
    /// secret).
    pub fn shared(salt: &[u8]) -> FrameKeys {
        Self::make(salt, true)
    }

    /// The same as [`FrameKeys::shared`], by the name the first callers
    /// used.
    pub fn new(salt: &[u8]) -> FrameKeys {
        Self::shared(salt)
    }

    /// A ring with a key per slot per sender: this side's under
    /// [`crate::Session::set_sender_key`], every other participant's
    /// under the mid of its stream ([`crate::Session::set_receiver_key`]).
    pub fn per_sender(salt: &[u8]) -> FrameKeys {
        Self::make(salt, false)
    }

    fn make(salt: &[u8], shared: bool) -> FrameKeys {
        let provider = KeyProvider::new(KeyProviderOptions {
            shared_key: shared,
            ratchet_window_size: RATCHET_WINDOW,
            ratchet_salt: salt.to_vec(),
            // A decryption failure never silences the receiver for good:
            // the next key may arrive a moment later.
            failure_tolerance: -1,
            key_ring_size: KEY_RING,
            key_derivation_algorithm: KeyDerivationAlgorithm::HKDF,
        });
        FrameKeys { provider, shared }
    }

    /// Whether the ring holds one key per slot for everybody.
    pub fn is_shared(&self) -> bool {
        self.shared
    }

    /// The key of slot `index` for everybody (a shared ring; on a
    /// per-sender ring it is this side's sender key, as
    /// [`FrameKeys::set_sender_key`]). Replaces what was there.
    pub fn set_key(&self, index: u8, key: &[u8]) -> bool {
        self.set_sender_key(index, key)
    }

    /// The key of slot `index` ratcheted one step forward, for everybody
    /// who holds this ring; the new material, to tell the others.
    pub fn ratchet(&self, index: u8) -> Option<Vec<u8>> {
        self.ratchet_sender_key(index)
    }

    /// The material in slot `index`.
    pub fn key(&self, index: u8) -> Option<Vec<u8>> {
        if self.shared {
            self.provider.get_shared_key(index as i32)
        } else {
            self.provider.get_key(SENDER.into(), index as i32)
        }
    }

    /// The key of slot `index` this side's frames are encrypted with
    /// (a per-sender ring; on a shared ring the one key of the slot).
    pub fn set_sender_key(&self, index: u8, key: &[u8]) -> bool {
        if self.shared {
            self.provider.set_shared_key(index as i32, key.to_vec())
        } else {
            self.provider.set_key(SENDER.into(), index as i32, key.to_vec())
        }
    }

    /// This side's sender key of slot `index` ratcheted forward; the new
    /// material.
    pub fn ratchet_sender_key(&self, index: u8) -> Option<Vec<u8>> {
        if self.shared {
            self.provider.ratchet_shared_key(index as i32)
        } else {
            self.provider.ratchet_key(SENDER.into(), index as i32)
        }
    }

    /// The key of slot `index` the frames that come on the m-line `mid`
    /// are decrypted with (a per-sender ring; on a shared ring the one
    /// key of the slot, whatever the mid).
    pub fn set_receiver_key(&self, mid: &str, index: u8, key: &[u8]) -> bool {
        if self.shared {
            self.provider.set_shared_key(index as i32, key.to_vec())
        } else {
            self.provider.set_key(receiver_id(mid), index as i32, key.to_vec())
        }
    }

    /// The key of slot `index` of the m-line `mid` ratcheted forward; the
    /// new material (a receiver may also be ratcheted by the engine
    /// itself, a few steps, when a frame does not decrypt).
    pub fn ratchet_receiver_key(&self, mid: &str, index: u8) -> Option<Vec<u8>> {
        if self.shared {
            self.provider.ratchet_shared_key(index as i32)
        } else {
            self.provider.ratchet_key(receiver_id(mid), index as i32)
        }
    }

    /// The material in slot `index` of the m-line `mid`.
    pub fn receiver_key(&self, mid: &str, index: u8) -> Option<Vec<u8>> {
        if self.shared {
            self.provider.get_shared_key(index as i32)
        } else {
            self.provider.get_key(receiver_id(mid), index as i32)
        }
    }
}

impl std::fmt::Debug for FrameKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameKeys").field("shared", &self.shared).finish()
    }
}

/// Frame encryption for a session: the ring and the name this side goes
/// by in the state events. Given at the session's creation, so that the
/// far end's tracks are decrypted from their first frame.
#[derive(Debug, Clone)]
pub struct Encryption {
    pub keys: FrameKeys,
    /// This participant, as the state events name it.
    pub participant: String,
}

/// What the cryptor of one direction reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionState {
    New,
    Ok,
    EncryptionFailed,
    DecryptionFailed,
    MissingKey,
    KeyRatcheted,
    InternalError,
}

impl From<fc::EncryptionState> for EncryptionState {
    fn from(s: fc::EncryptionState) -> Self {
        match s {
            fc::EncryptionState::New => Self::New,
            fc::EncryptionState::Ok => Self::Ok,
            fc::EncryptionState::EncryptionFailed => Self::EncryptionFailed,
            fc::EncryptionState::DecryptionFailed => Self::DecryptionFailed,
            fc::EncryptionState::MissingKey => Self::MissingKey,
            fc::EncryptionState::KeyRatcheted => Self::KeyRatcheted,
            fc::EncryptionState::InternalError => Self::InternalError,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A per-sender ring keeps this side's key apart from the keys of the
    /// m-lines; a shared ring has one key per slot for all of them.
    #[test]
    fn a_per_sender_ring_keeps_the_keys_apart_and_a_shared_one_does_not() {
        let ring = FrameKeys::per_sender(b"salt");
        assert!(!ring.is_shared());
        assert!(ring.set_sender_key(0, &[1; 32]));
        assert!(ring.set_receiver_key("3", 0, &[2; 32]));
        assert!(ring.set_receiver_key("4", 1, &[3; 32]));
        assert_eq!(ring.key(0), Some(vec![1; 32]));
        assert_eq!(ring.receiver_key("3", 0), Some(vec![2; 32]));
        assert_eq!(ring.receiver_key("4", 1), Some(vec![3; 32]));
        assert_ne!(ring.receiver_key("4", 0), Some(vec![3; 32]), "another slot");
        let stepped = ring.ratchet_receiver_key("3", 0).expect("ratchet");
        assert_ne!(stepped, vec![2; 32]);
        assert_eq!(ring.receiver_key("3", 0), Some(stepped));
        assert_eq!(ring.receiver_key("4", 1), Some(vec![3; 32]), "the others are untouched");

        let shared = FrameKeys::shared(b"salt");
        assert!(shared.is_shared());
        assert!(shared.set_sender_key(0, &[7; 32]));
        assert_eq!(shared.receiver_key("any", 0), Some(vec![7; 32]));
        assert_eq!(receiver_id("3"), "rx:3");
    }
}

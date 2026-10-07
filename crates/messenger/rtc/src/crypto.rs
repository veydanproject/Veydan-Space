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
//! How a key is derived from the secret of a group, who confirms whose
//! identity and when the epoch moves are the caller's (`messenger-calls`);
//! this module holds the bytes and the switch.

use libwebrtc::native::frame_cryptor::{self as fc, KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions};

/// The key ring of a call, shared by every session of that call in this
/// process. Cheap to clone: a handle on one ring.
#[derive(Clone)]
pub struct FrameKeys {
    pub(crate) provider: KeyProvider,
}

/// How many ratchets ahead a receiver tries when a frame does not
/// decrypt, before it reports a missing key.
const RATCHET_WINDOW: i32 = 4;
/// Slots of the ring. The index of a frame is one byte; the whole range is
/// kept so that an epoch counter wraps without a collision within 256
/// epochs.
const KEY_RING: i32 = 256;

impl FrameKeys {
    /// A ring whose keys are derived with `salt` (the same for everybody
    /// in the call: it names the call, not a secret).
    pub fn new(salt: &[u8]) -> FrameKeys {
        let provider = KeyProvider::new(KeyProviderOptions {
            shared_key: true,
            ratchet_window_size: RATCHET_WINDOW,
            ratchet_salt: salt.to_vec(),
            // A decryption failure never silences the receiver for good:
            // the next key may arrive a moment later.
            failure_tolerance: -1,
            key_ring_size: KEY_RING,
            key_derivation_algorithm: KeyDerivationAlgorithm::HKDF,
        });
        FrameKeys { provider }
    }

    /// The key of slot `index` (the material; the engine derives the AES
    /// key from it with the salt). Replaces what was there.
    pub fn set_key(&self, index: u8, key: &[u8]) -> bool {
        self.provider.set_shared_key(index as i32, key.to_vec())
    }

    /// The key of slot `index` ratcheted one step forward, for everybody
    /// who holds this ring; the new material, to tell the others.
    pub fn ratchet(&self, index: u8) -> Option<Vec<u8>> {
        self.provider.ratchet_shared_key(index as i32)
    }

    /// The material in slot `index`.
    pub fn key(&self, index: u8) -> Option<Vec<u8>> {
        self.provider.get_shared_key(index as i32)
    }
}

impl std::fmt::Debug for FrameKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrameKeys")
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

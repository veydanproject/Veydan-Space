// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Recovery of access to Veydan Cloud (`internal/cloud-recovery.md`): the
//! formats both sides of every scenario have to agree on, and nothing else.
//! The crate knows no relay, no server and no screen.
//!
//! - [`rk`]: the Recovery Key, 128 bits shown as 12 words, and the HKDF
//!   labels every other key is derived from (§3).
//! - [`escrow`]: the escrow v1 the owner's device puts into vstore, sealed
//!   under the key (§4).
//! - [`devicelink`]: the inner wrap of the nsec when a new device is linked
//!   with a code of 8 words (§6).
//! - [`gf256`], [`shamir`]: the shares of the key for the trusted people
//!   (§8.2).
//! - [`pow`]: the proof of work of an activation or a recovery request (§9)
//!   and the check code of a request (`services/cloud/spec/recovery.md` §3).
//!
//! Everything secret the crate holds is zeroized when dropped, and what it
//! hands out by value says so at the function (`Share::to_bytes`,
//! `RecoveryKey::from_bytes`); the Debug output of a key names its type
//! and nothing else.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod cbor;
pub mod devicelink;
pub mod escrow;
pub mod gf256;
pub mod kdf;
pub mod pow;
pub mod rk;
pub mod shamir;

pub use escrow::{Escrow, EscrowError, LockKey, RecoveryParams, Storage, SyncKeys};
pub use kdf::{KdfError, KdfParams};
pub use pow::{Challenge, PowError, Purpose};
pub use rk::{AuthKey, Label, RecoveryKey, RkError};
pub use shamir::{Recovered, ShamirError, Share, ShareError};

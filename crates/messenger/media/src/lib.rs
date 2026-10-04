// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Encrypted blobs.
//!
//! A file is split into chunks, every chunk is encrypted with AES-256-GCM
//! under a fresh per-file key and stored on a blob server under the
//! SHA-256 of its ciphertext. Servers see opaque blobs named by their own
//! hash; the key travels only inside the end-to-end encrypted message.
//!
//! - `crypto`: key material, per-chunk nonces, chunk encryption.
//! - `descriptor`: what the message carries so the peer can fetch and open
//!   the file (our own format, versioned).
//! - `sigv4`: AWS Signature V4, enough for S3-compatible object storage.
//! - `backend`: `BlobBackend` with S3 (the default) and Blossom.
//! - `upload` / `download`: chunk-by-chunk streaming with resume.
//! - `service`: configured servers, the transfer table, scheduling.
//!
//! Shared by DMs now and by groups later; nothing here knows about chats.

pub mod backend;
pub mod crypto;
pub mod descriptor;
pub mod download;
pub mod service;
pub mod sigv4;
pub mod upload;

pub use backend::{BlobBackend, BlossomBackend, MemoryBackend, S3Backend, S3Config};
pub use descriptor::{ChunkRef, MediaDescriptor, MediaKind};
pub use service::{MediaServerInput, MediaServerView, MediaService, Progress, ProgressSink, TransferView};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a `media` message carries (internal/messenger-wire.md §3). Version 1:
//!
//! ```json
//! {"v":1,"t":"media","kind":"image","name":"cat.jpg","mime":"image/jpeg",
//!  "size":183422,"sha256":"<hex of the plaintext file>",
//!  "chunk_size":4194304,
//!  "chunks":[{"sha256":"<hex of ciphertext chunk 0>","size":183438}],
//!  "algo":"aes-256-gcm","key":"<base64>","iv":"<base64>",
//!  "servers":["https://media.example/bucket"],
//!  "caption":"look","batch":"<id>","dim":[1280,720]}
//! ```
//!
//! A chunk lives at `<server>/<chunk sha256>` on every listed server. The
//! key never leaves the encrypted message.

use crate::crypto::FileKey;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::envelope::T_MEDIA;
use messenger_core::{Envelope, MessengerError, Result};
use serde::{Deserialize, Serialize};

pub const ALGO: &str = "aes-256-gcm";
/// Files larger than this are refused (each chunk index is a u32 and a
/// descriptor must fit a message).
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Files larger than this are not sent from here. What others send is
/// held to `MAX_FILE_BYTES` only.
pub const MAX_SEND_BYTES: u64 = 1 << 30;
pub const MAX_CHUNKS: usize = 4096;
pub const MAX_WAVEFORM: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    File,
    /// A voice message recorded in the app.
    Voice,
    /// A short round video recorded in the app.
    Circle,
}

impl MediaKind {
    pub fn from_mime(mime: &str) -> Self {
        match mime.split('/').next().unwrap_or("") {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            _ => Self::File,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "file" => Self::File,
            "voice" => Self::Voice,
            "circle" => Self::Circle,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::File => "file",
            Self::Voice => "voice",
            Self::Circle => "circle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    /// SHA-256 of the encrypted chunk: its name on the server.
    pub sha256: String,
    /// Size of the encrypted chunk in bytes.
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaDescriptor {
    pub kind: MediaKind,
    pub name: String,
    pub mime: String,
    /// Plaintext size.
    pub size: u64,
    /// SHA-256 of the plaintext file.
    pub sha256: String,
    pub chunk_size: u64,
    pub chunks: Vec<ChunkRef>,
    pub algo: String,
    /// Base64 of the 32-byte key.
    pub key: String,
    /// Base64 of the 12-byte base nonce.
    pub iv: String,
    pub servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dim: Option<(u32, u32)>,
    /// Length of a recording in milliseconds (voice, circle, audio, video).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Loudness outline of a voice message: up to 64 values, 0..=255.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform: Option<Vec<u8>>,
}

impl MediaDescriptor {
    pub fn file_key(&self) -> Result<FileKey> {
        let bad = |_| MessengerError::Invalid("media key is not base64".into());
        FileKey::from_parts(&B64.decode(&self.key).map_err(bad)?, &B64.decode(&self.iv).map_err(bad)?)
    }

    pub fn set_key(&mut self, k: &FileKey) {
        self.key = B64.encode(k.key);
        self.iv = B64.encode(k.base_nonce);
    }

    /// Reject anything a hostile peer could use against the receiver:
    /// absurd sizes, mismatching chunk tables, non-http servers, names
    /// that are paths.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(MessengerError::Invalid(format!("media descriptor: {m}")));
        if self.algo != ALGO {
            return bad("unknown algorithm");
        }
        if self.size == 0 || self.size > MAX_FILE_BYTES {
            return bad("size out of range");
        }
        if self.chunk_size < 64 * 1024 || self.chunk_size > 64 * 1024 * 1024 {
            return bad("chunk size out of range");
        }
        let expected = self.size.div_ceil(self.chunk_size) as usize;
        if self.chunks.len() != expected || expected > MAX_CHUNKS {
            return bad("chunk table does not match the size");
        }
        // Every chunk is bound to its place, as every sender makes it: a
        // receiver never makes room for more than one chunk can hold.
        let overhead = crate::crypto::TAG_LEN as u64;
        let fits = |(i, c): (usize, &ChunkRef)| c.size == self.chunk_size.min(self.size - i as u64 * self.chunk_size) + overhead;
        if !self.chunks.iter().enumerate().all(fits) {
            return bad("chunk sizes do not match their places");
        }
        let hex64 = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        if !hex64(&self.sha256) || !self.chunks.iter().all(|c| hex64(&c.sha256)) {
            return bad("hashes must be lowercase hex sha256");
        }
        if self.servers.is_empty() || self.servers.len() > 8 {
            return bad("needs 1 to 8 servers");
        }
        if !self.servers.iter().all(|s| s.starts_with("https://") || s.starts_with("http://")) {
            return bad("servers must be http(s)");
        }
        if self.waveform.as_ref().is_some_and(|w| w.len() > MAX_WAVEFORM) {
            return bad("waveform is too long");
        }
        if self.batch.as_ref().is_some_and(|b| !valid_batch(b)) {
            return bad("batch is not an identifier");
        }
        if self.duration_ms.is_some_and(|d| d > 24 * 3600 * 1000) {
            return bad("duration out of range");
        }
        self.file_key()?;
        if safe_name(&self.name) != self.name {
            return bad("unsafe file name");
        }
        Ok(())
    }

    pub fn to_envelope(&self) -> Envelope {
        let mut e = Envelope::new(T_MEDIA);
        if let serde_json::Value::Object(map) = serde_json::to_value(self).expect("descriptor serializes") {
            e.fields = map;
        }
        e
    }

    pub fn from_envelope(e: &Envelope) -> Result<Self> {
        if e.t != T_MEDIA {
            return Err(MessengerError::Invalid("not a media envelope".into()));
        }
        Self::from_fields(&serde_json::Value::Object(e.fields.clone()))
    }

    /// From the `media_json` stored with a message.
    pub fn from_fields(v: &serde_json::Value) -> Result<Self> {
        let d: Self = serde_json::from_value(v.clone()).map_err(|e| MessengerError::Invalid(format!("media descriptor: {e}")))?;
        d.validate()?;
        Ok(d)
    }

    pub fn chunk_url(server: &str, sha256: &str) -> String {
        format!("{}/{}", server.trim_end_matches('/'), sha256)
    }
}

/// What says that files were sent by one action: 8 to 64 characters of
/// `0-9 a-z A-Z - _`. Made by the sender, compared by the receiver, never
/// shown.
pub fn valid_batch(batch: &str) -> bool {
    (8..=64).contains(&batch.len()) && batch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A file name that cannot escape a directory: no separators, no leading
/// dots, no control characters, bounded length. Empty becomes `file`.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let mut out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        out = "file".into();
    }
    out
}

/// Chunk size by file size: fewer round trips for big files, quick
/// retries for small ones.
pub fn chunk_size_for(size: u64) -> u64 {
    const MIB: u64 = 1024 * 1024;
    if size < 200 * MIB {
        4 * MIB
    } else if size < 1024 * MIB {
        8 * MIB
    } else {
        16 * MIB
    }
}

/// Mime type from the file extension; conservative list, `octet-stream`
/// otherwise.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "weba" => "audio/webm",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" | "log" | "md" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample() -> MediaDescriptor {
        let mut d = MediaDescriptor {
            kind: MediaKind::Image,
            name: "cat.jpg".into(),
            mime: "image/jpeg".into(),
            size: 100_000,
            sha256: "a".repeat(64),
            chunk_size: 4 * 1024 * 1024,
            chunks: vec![ChunkRef { sha256: "b".repeat(64), size: 100_016 }],
            algo: ALGO.into(),
            key: String::new(),
            iv: String::new(),
            servers: vec!["https://media.example/bucket".into()],
            caption: Some("look".into()),
            batch: None,
            dim: Some((1280, 720)),
            duration_ms: None,
            waveform: None,
        };
        d.set_key(&FileKey { key: [1; 32], base_nonce: [2; 12] });
        d
    }

    #[test]
    fn batch_is_an_identifier_or_nothing() {
        let mut d = sample();
        for ok in ["0123abcd", "b3f1c2d4-5e6f-4a7b-8c9d-0e1f2a3b4c5d", "A_b-9xyz"] {
            d.batch = Some(ok.into());
            assert!(d.validate().is_ok(), "{ok}");
            assert_eq!(MediaDescriptor::from_envelope(&d.to_envelope()).unwrap().batch.as_deref(), Some(ok));
        }
        for bad in ["", "short", "has space", "semi;colon", "путь", &"a".repeat(65)] {
            d.batch = Some(bad.to_string());
            assert!(d.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn envelope_golden_vector_and_round_trip() {
        let d = sample();
        d.validate().unwrap();
        let json = d.to_envelope().encode();
        assert_eq!(
            json,
            r#"{"v":1,"t":"media","algo":"aes-256-gcm","caption":"look","chunk_size":4194304,"chunks":[{"sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","size":100016}],"dim":[1280,720],"iv":"AgICAgICAgICAgIC","key":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=","kind":"image","mime":"image/jpeg","name":"cat.jpg","servers":["https://media.example/bucket"],"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":100000}"#
        );
        let back = MediaDescriptor::from_envelope(&Envelope::parse(&json).unwrap()).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.file_key().unwrap().key, [1; 32]);
        assert_eq!(MediaDescriptor::chunk_url("https://m.example/b/", &"c".repeat(64)), format!("https://m.example/b/{}", "c".repeat(64)));
    }

    #[test]
    fn hostile_descriptors_are_rejected() {
        let ok = sample();
        type Change = Box<dyn Fn(&mut MediaDescriptor)>;
        let cases: Vec<(&str, Change)> = vec![
            ("algo", Box::new(|d| d.algo = "none".into())),
            ("zero size", Box::new(|d| d.size = 0)),
            ("huge size", Box::new(|d| d.size = MAX_FILE_BYTES + 1)),
            ("tiny chunks", Box::new(|d| d.chunk_size = 16)),
            ("chunk count", Box::new(|d| d.chunks.push(ChunkRef { sha256: "c".repeat(64), size: 16 }))),
            ("chunk sizes", Box::new(|d| d.chunks[0].size = 5)),
            ("hash", Box::new(|d| d.sha256 = "zz".into())),
            ("upper hash", Box::new(|d| d.chunks[0].sha256 = "B".repeat(64))),
            ("no servers", Box::new(|d| d.servers.clear())),
            ("file url", Box::new(|d| d.servers = vec!["file:///etc".into()])),
            ("key", Box::new(|d| d.key = "AAAA".into())),
            ("path name", Box::new(|d| d.name = "../../etc/passwd".into())),
            ("hidden name", Box::new(|d| d.name = ".bashrc".into())),
            ("waveform", Box::new(|d| d.waveform = Some(vec![1; MAX_WAVEFORM + 1]))),
        ];
        for (what, change) in cases {
            let mut d = ok.clone();
            change(&mut d);
            assert!(d.validate().is_err(), "{what} must be rejected");
        }
    }

    /// Sizes that add up but not chunk by chunk: one chunk claims most of
    /// 4 GiB, so a receiver would make room for all of it at once.
    #[test]
    fn every_chunk_is_bound_to_its_place() {
        let mut d = sample();
        d.size = 4 << 30;
        d.chunk_size = 64 << 20;
        let n = d.size.div_ceil(d.chunk_size);
        let tag = crate::crypto::TAG_LEN as u64;
        d.chunks = (0..n).map(|_| ChunkRef { sha256: "b".repeat(64), size: d.chunk_size + tag }).collect();
        d.validate().unwrap();
        d.chunks[0].size = d.size + tag;
        for c in &mut d.chunks[1..] {
            c.size = tag;
        }
        assert_eq!(d.chunks.iter().map(|c| c.size).sum::<u64>(), d.size + tag * n, "they add up");
        assert!(d.validate().is_err());

        // The last chunk holds the rest, and only it.
        let mut d = sample();
        d.size = 3 * 64 * 1024 + 10;
        d.chunk_size = 64 * 1024;
        let sizes = [64 * 1024 + tag, 64 * 1024 + tag, 64 * 1024 + tag, 10 + tag];
        d.chunks = sizes.iter().map(|s| ChunkRef { sha256: "b".repeat(64), size: *s }).collect();
        d.validate().unwrap();
        (d.chunks[0].size, d.chunks[3].size) = (10 + tag, 64 * 1024 + tag);
        assert!(d.validate().is_err(), "the rest in the first place");
    }

    /// We send up to 1 GiB; a peer may send more and is still received.
    #[test]
    fn a_two_gib_file_from_a_peer_is_valid() {
        let mut d = sample();
        d.size = 2 << 30;
        d.chunk_size = chunk_size_for(d.size);
        let n = d.size.div_ceil(d.chunk_size);
        d.chunks = (0..n).map(|_| ChunkRef { sha256: "b".repeat(64), size: d.chunk_size + crate::crypto::TAG_LEN as u64 }).collect();
        assert!(d.size > MAX_SEND_BYTES);
        d.validate().unwrap();
    }

    #[test]
    fn names_sizes_and_mimes() {
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name("  .hidden "), "hidden");
        assert_eq!(safe_name("a\u{0}b:c.txt"), "a_b_c.txt");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name(&"я".repeat(300)).chars().count(), 120);
        assert_eq!(chunk_size_for(1), 4 << 20);
        assert_eq!(chunk_size_for(300 << 20), 8 << 20);
        assert_eq!(chunk_size_for(2 << 30), 16 << 20);
        assert_eq!(mime_for("Photo.JPG"), "image/jpeg");
        assert_eq!(mime_for("noext"), "application/octet-stream");
        assert_eq!(MediaKind::from_mime("video/mp4"), MediaKind::Video);
        assert_eq!(MediaKind::from_mime("application/pdf"), MediaKind::File);
    }
}

#[cfg(test)]
mod recording_tests {
    use super::tests::sample;
    use super::*;

    #[test]
    fn voice_fields_round_trip_and_stay_out_of_plain_files() {
        let mut d = sample();
        assert!(!d.to_envelope().encode().contains("duration_ms"), "absent fields are not written");
        d.kind = MediaKind::Voice;
        d.mime = "audio/webm".into();
        d.name = "voice.weba".into();
        d.duration_ms = Some(4200);
        d.waveform = Some(vec![0, 40, 255, 12]);
        d.validate().unwrap();
        let back = MediaDescriptor::from_envelope(&Envelope::parse(&d.to_envelope().encode()).unwrap()).unwrap();
        assert_eq!(back, d);
        assert_eq!(MediaKind::parse("circle"), Some(MediaKind::Circle));
        assert_eq!(MediaKind::parse("hologram"), None);
        assert_eq!(mime_for("v.weba"), "audio/webm");
    }
}

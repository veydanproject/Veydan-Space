// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The SHA-256 a downloaded archive is held against before anything is
//! unpacked from it: the Tor Expert Bundle here, Camoufox in Space.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// The SHA-256 of a file, in lower-case hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    sha256_reader(&mut std::fs::File::open(path)?)
}

/// The SHA-256 of what is left to read, in lower-case hex.
pub fn sha256_reader(file: &mut impl Read) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Holds a complete download of `file` against the checksum it must have.
/// A file that does not match is deleted with its marker (the file that
/// names what the partial download belongs to), so the next attempt starts
/// clean. An empty `expected` is no pin at all and matches nothing.
pub fn verify_download(
    tmp: &Path,
    marker: &Path,
    file: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = sha256_file(tmp).map_err(|e| format!("cannot read the download: {e}"))?;
    if expected.is_empty() || actual != expected {
        let _ = std::fs::remove_file(tmp);
        let _ = std::fs::remove_file(marker);
        return Err(format!(
            "checksum mismatch for {file}: expected {}, got {actual}",
            if expected.is_empty() {
                "<none pinned>"
            } else {
                expected
            }
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_kept_only_under_its_own_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("a.tmp");
        let marker = dir.path().join("a.marker");
        std::fs::write(&tmp, b"abc").unwrap();
        assert_eq!(
            sha256_file(&tmp).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let own = sha256_file(&tmp).unwrap();
        verify_download(&tmp, &marker, "a", &own).unwrap();
        assert!(tmp.exists());

        for expected in ["", &"0".repeat(64)] {
            std::fs::write(&tmp, b"abc").unwrap();
            std::fs::write(&marker, b"a").unwrap();
            let err = verify_download(&tmp, &marker, "a", expected).unwrap_err();
            assert!(err.contains("checksum mismatch"), "{err}");
            assert!(!tmp.exists() && !marker.exists());
        }
    }
}

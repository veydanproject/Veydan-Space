// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Browser -> notes capture bridge.
//!
//! Camoufox page --(WebExtension)--> native messaging host (this executable,
//! started by the browser with the extension id in argv) --(local socket)-->
//! IPC server inside the running Tauri app --> notes commands.
//!
//! - `extension` — builds the per-profile .xpi and registers the native host manifest
//! - `host`      — stdio native-messaging host mode (no Tauri runtime)
//! - `server`    — local socket server inside the app
//! - `rules`     — the rules a captured page is filed by, and their commands
//!
//! The request and response types the three share are those notes accept
//! (`veydan_notes::capture::protocol`).

pub mod extension;
pub mod host;
pub mod rules;
pub mod server;

/// Gecko extension id; also the argv marker the browser passes to the native host.
pub const EXTENSION_ID: &str = "notes@veydan.net";
/// Native messaging host name (manifest file name and `sendNativeMessage` target).
pub const HOST_NAME: &str = "veydan_notes";

/// True when this process was started by the browser as the native messaging host.
pub fn is_host_invocation() -> bool {
    std::env::args().skip(1).any(|a| a == EXTENSION_ID)
}

/// Set by the app on the browsers it launches and inherited by the native host,
/// so the host reaches the app instance (`--workdir` profile) that owns the browser.
pub const ENDPOINT_ENV: &str = "VEYDAN_CAPTURE_ENDPOINT";
/// Set beside `ENDPOINT_ENV`: the token the native host shows the bridge on
/// every request. It never goes into the profile (its files are synced and
/// exported), so a browser started before the bridge had it cannot capture
/// until its profile is started again.
pub const TOKEN_ENV: &str = "VEYDAN_CAPTURE_TOKEN";

/// The secret of the bridge in the app data folder: 32 random bytes, made once.
const SECRET_FILE: &str = "capture.secret";
const SECRET_LEN: usize = 32;
/// The longest path a Unix socket takes (`sun_path` is 104 bytes on macOS and
/// the BSDs, 108 on Linux; one byte for the NUL).
#[cfg(not(windows))]
const SUN_PATH_MAX: usize = 103;

/// Where the app listens and what the native host must show it.
pub struct Bridge {
    pub endpoint: String,
    pub token: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The secret of `data_dir`, made on the first call (0600 on Unix). The lock
/// keeps the server and a launching browser from making two.
fn secret(data_dir: &std::path::Path) -> std::io::Result<[u8; SECRET_LEN]> {
    use std::io::Write;
    static MAKE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = MAKE.lock().unwrap_or_else(|e| e.into_inner());
    let path = data_dir.join(SECRET_FILE);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(secret) = <[u8; SECRET_LEN]>::try_from(bytes.as_slice()) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            return Ok(secret);
        }
    }
    let mut secret = [0u8; SECRET_LEN];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut secret);
    std::fs::create_dir_all(data_dir)?;
    let tmp = data_dir.join(format!("{SECRET_FILE}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp)?;
    file.write_all(&secret)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, &path)?;
    Ok(secret)
}

/// The endpoint and the token of the bridge of `data_dir`: the endpoint is
/// named by the first half of the secret, the token is the second half.
pub fn bridge(data_dir: &std::path::Path) -> std::io::Result<Bridge> {
    let secret = secret(data_dir)?;
    let suffix = veydan_shell::workdir::current()
        .map(|w| format!("-{}", w.tag))
        .unwrap_or_default();
    Ok(Bridge {
        endpoint: endpoint(data_dir, &suffix, &hex(&secret[..SECRET_LEN / 2])),
        token: hex(&secret[SECRET_LEN / 2..]),
    })
}

/// Per-user local IPC endpoint the app listens on; per profile under `--workdir`.
/// Its name carries a part of the secret: another process can neither guess
/// it nor take it first.
#[cfg(windows)]
fn endpoint(_data_dir: &std::path::Path, suffix: &str, name: &str) -> String {
    format!(r"\\.\pipe\veydan-capture{suffix}-{name}")
}

/// Per-user local socket the app listens on; per profile under `--workdir`.
/// It goes into `XDG_RUNTIME_DIR` (per user, 0700), else into the app data
/// folder, else into the temporary folder: the first whose path fits
/// `sun_path`, with the full name and then with a shorter one. The name
/// carries a part of the secret, as on Windows.
#[cfg(not(windows))]
fn endpoint(data_dir: &std::path::Path, suffix: &str, name: &str) -> String {
    let mut dirs: Vec<String> = Vec::new();
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            dirs.push(dir);
        }
    }
    dirs.push(data_dir.to_string_lossy().into_owned());
    dirs.push(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string()));
    let names = [
        format!("veydan-capture{suffix}-{name}.sock"),
        format!("vc{suffix}-{}.sock", &name[..name.len().min(16)]),
    ];
    let mut last = String::new();
    for dir in &dirs {
        for n in &names {
            let path = format!("{}/{n}", dir.trim_end_matches('/'));
            if path.len() <= SUN_PATH_MAX {
                return path;
            }
            last = path;
        }
    }
    last
}

/// Endpoint and token the native host uses: those its browser was launched
/// with. A browser started without them (before an update) has none.
pub fn host_bridge() -> Option<Bridge> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Some(Bridge {
        endpoint: var(ENDPOINT_ENV)?,
        token: var(TOKEN_ENV)?,
    })
}

/// The first bytes of every request line: the token in its envelope
/// `{"token":"<hex>","req":<request>}`. The bridge reads exactly these and
/// compares them before it parses anything.
pub fn envelope_head(token: &str) -> String {
    format!("{{\"token\":\"{token}\",\"req\":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_is_made_once_and_names_the_endpoint() {
        let dir = std::env::temp_dir().join(format!(
            "veydan-capture-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let a = bridge(&dir).unwrap();
        let b = bridge(&dir).unwrap();
        assert_eq!(a.endpoint, b.endpoint);
        assert_eq!(a.token, b.token);
        assert_eq!(a.token.len(), SECRET_LEN);
        assert!(a.token.bytes().all(|c| c.is_ascii_hexdigit()));
        let secret = std::fs::read(dir.join(SECRET_FILE)).unwrap();
        assert_eq!(secret.len(), SECRET_LEN);
        // The endpoint carries the first half; the token is the second.
        assert!(a.endpoint.contains(&hex(&secret[..8])), "{}", a.endpoint);
        assert_eq!(a.token, hex(&secret[16..]));
        assert!(!a.endpoint.contains(&a.token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(SECRET_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Another folder, another secret.
        let other = dir.join("other");
        assert_ne!(bridge(&other).unwrap().token, a.token);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_socket_path_fits_sun_path() {
        let name = "0123456789abcdef0123456789abcdef";
        // The test does not know XDG_RUNTIME_DIR: whatever directory is
        // chosen, the path carries the name and fits.
        let path = endpoint(std::path::Path::new("/d"), "-w1", name);
        assert!(path.len() <= SUN_PATH_MAX, "{path}");
        assert!(path.contains(&name[..16]), "{path}");
        assert!(path.ends_with(".sock"), "{path}");
        let long = std::path::PathBuf::from(format!("/{}", "x".repeat(120)));
        let path = endpoint(&long, "-w1", name);
        assert!(path.len() <= SUN_PATH_MAX, "{path}");
        assert!(path.contains(&name[..16]), "{path}");
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The boundary of the local file commands, whichever window asks
//! (platform-spec 5.2): they may not change the root of a file system, the
//! home directory or any folder above it, or the data of the app (the data
//! file, `camoufox/`, `tor/`, `profiles/`); the listing of the data folder is
//! refused too. Paths are compared once resolved, so `..` and links do not
//! lead around.

use crate::error::{AppError, CmdResult};
use std::path::{Path, PathBuf};

fn refuse(path: &Path, why: &str) -> AppError {
    AppError::other(format!("Refused: {} {why}", path.display()))
}

/// The path with the links of its folders resolved. The last name is kept
/// as it is: a link there is the object of the command (`fs_delete` removes
/// the link, not its target). The folder must exist.
fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            Ok(parent.canonicalize()?.join(name))
        }
        // A root, or a name ending in `..`: resolve the whole.
        _ => path.canonicalize(),
    }
}

fn home() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|h| !h.is_empty()))
        .and_then(|h| Path::new(&h).canonicalize().ok())
}

fn data_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .canonicalize()
        .unwrap_or_else(|_| app_data_dir.to_path_buf())
}

fn check_resolved(target: &Path, home: Option<&Path>, data: &Path) -> Result<(), String> {
    if target.parent().is_none() {
        return Err("is the root of a file system".into());
    }
    if home.is_some_and(|h| h.starts_with(target)) {
        return Err("is the home folder or holds it".into());
    }
    if target.starts_with(data) || data.starts_with(target) {
        return Err("is the data of the app or holds it".into());
    }
    Ok(())
}

fn check_with(path: &Path, home: Option<&Path>, data: &Path, follow: bool) -> CmdResult<()> {
    let target = if follow {
        path.canonicalize()
    } else {
        resolve(path)
    }
    .map_err(AppError::io)?;
    check_resolved(&target, home, &data_dir(data)).map_err(|why| refuse(path, &why))
}

/// For a command that creates, renames or removes `path`. The folder of
/// `path` must exist; `path` itself need not.
pub(crate) fn check_mutable(path: &Path, app_data_dir: &Path) -> CmdResult<()> {
    check_with(path, home().as_deref(), app_data_dir, false)
}

/// For a command that follows links (`chmod`): the target is what changes.
pub(crate) fn check_mutable_following(path: &Path, app_data_dir: &Path) -> CmdResult<()> {
    check_with(path, home().as_deref(), app_data_dir, true)
}

/// For a listing or a stat: free everywhere but in the data of the app.
pub(crate) fn check_readable(path: &Path, app_data_dir: &Path) -> CmdResult<()> {
    // A path that does not resolve fails by itself in the command.
    let Ok(target) = path.canonicalize() else {
        return Ok(());
    };
    if target.starts_with(data_dir(app_data_dir)) {
        return Err(refuse(path, "is the data of the app"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("veydan-fsguard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn root_home_and_its_ancestors_are_refused() {
        let base = tmp();
        let home = base.join("u/home");
        let data = base.join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let root = base.ancestors().last().unwrap().to_path_buf();
        for p in [&root, &home, &home.parent().unwrap().to_path_buf(), &base] {
            assert!(
                check_with(p, Some(&home), &data, false).is_err(),
                "{p:?} must be refused"
            );
        }
        // A folder inside the home is fine; so is a missing name in an existing folder.
        std::fs::create_dir_all(home.join("docs")).unwrap();
        assert!(check_with(&home.join("docs"), Some(&home), &data, false).is_ok());
        assert!(check_with(&home.join("docs/new.txt"), Some(&home), &data, false).is_ok());
        // `..` that climbs to the home is resolved.
        assert!(check_with(&home.join("docs/.."), Some(&home), &data, false).is_err());
        // A missing folder above the name: refused (nothing to resolve).
        assert!(check_with(&home.join("none/x"), Some(&home), &data, false).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_data_of_the_app_is_refused() {
        let base = tmp();
        let home = base.join("home");
        let data = home.join("data");
        std::fs::create_dir_all(data.join("profiles")).unwrap();
        for p in [
            data.clone(),
            data.join("profiles"),
            data.join("app.db"),
            data.join("profiles/p1"),
        ] {
            assert!(check_with(&p, Some(&home), &data, false).is_err(), "{p:?}");
        }
        assert!(check_with(&home.join("other"), Some(&home), &data, false).is_ok());
        assert!(check_readable(&data.join("profiles"), &data).is_err());
        assert!(check_readable(&data, &data).is_err());
        assert!(check_readable(&home, &data).is_ok());
        assert!(check_readable(&home.join("missing"), &data).is_ok());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_judged_by_its_folder_not_its_target() {
        let base = tmp();
        let home = base.join("home");
        let data = base.join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        // A link to the data inside an ordinary folder: deleting the link is fine…
        let link = home.join("l");
        std::os::unix::fs::symlink(&data, &link).unwrap();
        assert!(check_with(&link, Some(&home), &data, false).is_ok());
        // …but going through it, or following it, is not.
        assert!(check_with(&link.join("app.db"), Some(&home), &data, false).is_err());
        assert!(check_with(&link, Some(&home), &data, true).is_err());
        assert!(check_readable(&link, &data).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}

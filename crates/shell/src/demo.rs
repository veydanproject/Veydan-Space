// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The demo data of a product, put together from the parts of its modules,
//! and the wipe of everything they keep. The driver knows no module: it goes
//! over `Module::demo` of the product's list.

use crate::module::DemoPart;
use crate::services::Shell;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};
use veydan_core::{AppError, CmdResult, Core};
use veydan_lock::Lock;
use veydan_sync_host::SyncManager;

/// The lock the demo data stands behind: a password, named in the demo notes.
pub const DEMO_LOCK: &str = "demo";
/// How long a command waits for a running cycle of sync to end.
const SYNC_WAIT: Duration = Duration::from_secs(30);

/// Empty what every part keeps. A module comes after those whose data it
/// refers to, so the parts are cleared from the last one back. The lock and
/// its key stay; the positions in the logs and the states of rows stay too,
/// so the next cycle publishes the tombstones of what disappeared — and of
/// the labels of what the owners here had.
pub async fn clear<R: Runtime>(app: &AppHandle<R>, parts: &[DemoPart<R>]) -> CmdResult<()> {
    for part in parts.iter().rev() {
        (part.clear)(app.clone()).await?;
    }
    let core = app.state::<Core>();
    core.directory.republish_all().await?;
    veydan_sync_host::gc::forget_candidates(&core.db).await
}

/// Replace everything with the demo data of `locale`: the parts are cleared,
/// the key and the secret boxes go, the lock becomes [`DEMO_LOCK`] over a
/// new key, then the parts are seeded in the order of the product's list —
/// a module after those it links to. A product none of whose modules has
/// demo data (Chat) has nothing to replace: its lock, and what stands behind
/// the lock's key (the messenger's key), are left as they are.
pub async fn seed<R: Runtime>(
    app: &AppHandle<R>,
    parts: &[DemoPart<R>],
    locale: &str,
) -> CmdResult<()> {
    if parts.is_empty() {
        return Ok(());
    }
    clear(app, parts).await?;
    let lock = app.state::<Lock>();
    lock.wipe().await?;
    lock.set(Some(DEMO_LOCK.into()), None, Some("password".into()), None)
        .await?;
    for part in parts {
        (part.seed)(app.clone(), locale.to_owned()).await?;
    }
    // The parts write their rows themselves; the owners name them all at once.
    app.state::<Core>().directory.republish_all().await?;
    Ok(())
}

/// Wipe what the modules keep: the rows, the folders and files of notes and
/// profiles, the settings of the modules.
#[tauri::command]
pub async fn app_clear_data(app: AppHandle, sync: tauri::State<'_, SyncManager>) -> CmdResult<()> {
    let guard = sync.pause(SYNC_WAIT).await;
    if guard.is_none() {
        return Err(AppError::other("sync is running, try again"));
    }
    let parts = app.state::<Shell>().demo_parts();
    let result = clear(&app, &parts).await;
    drop(guard);
    veydan_sync_host::trigger_cycle(&app, "clear_data");
    result
}

/// Wipe, then load the demo data of `locale` (`ru` or `en`). Runs with the
/// sync slot held, then pushes right away, so the fresh rows carry a newer
/// clock than any tombstone another device may still publish.
#[tauri::command]
pub async fn demo_seed(
    locale: String,
    app: AppHandle,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<()> {
    let guard = sync.pause(SYNC_WAIT).await;
    if guard.is_none() {
        return Err(AppError::other("sync is running, try again"));
    }
    let parts = app.state::<Shell>().demo_parts();
    let result = seed(&app, &parts, &locale).await;
    drop(guard);
    veydan_sync_host::trigger_cycle(&app, "demo_seed");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tauri::test::MockRuntime;
    use veydan_core::{db, Directory};

    /// What the parts did, in order.
    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);

    fn note(app: &AppHandle<MockRuntime>, line: String) {
        app.state::<Log>().0.lock().unwrap().push(line);
    }

    fn part_a() -> DemoPart<MockRuntime> {
        DemoPart {
            seed: |app, locale| {
                Box::pin(async move {
                    // The key is open behind the demo lock while a part seeds.
                    let (_key, vault) = app.state::<Lock>().require_open()?;
                    assert!(!vault.is_empty());
                    note(&app, format!("seed a {locale}"));
                    Ok(())
                })
            },
            clear: |app| {
                Box::pin(async move {
                    note(&app, "clear a".into());
                    Ok(())
                })
            },
        }
    }

    fn part_b() -> DemoPart<MockRuntime> {
        DemoPart {
            seed: |app, locale| {
                Box::pin(async move {
                    note(&app, format!("seed b {locale}"));
                    Ok(())
                })
            },
            clear: |app| {
                Box::pin(async move {
                    note(&app, "clear b".into());
                    Ok(())
                })
            },
        }
    }

    async fn app() -> (tauri::App<MockRuntime>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let schemas = [
            veydan_core::SCHEMA,
            crate::lock::SCHEMA,
            veydan_sync_host::SCHEMA,
        ];
        let pool = db::open(&dir.path().join(db::DB_FILE), &schemas)
            .await
            .unwrap();
        let app = tauri::test::mock_app();
        app.manage(Lock::new(pool.clone()));
        app.manage(Core::new(pool, dir.path().to_owned(), Directory::default()));
        app.manage(Log::default());
        (app, dir)
    }

    fn log(app: &tauri::App<MockRuntime>) -> Vec<String> {
        std::mem::take(&mut app.state::<Log>().0.lock().unwrap())
    }

    #[tokio::test]
    async fn the_parts_are_cleared_from_the_last_and_seeded_from_the_first() {
        let (app, _dir) = app().await;
        let parts = [part_a(), part_b()];

        clear(app.handle(), &parts).await.unwrap();
        assert_eq!(log(&app), ["clear b", "clear a"]);

        seed(app.handle(), &parts, "ru").await.unwrap();
        assert_eq!(log(&app), ["clear b", "clear a", "seed a ru", "seed b ru"]);
    }

    #[tokio::test]
    async fn the_demo_data_stands_behind_the_demo_lock() {
        let (app, _dir) = app().await;
        let lock = app.state::<Lock>();
        lock.open_default().await.unwrap();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        lock.secret_box("messenger")
            .put("nsec", b"k")
            .await
            .unwrap();
        let before = veydan_lock::key_id(&app.state::<Core>().db).await.unwrap();

        seed(app.handle(), &[part_a()], "en").await.unwrap();

        let status = lock.status().await;
        assert!(status.enabled && !status.locked);
        assert_eq!(status.kind, "password");
        assert_ne!(
            veydan_lock::key_id(&app.state::<Core>().db).await.unwrap(),
            before
        );
        assert!(lock
            .secret_box("messenger")
            .get("nsec")
            .await
            .unwrap()
            .is_none());
        lock.lock();
        assert!(lock.unlock("4821").await.is_err());
        assert!(lock.unlock(DEMO_LOCK).await.unwrap());
    }

    #[tokio::test]
    async fn a_product_without_demo_data_keeps_its_lock_and_its_secrets() {
        let (app, _dir) = app().await;
        let lock = app.state::<Lock>();
        lock.open_default().await.unwrap();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        lock.secret_box("messenger")
            .put("nsec", b"k")
            .await
            .unwrap();
        let before = veydan_lock::key_id(&app.state::<Core>().db).await.unwrap();

        seed(app.handle(), &[], "en").await.unwrap();

        assert_eq!(
            veydan_lock::key_id(&app.state::<Core>().db).await.unwrap(),
            before
        );
        let kept = lock.secret_box("messenger").get("nsec").await.unwrap();
        assert_eq!(kept.as_deref().map(Vec::as_slice), Some(&b"k"[..]));
        lock.lock();
        assert!(lock.unlock("4821").await.unwrap());
    }

    #[tokio::test]
    async fn a_part_that_fails_stops_the_seed() {
        let (app, _dir) = app().await;
        let failing = DemoPart {
            seed: |_app, _locale| Box::pin(async { Err(AppError::other("no room")) }),
            ..part_b()
        };
        let refused = seed(app.handle(), &[failing, part_a()], "en").await;
        assert!(matches!(refused, Err(AppError::Other(m)) if m == "no room"));
        assert_eq!(log(&app), ["clear a", "clear b"]);
    }
}

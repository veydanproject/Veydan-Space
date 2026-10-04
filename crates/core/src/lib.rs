// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What every module stands on and no module owns: the error type of the
//! commands, the data file with the schemas of the modules, the settings
//! table, deletion hooks and the entity directory.
//!
//! The core knows no module: a product hands it the schemas, the providers
//! and the hooks of the modules it is built from.

pub mod db;
pub mod deletions;
pub mod directory;
pub mod error;
pub mod settings;

pub use db::{Schema, SCHEMA};
pub use deletions::{DeletionHook, Deletions};
pub use directory::{
    label_key, like_pattern, Action, Directory, Label, Provider, LABELS_TABLE, LABEL_COLUMNS,
};
pub use error::{AppError, CmdResult};
pub use settings::Settings;

use sqlx::{Pool, Sqlite};
use std::path::PathBuf;

/// A boxed future, the return type of hooks kept as plain function pointers.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The state every module shares; managed by the app before any module is
/// set up: `app.state::<veydan_core::Core>()`.
pub struct Core {
    pub db: Pool<Sqlite>,
    /// The data directory: `app.db`, `install.id` and the data folders of the
    /// modules live in it.
    pub app_data_dir: PathBuf,
    pub settings: Settings,
    pub deletions: Deletions,
    pub directory: Directory,
}

impl Core {
    /// `directory` arrives with the providers of the product's modules in it;
    /// it keeps its labels in this data file.
    pub fn new(db: Pool<Sqlite>, app_data_dir: PathBuf, mut directory: Directory) -> Self {
        directory.keep_labels(db.clone());
        Self {
            settings: Settings::new(db.clone()),
            db,
            app_data_dir,
            deletions: Deletions::default(),
            directory,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_parts_of_the_core_share_the_data_file() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(&dir.path().join(db::DB_FILE), &[SCHEMA])
            .await
            .unwrap();
        let core = Core::new(pool, dir.path().to_owned(), Directory::default());

        core.settings.set("ui_locale", "ru").await.unwrap();
        assert_eq!(
            settings::get(&core.db, "ui_locale").await.as_deref(),
            Some("ru")
        );
        assert_eq!(core.app_data_dir, dir.path());
        assert!(core.directory.kinds().is_empty());
        core.db.close().await;
    }
}

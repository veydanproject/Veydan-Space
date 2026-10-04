// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A start that could not open the app's data.
//!
//! The window still opens so the UI can say why, but neither `Core` nor
//! `Shell` nor any state of a module is managed, and no module is set up.
//! `app_start_error` tells the reason; the router turns every other command
//! away with the same value before its body runs, so none of them meets the
//! missing state.

use serde::Serialize;
use std::sync::OnceLock;

/// Why the app runs without its data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum StartError {
    /// The file at `path` — the data file, or a journal beside the place of
    /// an absent one — was not created by this app; it is neither opened nor
    /// modified.
    DbForeign { path: String },
    /// The data file at `path` is this app's, but its schema is of another
    /// build; it is neither opened nor modified.
    DbDevSchema { path: String },
}

/// Managed before the app is built, so `app_start_error` answers in any start.
#[derive(Default)]
pub struct StartState(OnceLock<StartError>);

impl StartState {
    pub fn fail(&self, error: StartError) {
        let _ = self.0.set(error);
    }

    pub fn error(&self) -> Option<StartError> {
        self.0.get().cloned()
    }
}

/// The one command that is answered in a start without data.
pub(crate) const STATUS_COMMAND: &str = "app_start_error";

pub mod commands {
    use super::{StartError, StartState};

    /// `null` after a normal start, otherwise the reason there is no data.
    #[tauri::command]
    pub fn app_start_error(state: tauri::State<'_, StartState>) -> Option<StartError> {
        state.error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_is_an_object_with_code_and_path() {
        let error = StartError::DbForeign {
            path: "/home/u/.local/share/net.veydan.space/data/app.db".into(),
        };
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"db_foreign","path":"/home/u/.local/share/net.veydan.space/data/app.db"}"#
        );
        let error = StartError::DbDevSchema {
            path: "/data/app.db".into(),
        };
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"db_dev_schema","path":"/data/app.db"}"#
        );
    }

    #[test]
    fn a_normal_start_answers_null() {
        let state = StartState::default();
        assert_eq!(serde_json::to_string(&state.error()).unwrap(), "null");
        state.fail(StartError::DbForeign { path: "p".into() });
        assert_eq!(
            state.error(),
            Some(StartError::DbForeign { path: "p".into() })
        );
    }
}

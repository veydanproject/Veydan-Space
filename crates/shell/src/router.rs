// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! One `invoke_handler` for a product assembled from modules.
//!
//! Each module builds its own `tauri::generate_handler!`; the router keeps a
//! table from the name of a command to the module that declared it and hands
//! the call to that module's handler.

use std::collections::HashMap;

use tauri::ipc::Invoke;
use tauri::{Runtime, Wry};

use crate::module::Handler;
use crate::start::{StartState, STATUS_COMMAND};

/// The command table of a product.
pub struct Router<R: Runtime = Wry> {
    product: &'static str,
    by_command: HashMap<&'static str, usize>,
    handlers: Vec<(&'static str, Handler<R>)>,
}

/// Two modules of a product declare one command name.
#[derive(Debug, PartialEq, Eq)]
pub struct DuplicateCommand {
    pub command: &'static str,
    pub first: &'static str,
    pub second: &'static str,
}

impl std::fmt::Display for DuplicateCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "command `{}` is declared by module `{}` and by module `{}`",
            self.command, self.first, self.second
        )
    }
}

impl<R: Runtime> Router<R> {
    /// `parts`: the id, the command names and the handler of each module.
    /// `product` is named in the answer to an unknown command.
    pub fn new(
        product: &'static str,
        parts: Vec<(&'static str, &'static [&'static str], Handler<R>)>,
    ) -> Result<Self, DuplicateCommand> {
        let mut by_command = HashMap::new();
        let mut handlers: Vec<(&'static str, Handler<R>)> = Vec::with_capacity(parts.len());
        for (index, (module, commands, handler)) in parts.into_iter().enumerate() {
            handlers.push((module, handler));
            for command in commands {
                if let Some(previous) = by_command.insert(*command, index) {
                    return Err(DuplicateCommand {
                        command,
                        first: handlers[previous].0,
                        second: module,
                    });
                }
            }
        }
        Ok(Self {
            product,
            by_command,
            handlers,
        })
    }

    /// The module that declared `command`.
    pub fn module_of(&self, command: &str) -> Option<&'static str> {
        self.by_command.get(command).map(|i| self.handlers[*i].0)
    }

    /// Every command with its module, ordered by the name of the command.
    pub fn table(&self) -> Vec<(&'static str, &'static str)> {
        let mut all: Vec<_> = self
            .by_command
            .iter()
            .map(|(command, i)| (*command, self.handlers[*i].0))
            .collect();
        all.sort_unstable();
        all
    }

    /// The single `invoke_handler` of the product. `tauri::ipc::Invoke` is
    /// documented as unstable; the two things the router needs of it — the
    /// name of the command and the resolver — are used here and nowhere else.
    pub fn dispatch(&self, invoke: Invoke<R>) -> bool {
        // The index is `Copy`, so the borrow of `invoke.message` ends before
        // `invoke` moves into the module's handler.
        let command = invoke.message.command();
        if command != STATUS_COMMAND {
            let error = invoke
                .message
                .state_ref()
                .try_get::<StartState>()
                .and_then(|state| state.error());
            if let Some(error) = error {
                invoke.resolver.reject(error);
                return true;
            }
        }
        match self.by_command.get(command).copied() {
            // A `false` from here means the module declared a name its own
            // handler does not match; Tauri then answers "Command X not found".
            Some(index) => (self.handlers[index].1)(invoke),
            None => {
                let message = unknown_command(self.product, command);
                invoke.resolver.reject(message);
                true
            }
        }
    }
}

/// The answer to a command no module of `product` declares. It opens with
/// the words of Tauri's own answer, "Command x not found": by them the UI of
/// a module tells that the product was built without it.
pub fn unknown_command(product: &str, command: &str) -> String {
    format!("Command `{command}` not found: no module of product `{product}` declares it")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::start::{StartError, StartState};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{
        get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
    };
    use tauri::webview::InvokeRequest;

    /// Stands for the state of a module: a state no start of these tests manages.
    struct Unmanaged;

    #[tauri::command]
    fn note_ping() -> &'static str {
        "notes"
    }

    #[tauri::command]
    fn with_state(_state: tauri::State<'_, Unmanaged>) -> u8 {
        1
    }

    #[tauri::command]
    fn password_ping() -> &'static str {
        "pass"
    }

    /// Looks the state up on its own, as some commands do; panics without it.
    #[tauri::command]
    fn with_handle<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> u8 {
        use tauri::Manager;
        let _ = app.state::<Unmanaged>();
        1
    }

    type Part = (&'static str, &'static [&'static str], Handler<MockRuntime>);

    /// A module's part that counts the calls its handler gets.
    fn counted(
        id: &'static str,
        commands: &'static [&'static str],
        handler: Handler<MockRuntime>,
    ) -> (Part, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let handler: Handler<MockRuntime> = Box::new(move |invoke| {
            seen.fetch_add(1, Ordering::Relaxed);
            handler(invoke)
        });
        ((id, commands, handler), calls)
    }

    fn silent(id: &'static str, commands: &'static [&'static str]) -> Part {
        (id, commands, Box::new(|_| true))
    }

    struct Probe {
        webview: tauri::WebviewWindow<MockRuntime>,
        app: tauri::App<MockRuntime>,
        notes_calls: Arc<AtomicUsize>,
        pass_calls: Arc<AtomicUsize>,
    }

    /// An app whose commands go through a router of the status command and
    /// two modules.
    fn probe() -> Probe {
        let (notes, notes_calls) = counted(
            "notes",
            &["note_ping", "with_state"],
            Box::new(tauri::generate_handler![note_ping, with_state]),
        );
        let (pass, pass_calls) = counted(
            "pass",
            &["password_ping", "with_handle"],
            Box::new(tauri::generate_handler![password_ping, with_handle]),
        );
        let shell: Part = (
            "shell",
            &["app_start_error"],
            Box::new(tauri::generate_handler![
                crate::start::commands::app_start_error
            ]),
        );
        let router = Router::new("probe", vec![shell, notes, pass]).unwrap();
        let app = mock_builder()
            .manage(StartState::default())
            .invoke_handler(move |invoke| router.dispatch(invoke))
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        Probe {
            webview,
            app,
            notes_calls,
            pass_calls,
        }
    }

    fn invoke(webview: &tauri::WebviewWindow<MockRuntime>, cmd: &str) -> Result<Value, Value> {
        let url = if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        get_ipc_response(
            webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: url.parse().unwrap(),
                body: InvokeBody::default(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().unwrap())
    }

    #[test]
    fn a_command_reaches_the_module_that_declared_it_and_no_other() {
        let probe = probe();
        assert_eq!(invoke(&probe.webview, "note_ping"), Ok(json!("notes")));
        assert_eq!(probe.notes_calls.load(Ordering::Relaxed), 1);
        assert_eq!(probe.pass_calls.load(Ordering::Relaxed), 0);

        assert_eq!(invoke(&probe.webview, "password_ping"), Ok(json!("pass")));
        assert_eq!(probe.notes_calls.load(Ordering::Relaxed), 1);
        assert_eq!(probe.pass_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn an_unknown_command_is_rejected_by_name() {
        let probe = probe();
        assert_eq!(
            invoke(&probe.webview, "no_such_command"),
            Err(json!(
                "Command `no_such_command` not found: no module of product `probe` declares it"
            ))
        );
        assert_eq!(probe.notes_calls.load(Ordering::Relaxed), 0);
        assert_eq!(probe.pass_calls.load(Ordering::Relaxed), 0);
    }

    /// The UI of a module asks its status first and takes an answer that
    /// says "command … not found", in any case, for a product built without
    /// the module; the product's tests hold each UI's pattern to it.
    #[test]
    fn a_product_without_a_module_answers_its_status_in_the_words_its_ui_reads() {
        let probe = probe();
        let Err(Value::String(answer)) = invoke(&probe.webview, "probe_status") else {
            panic!("probe_status was answered in a product without its module");
        };
        let answer = answer.to_lowercase();
        let (_, after) = answer.split_once("command ").expect(&answer);
        assert!(after.contains(" not found"), "{answer}");
    }

    #[test]
    fn after_a_failed_start_only_the_reason_can_be_asked() {
        use tauri::Manager;
        let probe = probe();

        // A normal start: no reason, and the commands are reached.
        assert_eq!(invoke(&probe.webview, "app_start_error"), Ok(Value::Null));
        let unmanaged = invoke(&probe.webview, "with_state").unwrap_err();
        assert!(
            unmanaged.as_str().is_some_and(|m| m.contains("state")),
            "{unmanaged}"
        );
        let calls = probe.notes_calls.load(Ordering::Relaxed);

        probe.app.state::<StartState>().fail(StartError::DbForeign {
            path: "/data/app.db".into(),
        });
        let reason = json!({ "code": "db_foreign", "path": "/data/app.db" });
        assert_eq!(
            invoke(&probe.webview, "app_start_error"),
            Ok(reason.clone())
        );
        // Neither command runs: the second would panic on the missing state.
        assert_eq!(invoke(&probe.webview, "with_state"), Err(reason.clone()));
        assert_eq!(invoke(&probe.webview, "with_handle"), Err(reason.clone()));
        assert_eq!(invoke(&probe.webview, "note_ping"), Err(reason.clone()));
        assert_eq!(invoke(&probe.webview, "no_such_command"), Err(reason));
        // No handler of a module was even called.
        assert_eq!(probe.notes_calls.load(Ordering::Relaxed), calls);
        assert_eq!(probe.pass_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn two_modules_with_one_command_are_refused() {
        let error = Router::new(
            "probe",
            vec![silent("a", &["ping", "x"]), silent("b", &["y", "ping"])],
        )
        .err()
        .unwrap();
        assert_eq!(
            error,
            DuplicateCommand {
                command: "ping",
                first: "a",
                second: "b"
            }
        );
        assert_eq!(
            error.to_string(),
            "command `ping` is declared by module `a` and by module `b`"
        );
    }

    #[test]
    fn a_command_listed_twice_by_one_module_is_refused() {
        let error = Router::new("probe", vec![silent("a", &["ping", "ping"])])
            .err()
            .unwrap();
        assert_eq!(
            error,
            DuplicateCommand {
                command: "ping",
                first: "a",
                second: "a"
            }
        );
    }

    #[test]
    fn the_table_names_the_module_of_every_command() {
        let router = Router::new(
            "probe",
            vec![silent("a", &["b_first"]), silent("b", &["a_second"])],
        )
        .unwrap();
        assert_eq!(router.module_of("b_first"), Some("a"));
        assert_eq!(router.module_of("a_second"), Some("b"));
        assert_eq!(router.module_of("nope"), None);
        assert_eq!(router.table(), vec![("a_second", "b"), ("b_first", "a")]);
    }
}

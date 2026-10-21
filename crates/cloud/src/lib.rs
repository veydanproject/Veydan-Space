// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The module Cloud: Veydan Cloud in the app (internal/vcloud-plan.md 2.2).
//!
//! The account, its plan, limits and usage, the storage of the vault and
//! the recovery of an account, the approval of sign-ins to the web
//! cabinet (`weblogin`), for every product that has the messenger
//! (`requires` in products.json): Space shows it as a tab, Chat from its
//! settings. The module is on while the messenger is (`Switch::Follows`).
//!
//! It takes what it needs of the messenger — signing with the identity,
//! service notes, the cards of the vcloud bot — through the seams of the
//! shell (`veydan_shell::seams`), never through the messenger's crate or
//! tables, and offers its client of Veydan Cloud, [`CloudApi`], the same
//! way. It keeps no tables: the account it knows lives in the box `cloud`
//! of the lock (`account`). The client is [`HttpCloud`] over the seams
//! `Http` and `Signer` of the messenger; a product without them gets
//! [`Unconfigured`].

pub mod account;
pub mod activate;
pub mod api;
pub mod cancel;
pub mod delete;
pub mod escrow;
#[cfg(test)]
mod fake;
pub mod link;
pub mod nip98;
pub mod recovery;
pub mod reissue;
pub mod request;
pub mod restore;
pub mod social;
#[cfg(test)]
mod social_tests;
mod status;
pub mod sync;
pub mod trustee;
pub mod weblogin;

pub use api::{CloudApi, HttpCloud, Unconfigured};
pub use status::CloudStatus;

use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};
use veydan_shell::seams::{Http, Signer};
use veydan_shell::{Module, SetupResult, Switch};

/// The id of the module, as products.json lists it.
pub const MODULE_ID: &str = "cloud";

/// The module the cloud follows and requires: it signs and carries for it.
pub const FOLLOWS: &str = "messenger";

pub fn module() -> Module {
    Module {
        switch: Switch::Follows(FOLLOWS),
        start: Some(sync::start),
        ..veydan_shell::module! {
            id: "cloud",
            setup: setup,
            commands: [
                status::cloud_status,
                sync::cloud_usage,
                activate::cloud_activate_free,
                activate::cloud_redeem,
                account::cloud_account_refresh,
                account::cloud_limits,
                account::cloud_plans,
                delete::cloud_delete_account,
                recovery::cloud_recovery_key_show_once,
                recovery::cloud_recovery_key_reissue,
                recovery::cloud_recovery_status,
                recovery::cloud_restore_peek,
                recovery::cloud_restore_with_words,
                link::cloud_link_start,
                link::cloud_link_poll,
                link::cloud_link_receive,
                link::cloud_link_resolve,
                link::cloud_link_approve,
                link::cloud_link_cancel,
                social::cloud_trustees_status,
                social::cloud_contacts_approved,
                social::cloud_trustees_setup,
                social::cloud_trustees_disable,
                social::cloud_trustees_check,
                trustee::cloud_help_lookup,
                trustee::cloud_help_confirm,
                trustee::cloud_help_close,
                request::cloud_recover_start,
                request::cloud_recover_poll,
                request::cloud_recover_peek,
                request::cloud_recover_finish,
                request::cloud_recover_cancel,
                cancel::cloud_recovery_alerts,
                cancel::cloud_recovery_cancel,
                cancel::cloud_recovery_alert_dismiss,
                weblogin::cloud_web_login_inspect,
                weblogin::cloud_web_login_approve,
                weblogin::cloud_web_login_deny,
                weblogin::cloud_web_confirm,
                weblogin::cloud_web_unfreeze,
                weblogin::cloud_web_sessions,
                weblogin::cloud_web_sessions_revoke,
            ],
        }
    }
}

/// Offers the client of Veydan Cloud to the shell, keeps the state of an
/// activation, asks for a kept account once a day and listens to the
/// cards of the vcloud bot.
fn setup(app: &mut tauri::App) -> SetupResult {
    offer(app.handle());
    sync::register(app.handle());
    app.manage(activate::Activation::default());
    app.manage(Arc::new(link::Sessions::default()));
    app.manage(Arc::new(request::Sessions::default()));
    app.manage(Arc::new(escrow::ReadEscrows::default()));
    app.manage(trustee::HelpState::default());
    account::start_daily(app.handle().clone());
    delete::listen_cards(app.handle().clone());
    recovery::watch(app.handle().clone());
    // The vault of this device for a link: what A sends, how B enters.
    let vault: Arc<dyn link::VaultEntry> = Arc::new(recovery::AppVault::new(app.handle().clone()));
    veydan_shell::seams::provide(app.handle(), vault);
    recovery::start_daily(app.handle().clone());
    social::listen(app.handle().clone());
    social::start_daily(app.handle().clone());
    // The camera reads the QR code of a new device ("Add a device"). The
    // module pass reads TOTP secrets with the same plugin: whichever is set
    // up first registers it.
    #[cfg(mobile)]
    if app.try_state::<tauri_plugin_barcode_scanner::BarcodeScanner<tauri::Wry>>().is_none() {
        app.handle().plugin(tauri_plugin_barcode_scanner::init())?;
    }
    Ok(())
}

/// Puts the client in place for the other modules, on any runtime: over
/// the messenger's seams when the product has them.
fn offer<R: Runtime, M: Manager<R>>(manager: &M) {
    let http = veydan_shell::seam::<dyn Http, _, _>(manager);
    let signer = veydan_shell::seam::<dyn Signer, _, _>(manager);
    let client: Arc<dyn CloudApi> = match (http, signer) {
        (Some(http), Some(signer)) => Arc::new(HttpCloud::new(http, signer)),
        _ => Arc::new(Unconfigured),
    };
    veydan_shell::provide::<dyn CloudApi, _, _>(manager, client);
}

/// The client the module offered, or [`Unconfigured`] in an app that has none.
pub fn client<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn CloudApi> {
    veydan_shell::seam::<dyn CloudApi, _, _>(app).unwrap_or_else(|| Arc::new(Unconfigured))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_follows_the_messenger_and_has_its_commands() {
        let module = module();
        assert_eq!(module.id, "cloud");
        assert_eq!(module.id, MODULE_ID);
        assert_eq!(module.switch, Switch::Follows("messenger"));
        assert_eq!(
            module.commands,
            &[
                "cloud_status",
                "cloud_usage",
                "cloud_activate_free",
                "cloud_redeem",
                "cloud_account_refresh",
                "cloud_limits",
                "cloud_plans",
                "cloud_delete_account",
                "cloud_recovery_key_show_once",
                "cloud_recovery_key_reissue",
                "cloud_recovery_status",
                "cloud_restore_peek",
                "cloud_restore_with_words",
                "cloud_link_start",
                "cloud_link_poll",
                "cloud_link_receive",
                "cloud_link_resolve",
                "cloud_link_approve",
                "cloud_link_cancel",
                "cloud_trustees_status",
                "cloud_contacts_approved",
                "cloud_trustees_setup",
                "cloud_trustees_disable",
                "cloud_trustees_check",
                "cloud_help_lookup",
                "cloud_help_confirm",
                "cloud_help_close",
                "cloud_recover_start",
                "cloud_recover_poll",
                "cloud_recover_peek",
                "cloud_recover_finish",
                "cloud_recover_cancel",
                "cloud_recovery_alerts",
                "cloud_recovery_cancel",
                "cloud_recovery_alert_dismiss",
                "cloud_web_login_inspect",
                "cloud_web_login_approve",
                "cloud_web_login_deny",
                "cloud_web_confirm",
                "cloud_web_unfreeze",
                "cloud_web_sessions",
                "cloud_web_sessions_revoke",
            ]
        );
        // No tables, nothing synced; the box `cloud` is not a key user.
        assert!(module.schema.is_none() && module.sync.is_none() && module.key_user.is_none());
        assert!(module.demo.is_none() && module.backup.is_none() && module.tray.is_none());
    }

    /// The setup offers the client to the shell, and another module finds
    /// it there by its trait alone.
    #[tokio::test]
    async fn the_setup_offers_the_client_through_the_shell() {
        let app = tauri::test::mock_app();
        assert!(veydan_shell::seam::<dyn CloudApi, _, _>(&app).is_none());
        offer(&app);
        let api = veydan_shell::seam::<dyn CloudApi, _, _>(&app).expect("offered");
        let e = api.account().await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(veydan_shell::seams::code::NO_HOST));
    }

    /// With the messenger's seams in place the client is the real one: it
    /// reaches the host they name.
    #[tokio::test]
    async fn with_the_seams_of_the_messenger_the_client_goes_to_the_host() {
        let fake = fake::Fake::start(2).await;
        let app = fake.app();
        fake.add_account();
        let account = client(app.handle()).account().await.expect("account");
        assert_eq!(account.plan, "free");
        assert_eq!(fake.requests(), 1);
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Deleting the account (vcloud-plan.md 4.6, cloud-api.md 4.4): the word
//! DELETE and the lock on the screen, `DELETE /cloud/v1/account?confirm=
//! DELETE` signed by the identity, then this device unbinds — the box
//! `cloud` erased — and shows the receipt. Another device of the same
//! identity unbinds on the card `account.deleted` of the vcloud bot.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use veydan_shell::seams::{self, BotCards, Card, CloudError, LockBox, LockConfirm, SeamFuture};
use zeroize::Zeroizing;

use crate::account;
use crate::activate::Activation;
use crate::api::{CloudApi, Deleted};

/// The event a device sends its screens when its account is gone, with a
/// [`DeleteReceipt`]: the sync backend `cloud` and the pages let go of it.
pub const EVENT_DELETED: &str = "cloud://account-deleted";

/// The word the screen asks for, and the query of the request carries.
pub const CONFIRM_WORD: &str = "DELETE";

/// The card of the bot that tells the other devices (cloud-recovery.md §10).
pub const CARD_DELETED: &str = "account.deleted";

/// A local refusal of the module: a lock is set and the secret given is
/// not its own.
pub const LOCK_WRONG: &str = "lock_wrong";

/// The check of the lock before a dangerous action: the shell's lock in
/// the app ([`seams::lock_confirm`]), a fake one in the tests.
pub trait LockGate: Send + Sync {
    /// What the lock says to `secret`, changing nothing.
    fn confirm<'a>(&'a self, secret: Option<&'a str>) -> SeamFuture<'a, LockConfirm>;
}

/// [`LockGate`] over the lock the shell runs.
struct AppLockGate<R: Runtime>(AppHandle<R>);

impl<R: Runtime> LockGate for AppLockGate<R> {
    fn confirm<'a>(&'a self, secret: Option<&'a str>) -> SeamFuture<'a, LockConfirm> {
        Box::pin(seams::lock_confirm(&self.0, secret))
    }
}

/// The gate offered as a seam (a test's), else the app's lock.
pub(crate) fn lock_gate<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn LockGate> {
    veydan_shell::seam::<dyn LockGate, _, _>(app)
        .unwrap_or_else(|| Arc::new(AppLockGate(app.clone())))
}

/// What the screen shows after the account is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteReceipt {
    pub deleted_at: u64,
    /// The number for the stores: 12 Crockford characters in three groups
    /// of four, 14 with the dashes (`7K2M-ABCD-EFGH`).
    pub receipt: String,
    /// How many trusted people were told to drop their share: none until
    /// the shares exist (Social Recovery, stage C5).
    pub shares_revoked: u32,
    /// Learnt from the card of the bot, not deleted from this device.
    pub from_card: bool,
}

/// Whether the word typed is the confirmation, spaces around it aside.
pub fn confirmed(word: &str) -> bool {
    word.trim() == CONFIRM_WORD
}

/// The keys of the box `cloud` the account leaves behind besides itself:
/// the Recovery Key and its generation, the state of the escrow, a reissue
/// under way, the escrows it retired and the trusted people. A new
/// activation of the same npub starts from nothing (the first issue, not
/// `already_set_up`). The shares this device keeps for others lie in a box
/// of their own (`recovery_shares`) and stay.
pub const RECOVERY_KEYS: [&str; 6] = [
    crate::escrow::KEY_RK,
    crate::escrow::KEY_GENERATION,
    crate::escrow::KEY_ESCROW_STATE,
    crate::reissue::KEY_PENDING,
    crate::reissue::KEY_RETIRED,
    crate::social::KEY_TRUSTEES,
];

/// Forgets the account in the box: what recovery kept of it, then the
/// account itself (it works while the lock is closed).
pub async fn erase_box(boxed: &dyn LockBox) -> Result<(), CloudError> {
    for key in RECOVERY_KEYS {
        boxed.delete(key).await?;
    }
    account::erase(boxed).await
}

/// Deletes the account on the server and forgets it here: the receipt of
/// the server, then the box erased ([`erase_box`]).
pub async fn delete_account(
    api: &dyn CloudApi,
    boxed: &dyn LockBox,
) -> Result<Deleted, CloudError> {
    let deleted = api.delete().await?;
    erase_box(boxed).await?;
    Ok(deleted)
}

/// This device lets go of the account: nothing kept, no activation shown,
/// and the screens told.
async fn unbind<R: Runtime>(app: &AppHandle<R>, receipt: &DeleteReceipt) {
    let _ = erase_box(account::boxed(app).as_ref()).await;
    if let Some(activation) = app.try_state::<Activation>() {
        activation.reset();
    }
    let _ = app.emit(EVENT_DELETED, receipt);
}

/// Deletes the account of Veydan Cloud of this identity: `confirm` must be
/// the word DELETE (else `confirm_required`), `lock_secret` the secret of
/// the lock when one is set (else `lock_wrong`). Then the request, and this
/// device unbinds. With `revoke_shares` (plan 4.6) the trusted people are
/// asked to drop their shares first, best effort: how many were asked is
/// in the receipt.
#[tauri::command]
pub async fn cloud_delete_account<R: Runtime>(
    app: AppHandle<R>,
    confirm: String,
    lock_secret: Option<String>,
    revoke_shares: bool,
) -> Result<DeleteReceipt, CloudError> {
    let lock_secret = lock_secret.map(Zeroizing::new);
    if !confirmed(&confirm) {
        return Err(CloudError::new(
            "confirm_required",
            "type the word DELETE to delete the account",
        ));
    }
    let secret = lock_secret.as_ref().map(|s| s.as_str());
    match lock_gate(&app).confirm(secret).await? {
        LockConfirm::NoLock | LockConfirm::Confirmed => {}
        LockConfirm::Wrong => {
            return Err(CloudError::new(
                LOCK_WRONG,
                "the secret of the lock is not right",
            ))
        }
    }
    // Plan 4.6: the shares are revoked before the request of deletion, best
    // effort (without the escrow a share is useless anyway).
    let shares_revoked = if revoke_shares {
        match crate::social::social_of(&app).await {
            Ok(social) => social.revoke_all().await.unwrap_or(0),
            Err(_) => 0,
        }
    } else {
        0
    };
    let deleted =
        delete_account(crate::client(&app).as_ref(), account::boxed(&app).as_ref()).await?;
    let receipt = DeleteReceipt {
        deleted_at: deleted.deleted_at,
        receipt: deleted.receipt,
        shares_revoked,
        from_card: false,
    };
    unbind(&app, &receipt).await;
    Ok(receipt)
}

/// The receipt a card `account.deleted` carries; `None` for another card.
pub fn receipt_of(card: &Card) -> Option<DeleteReceipt> {
    if card.t != CARD_DELETED {
        return None;
    }
    let receipt = card.fields.get("receipt")?.as_str()?.to_string();
    let at = card
        .fields
        .get("at")
        .and_then(|v| v.as_u64())
        .unwrap_or(card.at);
    Some(DeleteReceipt {
        deleted_at: at,
        receipt,
        shares_revoked: 0,
        from_card: true,
    })
}

/// A card of the bot: `account.deleted` unbinds this device and is
/// dismissed; the others are left to whoever handles them.
pub async fn take_card<R: Runtime>(app: &AppHandle<R>, cards: &dyn BotCards, card: &Card) -> bool {
    let Some(receipt) = receipt_of(card) else {
        return false;
    };
    unbind(app, &receipt).await;
    let _ = cards.dismiss(&card.id).await;
    true
}

/// Listens to the cards of the vcloud bot for the whole run of the app.
pub fn listen_cards<R: Runtime>(app: AppHandle<R>) {
    let Some(cards) = veydan_shell::seam::<dyn BotCards, _, _>(&app) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let Ok(mut rx) = cards.cards().await else {
            return;
        };
        while let Some(card) = rx.recv().await {
            if !take_card(&app, cards.as_ref(), &card).await {
                // The cards of recovery (§10) are the alerts of cancel.rs.
                crate::cancel::take_card(&app, cards.as_ref(), &card).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, MemoryBox};
    use serde_json::{json, Map};
    use std::sync::Mutex;
    use veydan_shell::seams::code;

    /// What recovery kept of the account, as a device that issued a key,
    /// reissued it and chose trusted people would have it.
    async fn keep_recovery(boxed: &dyn LockBox) {
        for key in RECOVERY_KEYS {
            boxed.put(key, b"kept").await.expect("put");
        }
    }

    /// The keys of recovery still in the box.
    async fn recovery_left(boxed: &dyn LockBox) -> Vec<&'static str> {
        let mut left = Vec::new();
        for key in RECOVERY_KEYS {
            if boxed.get(key).await.expect("get").is_some() {
                left.push(key);
            }
        }
        left
    }

    #[test]
    fn only_the_word_delete_confirms() {
        assert!(confirmed("DELETE") && confirmed("  DELETE\n"));
        for no in ["", "delete", "Delete", "DELETE!", "УДАЛИТЬ"] {
            assert!(!confirmed(no), "{no}");
        }
    }

    /// The request is the contract's, the receipt comes back, the box is
    /// empty after; the same request again answers the same receipt.
    #[tokio::test]
    async fn the_account_is_deleted_and_the_box_erased() {
        let fake = Fake::start(2).await;
        fake.add_account();
        let api = fake.client();
        let boxed = MemoryBox::default();
        crate::account::refresh(&api, &boxed)
            .await
            .expect("refresh");
        assert!(crate::account::read(&boxed).await.expect("read").is_some());
        keep_recovery(&boxed).await;
        let deleted = delete_account(&api, &boxed).await.expect("deleted");
        assert_eq!(deleted.receipt.len(), 14);
        assert_eq!(crate::account::read(&boxed).await, Ok(None));
        assert_eq!(
            recovery_left(&boxed).await,
            Vec::<&str>::new(),
            "nothing of recovery is kept"
        );
        assert_eq!(
            fake.last_path().as_deref(),
            Some("/cloud/v1/account?confirm=DELETE")
        );
        let again = delete_account(&api, &boxed).await.expect("again");
        assert_eq!(again.receipt, deleted.receipt);
        assert_eq!(
            api.account().await.map_err(|e| e.code),
            Err("not_activated".to_string())
        );
    }

    #[tokio::test]
    async fn the_command_refuses_without_the_word() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        let refused =
            cloud_delete_account(app.handle().clone(), "delete".into(), None, false).await;
        assert_eq!(
            refused.map_err(|e| e.code),
            Err("confirm_required".to_string())
        );
        assert_eq!(fake.requests(), 0);
    }

    /// The command end to end without a lock: the request, the receipt,
    /// the box empty, the activation back to idle, the event to the screens.
    #[tokio::test]
    async fn the_command_deletes_unbinds_and_tells_the_screens() {
        use crate::activate::{ActivationView, Phase};
        use tauri::Listener;
        let fake = Fake::start(2).await;
        fake.add_account();
        let app = fake.app();
        crate::account::refresh(&fake.client(), crate::account::boxed(app.handle()).as_ref())
            .await
            .expect("refresh");
        let state = app.state::<Activation>();
        state.set(ActivationView {
            phase: Phase::Done,
            ..ActivationView::IDLE
        });
        let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let heard = seen.clone();
        app.listen(EVENT_DELETED, move |event| {
            heard
                .lock()
                .expect("seen")
                .push(event.payload().to_string());
        });
        let receipt = cloud_delete_account(app.handle().clone(), " DELETE ".into(), None, false)
            .await
            .expect("deleted");
        assert_eq!(receipt.receipt, crate::fake::RECEIPT);
        assert_eq!((receipt.shares_revoked, receipt.from_card), (0, false));
        assert_eq!(
            fake.log().last().map(String::as_str),
            Some("DELETE /cloud/v1/account?confirm=DELETE")
        );
        assert_eq!(crate::account::load(app.handle()).await, Ok(None));
        assert_eq!(state.view(), ActivationView::IDLE);
        let seen = seen.lock().expect("seen").clone();
        assert_eq!(seen.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&seen[0]).expect("json");
        assert_eq!(payload["receipt"], crate::fake::RECEIPT);
    }

    /// A lock that is set takes its own secret only: a wrong one, or none,
    /// is `lock_wrong` and nothing goes to the server; the right one passes.
    #[tokio::test]
    async fn the_lock_guards_the_deletion() {
        let fake = Fake::start(2).await;
        fake.add_account();
        let app = fake.app_locked(Some("4821"));
        for secret in [Some("0000".to_string()), None] {
            let refused =
                cloud_delete_account(app.handle().clone(), "DELETE".into(), secret, false).await;
            assert_eq!(refused.map_err(|e| e.code), Err(LOCK_WRONG.to_string()));
        }
        assert_eq!(fake.requests(), 0);
        let receipt = cloud_delete_account(
            app.handle().clone(),
            "DELETE".into(),
            Some("4821".into()),
            false,
        )
        .await
        .expect("deleted");
        assert_eq!(receipt.receipt, crate::fake::RECEIPT);
        assert_eq!(fake.requests(), 1);
    }

    /// Without trusted people there is nobody to ask: the receipt says
    /// none were, and the account is deleted all the same.
    #[tokio::test]
    async fn revoking_the_shares_without_trusted_people_asks_nobody() {
        let fake = Fake::start(2).await;
        fake.add_account();
        let app = fake.app();
        let receipt = cloud_delete_account(app.handle().clone(), "DELETE".into(), None, true)
            .await
            .expect("deleted");
        assert_eq!(receipt.shares_revoked, 0);
        assert_eq!(fake.requests(), 1);
    }

    struct Dismissed(Mutex<Vec<String>>);

    impl BotCards for Dismissed {
        fn cards(&self) -> SeamFuture<'_, tokio::sync::mpsc::Receiver<Card>> {
            Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "test")) })
        }
        fn dismiss<'a>(&'a self, id: &'a str) -> SeamFuture<'a, ()> {
            self.0.lock().expect("ids").push(id.to_string());
            Box::pin(async { Ok(()) })
        }
    }

    fn card(t: &str, fields: serde_json::Value) -> Card {
        let fields: Map<String, serde_json::Value> = serde_json::from_value(fields).expect("map");
        Card {
            id: format!("id-{t}"),
            t: t.into(),
            fields,
            at: 1_791_012_345,
        }
    }

    /// The card «account deleted» unbinds this device and is dismissed;
    /// any other card is left alone.
    #[tokio::test]
    async fn the_card_of_the_bot_unbinds_this_device() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        fake.add_account();
        crate::account::refresh(&fake.client(), crate::account::boxed(app.handle()).as_ref())
            .await
            .expect("refresh");
        let boxed = crate::account::boxed(app.handle());
        keep_recovery(boxed.as_ref()).await;
        let cards = Dismissed(Mutex::new(Vec::new()));
        let other = card("plan.expiring", json!({ "plan": "plus", "until": 1 }));
        assert!(!take_card(app.handle(), &cards, &other).await);
        assert!(crate::account::load(app.handle())
            .await
            .expect("load")
            .is_some());
        let deleted = card(
            "account.deleted",
            json!({ "receipt": "7K2M-ABCD-EFGH", "at": 1791012000u64 }),
        );
        assert_eq!(
            receipt_of(&deleted),
            Some(DeleteReceipt {
                deleted_at: 1_791_012_000,
                receipt: "7K2M-ABCD-EFGH".into(),
                shares_revoked: 0,
                from_card: true,
            })
        );
        assert_eq!(
            recovery_left(boxed.as_ref()).await.len(),
            RECOVERY_KEYS.len(),
            "another card keeps it"
        );
        assert!(take_card(app.handle(), &cards, &deleted).await);
        assert_eq!(crate::account::load(app.handle()).await, Ok(None));
        assert_eq!(
            recovery_left(boxed.as_ref()).await,
            Vec::<&str>::new(),
            "nothing of recovery is kept"
        );
        assert_eq!(
            *cards.0.lock().expect("ids"),
            vec!["id-account.deleted".to_string()]
        );
        // A card without its receipt is not the bot's word: nothing happens.
        assert!(receipt_of(&card("account.deleted", json!({}))).is_none());
    }
}

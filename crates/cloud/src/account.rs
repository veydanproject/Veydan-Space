// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The account as this device keeps it (vcloud-plan.md 4.3: «хранится в
//! коробке `cloud` замка»): the plan, its limits and dates, the usage and
//! when the server last saw the identity, in the box `cloud` of the lock,
//! under the vault key, never synced. Asked again once a day, when the
//! page opens and when the user asks; the screens and the limits of the
//! other modules read it without a request.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use veydan_shell::seams::{code, CloudError, LockBox, Signer};

use crate::api::{now, Account, AccountState, AccountUsage, CloudApi, Plans};

/// The box of the lock the module keeps its values in.
pub const BOX: &str = "cloud";

/// The key of the account in the box.
const KEY: &str = "account";

/// The version of [`KeptAccount`].
const KEPT_V: u8 = 1;

/// How old the kept account may get before it is asked again.
pub const REFRESH_EVERY_SECS: u64 = 24 * 3600;

/// How often the background looks whether the kept account is due.
const LOOK_EVERY: Duration = Duration::from_secs(3600);

/// What the box keeps of the account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptAccount {
    pub v: u8,
    pub npub: String,
    pub state: AccountState,
    pub plan: String,
    pub until: Option<u64>,
    pub grace_until: Option<u64>,
    #[serde(default)]
    pub limits: BTreeMap<String, u64>,
    pub usage: Option<AccountUsage>,
    pub last_seen_at: Option<u64>,
    /// When this device last heard the account from the server.
    pub refreshed_at: u64,
}

impl KeptAccount {
    /// The account as the server answered it at `now`.
    pub fn of(account: &Account, now: u64) -> Self {
        Self {
            v: KEPT_V,
            npub: account.npub.clone(),
            state: account.state,
            plan: account.plan.clone(),
            until: account.until,
            grace_until: account.grace_until,
            limits: account.limits.clone(),
            usage: account.usage.clone(),
            last_seen_at: account.last_seen_at,
            refreshed_at: now,
        }
    }

    /// Whether it is a day old, or from a clock that went back.
    pub fn is_due(&self, now: u64) -> bool {
        now >= self.refreshed_at.saturating_add(REFRESH_EVERY_SECS) || now < self.refreshed_at
    }
}

/// The kept account; one that does not read (an older format) is none.
pub async fn read(boxed: &dyn LockBox) -> Result<Option<KeptAccount>, CloudError> {
    let Some(bytes) = boxed.get(KEY).await? else {
        return Ok(None);
    };
    Ok(serde_json::from_slice::<KeptAccount>(&bytes)
        .ok()
        .filter(|kept| kept.v == KEPT_V))
}

/// Keeps `kept` in the box.
pub async fn write(boxed: &dyn LockBox, kept: &KeptAccount) -> Result<(), CloudError> {
    let bytes =
        serde_json::to_vec(kept).map_err(|e| CloudError::new(code::CRYPTO, e.to_string()))?;
    boxed.put(KEY, &bytes).await
}

/// Erases the kept account; works while the lock is closed.
pub async fn erase(boxed: &dyn LockBox) -> Result<(), CloudError> {
    boxed.delete(KEY).await
}

/// Asks the server for the account and keeps what it says: the account,
/// or none when it has none (`not_activated`), which erases the kept one.
pub async fn refresh(
    api: &dyn CloudApi,
    boxed: &dyn LockBox,
) -> Result<Option<Account>, CloudError> {
    match api.account().await {
        Ok(account) => {
            write(boxed, &KeptAccount::of(&account, now()?)).await?;
            Ok(Some(account))
        }
        Err(e) if e.is("not_activated") => {
            erase(boxed).await?;
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// The box `cloud` of the app: the one offered as a seam (a test's), else
/// the lock's.
pub fn boxed<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn LockBox> {
    veydan_shell::seam::<dyn LockBox, _, _>(app)
        .unwrap_or_else(|| veydan_shell::seams::lock_box(app, BOX))
}

/// The account kept on this device.
pub async fn load<R: Runtime>(app: &AppHandle<R>) -> Result<Option<KeptAccount>, CloudError> {
    read(boxed(app).as_ref()).await
}

/// The account kept on this device when it is the one of the identity in
/// use: after another key is imported, the box keeps the previous one's
/// until the next answer of the server, and nothing reads it meanwhile.
pub async fn load_mine<R: Runtime>(app: &AppHandle<R>) -> Result<Option<KeptAccount>, CloudError> {
    let Some(kept) = load(app).await? else {
        return Ok(None);
    };
    let me = match veydan_shell::seam::<dyn Signer, _, _>(app) {
        Some(signer) => signer.pubkey().await.ok(),
        None => None,
    };
    Ok((Some(&kept.npub) == me.as_ref()).then_some(kept))
}

/// Keeps an account the server just answered.
pub async fn keep<R: Runtime>(app: &AppHandle<R>, account: &Account) -> Result<(), CloudError> {
    write(boxed(app).as_ref(), &KeptAccount::of(account, now()?)).await
}

/// Forgets the account on this device.
pub async fn forget<R: Runtime>(app: &AppHandle<R>) -> Result<(), CloudError> {
    erase(boxed(app).as_ref()).await
}

/// Asks again once a day for an account this device keeps; a device that
/// never activated asks nothing, and nothing goes out while the servers
/// are the user's own (the client answers `no_host` without a request).
pub fn start_daily<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(LOOK_EVERY).await;
            let boxed = boxed(&app);
            let due = match (read(boxed.as_ref()).await, now()) {
                (Ok(Some(kept)), Ok(now)) => kept.is_due(now),
                _ => false,
            };
            if due {
                let _ = refresh(crate::client(&app).as_ref(), boxed.as_ref()).await;
            }
        }
    });
}

/// Asks the server for the account now and keeps it; none when the
/// identity has no account.
#[tauri::command]
pub async fn cloud_account_refresh<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<Account>, CloudError> {
    refresh(crate::client(&app).as_ref(), boxed(&app).as_ref()).await
}

/// The answer of `cloud_limits`: the plan and its limits as kept on this
/// device, for the words «на вашем тарифе файлы до 20 МБ» (plan 4.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudLimits {
    pub plan: Option<String>,
    pub state: Option<AccountState>,
    pub until: Option<u64>,
    /// By the keys of the contract (`vault.files_bytes`, `media.max_file_bytes`, …).
    pub limits: BTreeMap<String, u64>,
    /// When the server last said them; none without an account.
    pub refreshed_at: Option<u64>,
}

/// The limits of the account from the box, without a request; empty
/// without an account of the identity in use on this device.
#[tauri::command]
pub async fn cloud_limits<R: Runtime>(app: AppHandle<R>) -> Result<CloudLimits, CloudError> {
    let kept = load_mine(&app).await?;
    Ok(match kept {
        Some(kept) => CloudLimits {
            plan: Some(kept.plan),
            state: Some(kept.state),
            until: kept.until,
            limits: kept.limits,
            refreshed_at: Some(kept.refreshed_at),
        },
        None => CloudLimits {
            plan: None,
            state: None,
            until: None,
            limits: BTreeMap::new(),
            refreshed_at: None,
        },
    })
}

/// The plans on offer, asked of the server now, for the screen that
/// chooses one. Nothing of the answer is kept: the device has no table of
/// plans of its own (plan 4.2). It needs no account and no identity that
/// has one.
#[tauri::command]
pub async fn cloud_plans<R: Runtime>(app: AppHandle<R>) -> Result<Plans, CloudError> {
    crate::client(&app).plans().await
}

/// The plan kept on this device, lent to the modules that store for the
/// user (`veydan_shell::seams::CloudPlan`): the channels of communities
/// keep their files on the free public media without an account.
pub struct KeptPlan<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> veydan_shell::seams::CloudPlan for KeptPlan<R> {
    fn media_retention_days(&self) -> veydan_shell::seams::SeamFuture<'_, Option<u64>> {
        Box::pin(async move {
            let kept = load_mine(&self.0).await?;
            Ok(kept.and_then(|k| match (k.state, k.limits.get(RETENTION_LIMIT).copied()) {
                (AccountState::Suspended, _) => None,
                (_, Some(days)) => Some(days),
                (AccountState::Paid | AccountState::Grace, None) => Some(0),
                (AccountState::Free, None) => None,
            }))
        })
    }
}

/// The delegates of the account kept on this device, lent to the keepers
/// of the channels of communities (`veydan_shell::seams::CloudDelegates`,
/// cloud-api.md §17): the staff of the owner's channels upload into his plan.
pub struct KeptDelegates<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> veydan_shell::seams::CloudDelegates for KeptDelegates<R> {
    fn account(&self) -> veydan_shell::seams::SeamFuture<'_, Option<String>> {
        Box::pin(async move {
            let kept = load_mine(&self.0).await?;
            Ok(kept.filter(|k| k.state != AccountState::Suspended).map(|k| k.npub))
        })
    }

    fn put<'a>(&'a self, delegate: &'a veydan_shell::seams::DelegatePut) -> veydan_shell::seams::SeamFuture<'a, ()> {
        Box::pin(async move { crate::client(&self.0).delegate_put(delegate).await })
    }

    fn revoke<'a>(&'a self, pubkey: &'a str) -> veydan_shell::seams::SeamFuture<'a, ()> {
        Box::pin(async move { crate::client(&self.0).delegate_revoke(pubkey).await })
    }
}

/// The limit of a plan that says how long its media keeps a file, days.
pub const RETENTION_LIMIT: &str = "media.retention_days";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, MemoryBox};

    fn account() -> Account {
        serde_json::from_value(serde_json::json!({
            "npub": "3f".repeat(32), "state": "paid", "plan": "plus", "until": 1793604345u64,
            "grace_until": null,
            "limits": { "vault.files_bytes": 53687091200u64, "media.max_file_bytes": 1073741824u64 },
            "usage": { "as_of": 1791010000u64,
                       "vault": { "files_bytes": 1234567, "text_bytes": 45678 },
                       "media": { "upload_today_bytes": 0, "storage_bytes": 7340032 } },
            "recovery": null, "last_seen_at": 1791012000u64, "server_time": 1791012345u64,
        }))
        .expect("account")
    }

    /// The account goes into the box and comes back the same; a value of
    /// another version or shape is no account; erasing leaves none.
    #[tokio::test]
    async fn the_account_goes_round_the_box() {
        let boxed = MemoryBox::default();
        assert_eq!(read(&boxed).await, Ok(None));
        let kept = KeptAccount::of(&account(), 1_791_012_400);
        write(&boxed, &kept).await.expect("write");
        assert_eq!(read(&boxed).await, Ok(Some(kept.clone())));
        assert_eq!(kept.plan, "plus");
        assert_eq!(kept.limits["media.max_file_bytes"], 1_073_741_824);
        assert_eq!(kept.last_seen_at, Some(1_791_012_000));
        boxed.put(KEY, b"{\"v\":9}").await.expect("put");
        assert_eq!(read(&boxed).await, Ok(None));
        write(&boxed, &kept).await.expect("write");
        erase(&boxed).await.expect("erase");
        assert_eq!(read(&boxed).await, Ok(None));
        // A closed box says so; nothing is guessed.
        boxed.close();
        assert_eq!(
            read(&boxed).await.map_err(|e| e.code),
            Err(code::LOCKED.to_string())
        );
    }

    #[test]
    fn a_kept_account_is_due_after_a_day_or_when_the_clock_went_back() {
        let kept = KeptAccount::of(&account(), 1_000_000);
        assert!(!kept.is_due(1_000_000));
        assert!(!kept.is_due(1_000_000 + REFRESH_EVERY_SECS - 1));
        assert!(kept.is_due(1_000_000 + REFRESH_EVERY_SECS));
        assert!(kept.is_due(999_999));
    }

    /// A refresh keeps what the server says, and an identity the server
    /// does not know any more leaves nothing kept.
    #[tokio::test]
    async fn a_refresh_keeps_the_account_or_forgets_it() {
        let fake = Fake::start(2).await;
        let api = fake.client();
        let boxed = MemoryBox::default();
        write(&boxed, &KeptAccount::of(&account(), 1))
            .await
            .expect("write");
        assert_eq!(refresh(&api, &boxed).await, Ok(None));
        assert_eq!(read(&boxed).await, Ok(None));
        fake.add_account();
        let answered = refresh(&api, &boxed)
            .await
            .expect("refresh")
            .expect("account");
        let kept = read(&boxed).await.expect("read").expect("kept");
        assert_eq!((kept.plan.as_str(), kept.npub), ("free", answered.npub));
        assert!(kept.refreshed_at > 0);
    }

    #[tokio::test]
    async fn the_limits_are_read_from_the_box_without_a_request() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        let empty = cloud_limits(app.handle().clone()).await.expect("limits");
        assert!(empty.plan.is_none() && empty.limits.is_empty());
        let mine = Account {
            npub: fake.npub(),
            ..account()
        };
        keep(app.handle(), &mine).await.expect("keep");
        let asked = fake.requests();
        let limits = cloud_limits(app.handle().clone()).await.expect("limits");
        assert_eq!(limits.plan.as_deref(), Some("plus"));
        assert_eq!(limits.limits["vault.files_bytes"], 53_687_091_200);
        assert_eq!(fake.requests(), asked);
        forget(app.handle()).await.expect("forget");
        assert_eq!(load(app.handle()).await, Ok(None));
    }

    /// The plans come from the server at every call, before there is an
    /// account, and the box keeps none of them; a product whose messenger
    /// offers no way to the project gets the refusal the other commands give.
    #[tokio::test]
    async fn the_plans_are_asked_of_the_server_and_not_kept() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        let catalog = cloud_plans(app.handle().clone()).await.expect("plans");
        assert_eq!(fake.log(), ["GET /cloud/v1/plans"]);
        let ids: Vec<&str> = catalog.plans.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["free", "plus"]);
        assert!(catalog.plans[0].free && !catalog.plans[1].free);
        assert_eq!(catalog.plans[1].limits["vault.files_bytes"], 53_687_091_200);
        assert_eq!(
            catalog.plans[1].prices[1],
            crate::api::PlanPrice {
                currency: "USD".into(),
                amount_minor: 3990,
                period_days: 365
            }
        );
        // What the screen receives: the fields of the contract, by their names.
        let shown = serde_json::to_value(&catalog).expect("json");
        assert_eq!(
            shown["plans"][1]["prices"][0],
            serde_json::json!({ "currency": "USD", "amount_minor": 399, "period_days": 30 })
        );
        assert!(shown["server_time"].is_u64());
        assert_eq!(load(app.handle()).await, Ok(None));
        cloud_plans(app.handle().clone()).await.expect("plans");
        assert_eq!(fake.requests(), 2);

        let bare = tauri::test::mock_app();
        let refused = cloud_plans(bare.handle().clone())
            .await
            .expect_err("no host");
        assert_eq!(refused.code, code::NO_HOST);
    }

    /// The account of another identity, kept from before a key was
    /// imported, gives no limits: the empty answer until the server speaks.
    #[tokio::test]
    async fn the_limits_of_another_identity_are_not_read() {
        let fake = Fake::start(2).await;
        let app = fake.app();
        let other = Account {
            npub: "ab".repeat(32),
            ..account()
        };
        keep(app.handle(), &other).await.expect("keep");
        let limits = cloud_limits(app.handle().clone()).await.expect("limits");
        assert!(limits.plan.is_none() && limits.limits.is_empty() && limits.refreshed_at.is_none());
        assert_eq!(load_mine(app.handle()).await, Ok(None));
        assert!(load(app.handle()).await.expect("load").is_some());
    }
}

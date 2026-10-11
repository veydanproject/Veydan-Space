// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The owner's live device during a request of recovery
//! (internal/cloud-recovery.md §8.5 step 4, §10): the card
//! `recovery.requested` of the vcloud bot becomes an alert on the screens
//! with «Это не я — отменить» → `POST /cloud/v1/recovery/cancel` signed by
//! the identity, a freeze of seven days, and the note `recovery.cancel` to
//! my other devices so that they take the alert down. The cards
//! `recovery.escrow_read` («у кого-то есть ваши 12 слов») and
//! `recovery.restored` are alerts too, closed by hand.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use veydan_shell::seams::{code, BotCards, Card, CloudError, Note, PeerNotes};

use crate::api::{Cancelled, CloudApi};

/// The event to the screens with the [`Alert`]s of this device.
pub const EVENT_ALERTS: &str = "cloud://recovery-alerts";
/// The cards of §10 this side turns into alerts.
pub const CARD_REQUESTED: &str = "recovery.requested";
pub const CARD_ESCROW_READ: &str = "recovery.escrow_read";
pub const CARD_RESTORED: &str = "recovery.restored";
/// The note to my own devices after a cancel (messenger-wire.md §3).
pub const T_CANCEL: &str = "recovery.cancel";

/// An alert on the screens, from a card of the bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alert {
    /// The id of the card: what dismisses it.
    pub id: String,
    /// `recovery.requested`, `recovery.escrow_read` or `recovery.restored`.
    pub t: String,
    /// Of `recovery.requested`: what «Это не я — отменить» cancels.
    pub request_id: Option<String>,
    pub created_at: Option<u64>,
    pub expires_at: Option<u64>,
    /// When the card was written, unix seconds.
    pub at: u64,
}

/// The alerts of this device, for the run of the app.
#[derive(Default)]
pub struct Alerts(Mutex<Vec<Alert>>);

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Alerts {
    pub fn list(&self) -> Vec<Alert> {
        guard(&self.0).clone()
    }

    /// Adds `alert` unless its card is known; whether it was added.
    pub fn add(&self, alert: Alert) -> bool {
        let mut alerts = guard(&self.0);
        if alerts.iter().any(|a| a.id == alert.id) {
            return false;
        }
        alerts.push(alert);
        true
    }

    /// Removes the alerts of card `id` or of `request_id`; their ids.
    pub fn remove(&self, id: Option<&str>, request_id: Option<&str>) -> Vec<String> {
        let mut alerts = guard(&self.0);
        let (gone, kept): (Vec<Alert>, Vec<Alert>) = alerts.drain(..).partition(|a| {
            id.is_some_and(|i| a.id == i) || request_id.is_some_and(|r| a.request_id.as_deref() == Some(r))
        });
        *alerts = kept;
        gone.into_iter().map(|a| a.id).collect()
    }
}

/// The alert a card carries; `None` for a card of another kind.
pub fn alert_of(card: &Card) -> Option<Alert> {
    if ![CARD_REQUESTED, CARD_ESCROW_READ, CARD_RESTORED].contains(&card.t.as_str()) {
        return None;
    }
    let at = card.fields.get("at").and_then(Value::as_u64).unwrap_or(card.at);
    let request_id = card
        .fields
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    if card.t == CARD_REQUESTED && request_id.is_none() {
        return None;
    }
    Some(Alert {
        id: card.id.clone(),
        t: card.t.clone(),
        request_id,
        created_at: card.fields.get("created_at").and_then(Value::as_u64),
        expires_at: card.fields.get("expires_at").and_then(Value::as_u64),
        at,
    })
}

/// The alerts of the app, made at the first use.
fn alerts_of<R: Runtime>(app: &AppHandle<R>) -> Arc<Alerts> {
    if let Some(alerts) = app.try_state::<Arc<Alerts>>() {
        return alerts.inner().clone();
    }
    let alerts = Arc::new(Alerts::default());
    app.manage(alerts.clone());
    app.try_state::<Arc<Alerts>>()
        .map(|kept| kept.inner().clone())
        .unwrap_or(alerts)
}

fn tell<R: Runtime>(app: &AppHandle<R>, alerts: &Alerts) {
    let _ = app.emit(EVENT_ALERTS, alerts.list());
}

/// A card of the bot: one of §10 about recovery becomes an alert and the
/// screens are told; it is dismissed when the alert is closed. Whether the
/// card was one of ours.
pub async fn take_card<R: Runtime>(app: &AppHandle<R>, _cards: &dyn BotCards, card: &Card) -> bool {
    let Some(alert) = alert_of(card) else {
        return false;
    };
    let alerts = alerts_of(app);
    if alerts.add(alert) {
        tell(app, &alerts);
    }
    true
}

/// `recovery.cancel` from my other device: the alert of that request goes.
pub async fn on_cancel_note<R: Runtime>(
    app: &AppHandle<R>,
    fields: &Map<String, Value>,
) -> Result<bool, CloudError> {
    let Some(request_id) = fields.get("request_id").and_then(Value::as_str) else {
        return Ok(false);
    };
    let alerts = alerts_of(app);
    let gone = alerts.remove(None, Some(request_id));
    if gone.is_empty() {
        return Ok(false);
    }
    if let Some(cards) = veydan_shell::seam::<dyn BotCards, _, _>(app) {
        for id in &gone {
            let _ = cards.dismiss(id).await;
        }
    }
    tell(app, &alerts);
    Ok(true)
}

fn stopped(what: &str) -> CloudError {
    CloudError::new(
        code::MESSENGER_STOPPED,
        format!("the messenger gives no {what} in this product"),
    )
}

/// «Это не я — отменить»: the request cancelled on the server, my other
/// devices told, the alert gone here.
pub async fn cancel_request<R: Runtime>(
    app: &AppHandle<R>,
    api: &dyn CloudApi,
    request_id: &str,
) -> Result<Cancelled, CloudError> {
    let cancelled = api.recovery_cancel(request_id).await?;
    if let Some(notes) = veydan_shell::seam::<dyn PeerNotes, _, _>(app) {
        let mut fields = Map::new();
        fields.insert("request_id".into(), Value::String(request_id.to_string()));
        if let Err(e) = notes.send_own(Note::new(T_CANCEL, fields)).await {
            eprintln!("cloud: my other devices were not told of the cancel: {e}");
        }
    }
    let alerts = alerts_of(app);
    let gone = alerts.remove(None, Some(request_id));
    if let Some(cards) = veydan_shell::seam::<dyn BotCards, _, _>(app) {
        for id in &gone {
            let _ = cards.dismiss(id).await;
        }
    }
    tell(app, &alerts);
    Ok(cancelled)
}

/// The alerts of this device.
#[tauri::command]
pub async fn cloud_recovery_alerts<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Alert>, CloudError> {
    Ok(alerts_of(&app).list())
}

/// «Это не я — отменить» of the request `request_id`.
#[tauri::command]
pub async fn cloud_recovery_cancel<R: Runtime>(
    request_id: String,
    app: AppHandle<R>,
) -> Result<Cancelled, CloudError> {
    let _ = veydan_shell::seam::<dyn PeerNotes, _, _>(&app).ok_or_else(|| stopped("notes"))?;
    let api = crate::client(&app);
    cancel_request(&app, api.as_ref(), &request_id).await
}

/// An alert closed by hand: its card is dismissed.
#[tauri::command]
pub async fn cloud_recovery_alert_dismiss<R: Runtime>(
    id: String,
    app: AppHandle<R>,
) -> Result<(), CloudError> {
    let alerts = alerts_of(&app);
    let gone = alerts.remove(Some(&id), None);
    if let Some(cards) = veydan_shell::seam::<dyn BotCards, _, _>(&app) {
        for id in &gone {
            let _ = cards.dismiss(id).await;
        }
    }
    tell(&app, &alerts);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card(t: &str, fields: Value) -> Card {
        Card {
            id: format!("card-{t}"),
            t: t.into(),
            fields: fields.as_object().cloned().unwrap_or_default(),
            at: 1_791_012_345,
        }
    }

    #[test]
    fn the_cards_of_recovery_become_alerts_and_others_do_not() {
        let requested = alert_of(&card(
            CARD_REQUESTED,
            json!({ "request_id": "9c", "created_at": 1, "expires_at": 1801 }),
        ))
        .expect("alert");
        assert_eq!(requested.request_id.as_deref(), Some("9c"));
        assert_eq!(requested.expires_at, Some(1801));
        assert!(alert_of(&card(CARD_REQUESTED, json!({}))).is_none(), "nothing to cancel");
        let read = alert_of(&card(CARD_ESCROW_READ, json!({ "at": 7 }))).expect("alert");
        assert_eq!((read.at, read.request_id.clone()), (7, None));
        assert!(alert_of(&card("plan.expiring", json!({}))).is_none());
        assert!(alert_of(&card("account.deleted", json!({ "receipt": "x" }))).is_none());

        let alerts = Alerts::default();
        assert!(alerts.add(requested.clone()));
        assert!(!alerts.add(requested.clone()), "a card once");
        assert!(alerts.add(read.clone()));
        assert_eq!(alerts.remove(None, Some("9c")), vec![requested.id.clone()]);
        assert_eq!(alerts.remove(Some(&read.id), None), vec![read.id]);
        assert!(alerts.list().is_empty());
    }

    #[tokio::test]
    async fn the_commands_work_without_the_messenger_where_they_can() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        assert_eq!(cloud_recovery_alerts(handle.clone()).await.expect("alerts"), Vec::new());
        let e = cloud_recovery_cancel("9c".into(), handle.clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        assert!(cloud_recovery_alert_dismiss("none".into(), handle.clone()).await.is_ok());
        let mut fields = Map::new();
        fields.insert("request_id".into(), Value::String("9c".into()));
        assert_eq!(on_cancel_note(&handle, &fields).await, Ok(false));
    }
}

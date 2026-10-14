// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Social Recovery, the trusted person's side (internal/cloud-recovery.md
//! §8.3, §8.5 steps 5-7, messenger-wire.md §3):
//!
//! - a share that comes (`recovery.share`, from an approved contact: the
//!   messenger lets no other through) is checked by its form, its owner
//!   must be the sealer, and it lies in the box `recovery_shares` of the
//!   lock under `<owner>:<generation>`; older generations of the owner go,
//!   an older share than the one kept is dropped; an ack goes back; nothing
//!   is shown — the person does not learn they are trusted;
//! - a ping is answered with a pong when the share is here, with silence
//!   otherwise; a revoke deletes and says nothing;
//! - «Помочь другу восстановить доступ»: by the owner's key or the link
//!   `veydan://recover/<npub>?c=<code>`, the open request is read from the
//!   server without a signature of the identity, the code is computed here
//!   from `temp_pub` and compared with the link's, and after the mandatory
//!   checkbox of a talk by voice or in person the share goes to T as
//!   `recovery.answer`, sealed to T, expiring with the request.

use std::sync::{Arc, Mutex, MutexGuard};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, Runtime};
use veydan_recovery::{pow, Share};
use veydan_shell::seams::{
    code, CloudError, Contacts, LockBox, Note, PeerNotes, Signer,
};
use zeroize::Zeroizing;

use crate::api::{now, CloudApi, RecoveryRequest};
use crate::social::{unhex32, Clock, T_ACK, T_PING, T_PONG, T_REVOKE, T_SHARE, WEEK_SECS};

/// The box of the lock the shares of others lie in (§8.3).
pub const BOX_SHARES: &str = "recovery_shares";
/// The key of the index of the box: JSON `{ "<owner>": generation }`.
const KEY_INDEX: &str = "index";
/// The type of the note with the share, to T.
pub const T_ANSWER: &str = "recovery.answer";
/// The prefix of the link to a trusted person (messenger-links `recover`).
pub const RECOVER_LINK_PREFIX: &str = "veydan://recover/";

/// The words of the refusals of this side.
pub mod words {
    /// What was typed is neither a key nor a link to a request.
    pub const BAD_NPUB: &str = "bad_npub";
    /// «У вас нет ключа для этого аккаунта».
    pub const NO_SHARE: &str = "no_share";
    /// «Открытого запроса нет».
    pub const NO_REQUEST: &str = "no_request";
    /// The code of the link is not the code of the request: «Запрос подменён».
    pub const CODE_MISMATCH: &str = "code_mismatch";
    /// The checkbox of the talk by voice or in person is not ticked.
    pub const CONFIRM_REQUIRED: &str = "confirm_required";
    /// The request is no longer open.
    pub const REQUEST_CLOSED: &str = "request_closed";
    /// No request was looked up on this device.
    pub const NOT_LOOKED_UP: &str = "not_looked_up";
}

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The key of a share in the box.
fn share_key(owner: &str, generation: u32) -> String {
    format!("{owner}:{generation}")
}

/// The index of the box: the generation kept for each owner.
async fn load_index(boxed: &dyn LockBox) -> Result<Map<String, Value>, CloudError> {
    let Some(bytes) = boxed.get(KEY_INDEX).await? else {
        return Ok(Map::new());
    };
    Ok(serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default())
}

async fn save_index(boxed: &dyn LockBox, index: &Map<String, Value>) -> Result<(), CloudError> {
    let json = serde_json::to_vec(index).map_err(crypto)?;
    boxed.put(KEY_INDEX, &json).await
}

/// The generation of the share of `owner` kept here, if any.
pub async fn generation_of(boxed: &dyn LockBox, owner: &str) -> Result<Option<u32>, CloudError> {
    Ok(load_index(boxed)
        .await?
        .get(owner)
        .and_then(Value::as_u64)
        .and_then(|g| u32::try_from(g).ok()))
}

/// The share of `owner` kept here, with its generation.
pub async fn share_of(boxed: &dyn LockBox, owner: &str) -> Result<Option<(u32, Share)>, CloudError> {
    let Some(generation) = generation_of(boxed, owner).await? else {
        return Ok(None);
    };
    let Some(bytes) = boxed.get(&share_key(owner, generation)).await? else {
        return Ok(None);
    };
    let share = Share::from_bytes(&bytes).map_err(crypto)?;
    Ok(Some((generation, share)))
}

/// Keeps `share` of `owner`: the index and the bytes; older generations of
/// the owner go.
async fn keep(boxed: &dyn LockBox, owner: &str, share: &Share) -> Result<(), CloudError> {
    let mut index = load_index(boxed).await?;
    if let Some(old) = index.get(owner).and_then(Value::as_u64) {
        if old != u64::from(share.generation) {
            boxed
                .delete(&share_key(owner, u32::try_from(old).unwrap_or(u32::MAX)))
                .await?;
        }
    }
    boxed
        .put(&share_key(owner, share.generation), share.to_bytes().as_slice())
        .await?;
    index.insert(owner.to_string(), Value::from(share.generation));
    save_index(boxed, &index).await
}

/// Forgets the share of `owner` of `generation`, when that is the one kept.
async fn forget(boxed: &dyn LockBox, owner: &str, generation: u32) -> Result<bool, CloudError> {
    let mut index = load_index(boxed).await?;
    if index.get(owner).and_then(Value::as_u64) != Some(u64::from(generation)) {
        return Ok(false);
    }
    boxed.delete(&share_key(owner, generation)).await?;
    index.remove(owner);
    save_index(boxed, &index).await?;
    Ok(true)
}

/// The receiving side over the seams of the messenger.
#[derive(Clone)]
pub struct TrusteeSide {
    notes: Arc<dyn PeerNotes>,
    boxed: Arc<dyn LockBox>,
    clock: Clock,
}

impl TrusteeSide {
    pub fn new(notes: Arc<dyn PeerNotes>, boxed: Arc<dyn LockBox>) -> Self {
        Self {
            notes,
            boxed,
            clock: Arc::new(|| now().unwrap_or(0)),
        }
    }

    /// The same with another clock: the tests.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// One note from an approved contact (`from` the sealer): a share, a
    /// ping or a revoke; whether it changed or answered anything.
    pub async fn on_note(&self, note: &Note) -> Result<bool, CloudError> {
        let Some(from) = note.from.as_deref() else {
            return Ok(false);
        };
        if unhex32(from).is_none() {
            return Ok(false);
        }
        match note.t.as_str() {
            T_SHARE => self.on_share(from, &note.fields).await,
            T_PING => self.on_ping(from, &note.fields).await,
            T_REVOKE => self.on_revoke(from, &note.fields).await,
            _ => Ok(false),
        }
    }

    /// `recovery.share`: kept when well-formed, from its owner and not older
    /// than the one here; then the ack.
    pub async fn on_share(&self, from: &str, fields: &Map<String, Value>) -> Result<bool, CloudError> {
        let Some(bytes) = fields
            .get("share")
            .and_then(Value::as_str)
            .and_then(|s| B64.decode(s).ok())
        else {
            return Ok(false);
        };
        let bytes = Zeroizing::new(bytes);
        let Ok(share) = Share::from_bytes(&bytes) else {
            return Ok(false);
        };
        if hex(&share.npub) != from {
            return Ok(false);
        }
        if let Some(kept) = generation_of(self.boxed.as_ref(), from).await? {
            if share.generation < kept {
                return Ok(false);
            }
        }
        keep(self.boxed.as_ref(), from, &share).await?;
        let mut ack = Map::new();
        ack.insert("npub".into(), Value::String(from.to_string()));
        ack.insert("generation".into(), Value::from(share.generation));
        ack.insert("index".into(), Value::from(share.index));
        let mut note = Note::new(T_ACK, ack);
        note.expires_at = Some((self.clock)() + WEEK_SECS);
        self.notes.send(note, from).await?;
        Ok(true)
    }

    /// `recovery.share.ping`: a pong with `have` when the share of that
    /// generation is here; silence otherwise.
    pub async fn on_ping(&self, from: &str, fields: &Map<String, Value>) -> Result<bool, CloudError> {
        let Some(generation) = fields.get("generation").and_then(Value::as_u64) else {
            return Ok(false);
        };
        let have = generation_of(self.boxed.as_ref(), from).await? == u32::try_from(generation).ok();
        if !have {
            return Ok(false);
        }
        let mut pong = Map::new();
        pong.insert("npub".into(), Value::String(from.to_string()));
        pong.insert("generation".into(), Value::from(generation));
        pong.insert("have".into(), Value::Bool(true));
        let mut note = Note::new(T_PONG, pong);
        note.expires_at = Some((self.clock)() + WEEK_SECS);
        self.notes.send(note, from).await?;
        Ok(true)
    }

    /// `recovery.share.revoke` from the owner (`npub` must be the sealer):
    /// the share of that generation goes, silently.
    pub async fn on_revoke(&self, from: &str, fields: &Map<String, Value>) -> Result<bool, CloudError> {
        if fields.get("npub").and_then(Value::as_str) != Some(from) {
            return Ok(false);
        }
        let Some(generation) = fields
            .get("generation")
            .and_then(Value::as_u64)
            .and_then(|g| u32::try_from(g).ok())
        else {
            return Ok(false);
        };
        forget(self.boxed.as_ref(), from, generation).await
    }
}

// ── «Помочь другу восстановить доступ» ───────────────────────────────────

/// What the screen «Подтвердить восстановление» shows (§8.5 step 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpView {
    /// The owner, 64 hex.
    pub npub: String,
    /// My name for the owner, when they are a contact.
    pub name: Option<String>,
    pub request_id: String,
    /// The code computed here from `temp_pub`, `AB7K-2QZD-9MXP`.
    pub code: String,
    /// `open`, `cancelled`, `expired` or `closed`.
    pub state: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub closed_at: Option<u64>,
    /// The generation of the share held here.
    pub generation: u32,
    /// Whether the code of the link is the code of the request; `None`
    /// without a link.
    pub code_matches: Option<bool>,
    /// The request is open and the code is not contradicted: the checkbox
    /// and the button are shown.
    pub can_confirm: bool,
}

/// The share went to T.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpSent {
    pub request_id: String,
    pub expires_at: u64,
}

/// The owner's key and, from a link, the code to compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpTarget {
    pub npub: String,
    pub code: Option<String>,
}

/// bech32 (BIP-173) `npub1…` as 32 bytes, the checksum verified.
pub fn npub_bech32_to_hex(text: &str) -> Option<String> {
    const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let lower = text.to_ascii_lowercase();
    let data = lower.strip_prefix("npub1")?;
    if data.len() != 58 {
        return None;
    }
    let values: Option<Vec<u8>> = data
        .bytes()
        .map(|b| CHARSET.iter().position(|c| *c == b).map(|p| p as u8))
        .collect();
    let values = values?;
    // The checksum over the expanded hrp and the data.
    let mut polymod_in: Vec<u8> = vec![3, 3, 3, 3, 0, 14, 16, 21, 2];
    polymod_in.extend_from_slice(&values);
    let mut chk: u32 = 1;
    for v in polymod_in {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ u32::from(v);
        for (i, g) in [0x3b6a_57b2u32, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62b3]
            .iter()
            .enumerate()
        {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    if chk != 1 {
        return None;
    }
    // 52 values of 5 bits → 32 bytes.
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::with_capacity(32);
    for v in &values[..52] {
        acc = (acc << 5) | u32::from(*v);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    (out.len() == 32).then(|| hex(&out))
}

/// What was typed or opened: a key (64 hex or `npub1…`) or a link
/// `veydan://recover/<npub>?c=<code>`.
pub fn parse_target(input: &str) -> Result<HelpTarget, CloudError> {
    let bad = || CloudError::new(words::BAD_NPUB, "type the key of the account or open its link");
    let input = input.trim();
    let n = RECOVER_LINK_PREFIX.len();
    let (key, code) = match (input.get(..n), input.get(n..)) {
        (Some(head), Some(rest)) if head.eq_ignore_ascii_case(RECOVER_LINK_PREFIX) => {
            let (id, query) = rest.split_once('?').unwrap_or((rest, ""));
            let code = query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(k, _)| *k == "c")
                .map(|(_, v)| v.replace("%20", " "));
            (id.to_string(), code)
        }
        _ => (input.strip_prefix("nostr:").unwrap_or(input).to_string(), None),
    };
    let key = key.trim().to_ascii_lowercase();
    let npub = if unhex32(&key).is_some() {
        key
    } else {
        npub_bech32_to_hex(&key).ok_or_else(bad)?
    };
    Ok(HelpTarget { npub, code })
}

/// The request looked up and the share to answer with, kept between the
/// look-up and the confirmation.
struct HelpSession {
    view: HelpView,
    temp_pub: String,
    share: Zeroizing<Vec<u8>>,
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// «Помочь другу» over the seams.
pub struct Helper {
    api: Arc<dyn CloudApi>,
    notes: Arc<dyn PeerNotes>,
    contacts: Option<Arc<dyn Contacts>>,
    boxed: Arc<dyn LockBox>,
    session: Mutex<Option<HelpSession>>,
}

/// What the help under way keeps between the commands: the view shown,
/// the id of the request and the share, sealed.
type HelpKept = (HelpView, String, Zeroizing<Vec<u8>>);

/// The state of the help under way, kept by the app between the commands.
#[derive(Default)]
pub struct HelpState(Mutex<Option<HelpKept>>);

impl Helper {
    pub fn new(
        api: Arc<dyn CloudApi>,
        notes: Arc<dyn PeerNotes>,
        contacts: Option<Arc<dyn Contacts>>,
        boxed: Arc<dyn LockBox>,
    ) -> Self {
        Self {
            api,
            notes,
            contacts,
            boxed,
            session: Mutex::new(None),
        }
    }

    /// Step 5: the share here, the request on the server, the code.
    pub async fn lookup(&self, input: &str) -> Result<HelpView, CloudError> {
        let target = parse_target(input)?;
        let Some((generation, share)) = share_of(self.boxed.as_ref(), &target.npub).await? else {
            return Err(CloudError::new(
                words::NO_SHARE,
                "no key of this account is kept on this device",
            ));
        };
        let request: RecoveryRequest = self
            .api
            .recovery_request_get(&target.npub)
            .await?
            .ok_or_else(|| CloudError::new(words::NO_REQUEST, "no open request"))?;
        let temp_pub = request
            .temp_pub
            .clone()
            .ok_or_else(|| crypto("the request names no temporary key"))?;
        let t_bytes = unhex32(&temp_pub).ok_or_else(|| crypto("the temporary key is not 64 hex"))?;
        let code = pow::check_code(&t_bytes);
        let code_matches = target
            .code
            .as_deref()
            .map(|typed| pow::check_code_matches(&t_bytes, typed));
        let state = request.state.clone().unwrap_or_else(|| "open".into());
        let name = match &self.contacts {
            Some(contacts) => contacts
                .approved()
                .await
                .unwrap_or_default()
                .into_iter()
                .find(|c| c.pubkey == target.npub)
                .map(|c| c.name),
            None => None,
        };
        let view = HelpView {
            npub: target.npub.clone(),
            name,
            request_id: request.request_id.clone(),
            code,
            can_confirm: state == "open" && code_matches != Some(false),
            state,
            created_at: request.created_at,
            expires_at: request.expires_at,
            closed_at: request.closed_at,
            generation,
            code_matches,
        };
        *guard(&self.session) = Some(HelpSession {
            view: view.clone(),
            temp_pub,
            share: Zeroizing::new(share.to_bytes().to_vec()),
        });
        Ok(view)
    }

    /// Step 7: «Подтвердить» with the checkbox ticked (`spoke`): the share
    /// to T, bound to the request and T, expiring with the request.
    pub async fn confirm(&self, spoke: bool) -> Result<HelpSent, CloudError> {
        let (view, temp_pub, share) = {
            let session = guard(&self.session);
            let s = session
                .as_ref()
                .ok_or_else(|| CloudError::new(words::NOT_LOOKED_UP, "look the request up first"))?;
            (s.view.clone(), s.temp_pub.clone(), Zeroizing::new(s.share.to_vec()))
        };
        if !spoke {
            return Err(CloudError::new(
                words::CONFIRM_REQUIRED,
                "confirm that you talked by voice or in person",
            ));
        }
        if view.code_matches == Some(false) {
            return Err(CloudError::new(
                words::CODE_MISMATCH,
                "the code of the link is not the code of the request",
            ));
        }
        if view.state != "open" {
            return Err(CloudError::new(words::REQUEST_CLOSED, format!("the request is {}", view.state)));
        }
        // The look-up may be minutes old: the owner may have said «это не я»
        // since. Read the request again; the share goes only to the very
        // request that was shown, still open.
        let again = self.api.recovery_request_get(&view.npub).await?;
        let alive = again.is_some_and(|r| {
            r.request_id == view.request_id
                && r.temp_pub.as_deref() == Some(temp_pub.as_str())
                && r.state.as_deref().unwrap_or("open") == "open"
                && now().is_ok_and(|n| r.expires_at > n)
        });
        if !alive {
            *guard(&self.session) = None;
            return Err(CloudError::new(
                words::REQUEST_CLOSED,
                "the request is gone, changed or closed since it was looked up",
            ));
        }
        let mut fields = Map::new();
        fields.insert("request_id".into(), Value::String(view.request_id.clone()));
        fields.insert("t_pub".into(), Value::String(temp_pub.clone()));
        fields.insert("generation".into(), Value::from(view.generation));
        fields.insert("share".into(), Value::String(B64.encode(share.as_slice())));
        let mut note = Note::new(T_ANSWER, fields);
        note.expires_at = Some(view.expires_at);
        self.notes.send_to_key(note, &temp_pub).await?;
        *guard(&self.session) = None;
        Ok(HelpSent {
            request_id: view.request_id,
            expires_at: view.expires_at,
        })
    }

    /// «Это не он — закрыть»: nothing goes, the look-up is forgotten.
    pub fn close(&self) {
        *guard(&self.session) = None;
    }
}

// ── The app ──────────────────────────────────────────────────────────────

fn stopped(what: &str) -> CloudError {
    CloudError::new(
        code::MESSENGER_STOPPED,
        format!("the messenger gives no {what} in this product"),
    )
}

/// The box `recovery_shares` of the lock.
pub fn shares_box<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn LockBox> {
    veydan_shell::seams::lock_box(app, BOX_SHARES)
}

/// The receiving side of the app.
pub async fn side_of<R: Runtime>(app: &AppHandle<R>) -> Result<TrusteeSide, CloudError> {
    let notes = veydan_shell::seam::<dyn PeerNotes, _, _>(app).ok_or_else(|| stopped("notes"))?;
    Ok(TrusteeSide::new(notes, shares_box(app)))
}

/// The help of the app, over its seams; its session lives in [`HelpState`].
fn helper_of<R: Runtime>(app: &AppHandle<R>) -> Result<Helper, CloudError> {
    let notes = veydan_shell::seam::<dyn PeerNotes, _, _>(app).ok_or_else(|| stopped("notes"))?;
    let _ = veydan_shell::seam::<dyn Signer, _, _>(app).ok_or_else(|| stopped("signer"))?;
    let contacts = veydan_shell::seam::<dyn Contacts, _, _>(app);
    let helper = Helper::new(crate::client(app), notes, contacts, shares_box(app));
    if let Some(state) = app.try_state::<HelpState>() {
        if let Some((view, temp_pub, share)) = guard(&state.0).as_ref() {
            *guard(&helper.session) = Some(HelpSession {
                view: view.clone(),
                temp_pub: temp_pub.clone(),
                share: Zeroizing::new(share.to_vec()),
            });
        }
    }
    Ok(helper)
}

/// Keeps the session of `helper` in the app between the commands.
fn keep_session<R: Runtime>(app: &AppHandle<R>, helper: &Helper) {
    if let Some(state) = app.try_state::<HelpState>() {
        *guard(&state.0) = guard(&helper.session)
            .as_ref()
            .map(|s| (s.view.clone(), s.temp_pub.clone(), Zeroizing::new(s.share.to_vec())));
    }
}

/// «Помочь другу» → the key or the link: what the screen shows.
#[tauri::command]
pub async fn cloud_help_lookup<R: Runtime>(
    input: String,
    app: AppHandle<R>,
) -> Result<HelpView, CloudError> {
    let helper = helper_of(&app)?;
    let view = helper.lookup(&input).await?;
    keep_session(&app, &helper);
    Ok(view)
}

/// «Подтвердить» with the checkbox: the share goes to T.
#[tauri::command]
pub async fn cloud_help_confirm<R: Runtime>(
    spoke: bool,
    app: AppHandle<R>,
) -> Result<HelpSent, CloudError> {
    let helper = helper_of(&app)?;
    let sent = helper.confirm(spoke).await?;
    keep_session(&app, &helper);
    Ok(sent)
}

/// «Это не он — закрыть».
#[tauri::command]
pub async fn cloud_help_close<R: Runtime>(app: AppHandle<R>) -> Result<(), CloudError> {
    if let Some(state) = app.try_state::<HelpState>() {
        *guard(&state.0) = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e";

    #[test]
    fn a_target_is_a_key_or_a_link() {
        let hex_target = parse_target(&format!(" {} ", HEX.to_ascii_uppercase())).expect("hex");
        assert_eq!(hex_target, HelpTarget { npub: HEX.into(), code: None });
        let link = parse_target(&format!("veydan://recover/{HEX}?c=AB7K-2QZD-9MXP")).expect("link");
        assert_eq!(link.npub, HEX);
        assert_eq!(link.code.as_deref(), Some("AB7K-2QZD-9MXP"));
        let bare = parse_target(&format!("VEYDAN://recover/{HEX}")).expect("link without a code");
        assert_eq!(bare.code, None);
        assert_eq!(
            parse_target("npub1nope").err().map(|e| e.code).as_deref(),
            Some(words::BAD_NPUB)
        );
        assert_eq!(parse_target("").err().map(|e| e.code).as_deref(), Some(words::BAD_NPUB));
    }

    /// The npub of the nostr documentation (NIP-19): its hex is known.
    #[test]
    fn an_npub_is_read_as_bech32() {
        let npub = "npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg";
        let hex = npub_bech32_to_hex(npub).expect("decodes");
        assert_eq!(hex, "7e7e9c42a91bfef19fa929e5fda1b72e0ebc1a4c1141673e2794234d86addf4e");
        assert_eq!(parse_target(&format!("nostr:{npub}")).expect("target").npub, hex);
        // A changed character breaks the checksum.
        let mut broken = npub.to_string();
        broken.replace_range(10..11, if &npub[10..11] == "q" { "p" } else { "q" });
        assert_eq!(npub_bech32_to_hex(&broken), None);
        assert_eq!(npub_bech32_to_hex("nsec1qqqq"), None);
    }

    #[tokio::test]
    async fn without_the_seams_the_commands_say_the_messenger_is_stopped() {
        let app = tauri::test::mock_app();
        let e = cloud_help_lookup(HEX.into(), app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_help_confirm(true, app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        assert!(cloud_help_close(app.handle().clone()).await.is_ok());
    }
}

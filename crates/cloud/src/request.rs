// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Social Recovery, the one who asks (internal/cloud-recovery.md §8.5,
//! steps 1-3 and 8-10): a clean install, «Восстановить аккаунт» → «Через
//! доверенных людей». The key of the account typed, a temporary key T, the
//! proof of work of `d = 8` («готовим запрос…»), `POST
//! /cloud/v1/recovery/request` signed by T; then the check code computed
//! from T (`AB7K-2QZD-9MXP`), the link `veydan://recover/<npub>?c=<code>`
//! to share, and the listening on T for `recovery.answer`. Every answer is
//! checked against the request and T, counted by the person who sealed it
//! (shown as «подтвердил npub1…»), and at `k` shares of one generation the
//! key is put together (crates/recovery `combine`), the commit and the
//! macs checked, a forged share named by its sender. Then the order of §7
//! through the restore of C3 ([`Restorer`]), `POST …/recovery/close`
//! signed by T, and the 12 words shown once more: the key did not change.
//! Thirty minutes without `k` answers: the request expired, T is wiped.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::mpsc;
use veydan_recovery::{pow, shamir, Share};
use veydan_shell::seams::{code, CloudError, Note, PeerNotes, SeamFuture, TempKey};
use zeroize::Zeroizing;

use crate::api::{now, CloudApi, Purpose};
use crate::nip98;
use crate::restore::{NewSecret, Restored, Vault};
use crate::social::{unhex32, Clock};
use crate::trustee::{npub_bech32_to_hex, T_ANSWER};

/// The difficulty of the proof of a request (§9).
pub const REQUEST_DIFFICULTY: u8 = pow::RECOVERY_REQUEST_DIFFICULTY;
/// The prefix of the link to a trusted person.
pub const LINK_PREFIX: &str = "veydan://recover/";

/// The words of the refusals of this side, beside the server's
/// (`not_configured`, `request_open`, `frozen`, `rate_limited`).
pub mod words {
    /// What was typed is not a key of an account.
    pub const BAD_NPUB: &str = "bad_npub";
    /// No request is under way on this device.
    pub const REQUEST_STATE: &str = "request_state";
    /// The key is not put together yet: fewer than `k` answers.
    pub const NOT_COLLECTED: &str = "not_collected";
    /// The thirty minutes passed without `k` answers.
    pub const REQUEST_EXPIRED: &str = "request_expired";
}

/// Where the request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPhase {
    /// No request on this device.
    Idle,
    /// The proof of work, the request to the server: «готовим запрос…».
    Preparing,
    /// The request is open; T listens: «Подтвердили: 1 из 3».
    Waiting,
    /// `k` shares came and the key is put together: the words wait for
    /// the new secret of the lock.
    Collected,
    /// The account is restored by the order of §7.
    Restored,
    /// Thirty minutes passed with fewer than `k` answers.
    Expired,
    /// The server refused the request.
    Failed,
}

/// One person who answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Confirmation {
    /// The sealer of the answer, 64 hex.
    pub npub: String,
    /// `index` of the share they sent.
    pub index: u8,
    /// The mac of their share did not match the key: «доля повреждена».
    pub forged: bool,
}

/// What «Запрос отправлен» shows (§11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestView {
    pub phase: RequestPhase,
    /// The account asked for, 64 hex.
    pub npub: Option<String>,
    pub request_id: Option<String>,
    /// `AB7K-2QZD-9MXP`, to say by voice.
    pub code: Option<String>,
    /// `veydan://recover/<npub>?c=<code>`, to share.
    pub link: Option<String>,
    pub k: u8,
    pub n: u8,
    pub created_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub confirmed: Vec<Confirmation>,
    /// Attempts of the proof so far, while preparing.
    pub attempts: u64,
    /// Why the request failed.
    pub error: Option<CloudError>,
}

/// What the restore after the shares ends with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recovered {
    pub restored: Restored,
    /// The 12 words, to write down: the key did not change.
    pub words: String,
    /// Whether the server took `close`: without it the request expires by
    /// itself and no card `recovery.restored` goes to the owner.
    pub closed: bool,
}

/// The restore of §7 by the words put together: the module's own
/// `cloud_restore_with_words` in the app, a fake in the tests.
pub trait Restorer: Send + Sync {
    fn restore<'a>(
        &'a self,
        words: &'a str,
        secret: NewSecret,
        vault: Vault,
    ) -> SeamFuture<'a, Restored>;
}

/// A share that came, with who sealed it.
struct Answer {
    from: String,
    share: Share,
}

/// A request under way.
struct Session {
    round: u64,
    npub: String,
    npub_bytes: [u8; 32],
    t: TempKey,
    request_id: String,
    k: u8,
    n: u8,
    created_at: u64,
    expires_at: u64,
    code: String,
    answers: Vec<Answer>,
    confirmed: Vec<Confirmation>,
    phase: RequestPhase,
    /// The key put together, as its words; wiped with the session.
    words: Option<Zeroizing<String>>,
    error: Option<CloudError>,
}

/// The request of this device, kept by the module between the commands.
#[derive(Default)]
pub struct Sessions {
    current: Mutex<Option<Session>>,
    rounds: Mutex<u64>,
    /// While the proof is worked: the attempts so far.
    preparing: Mutex<Option<(String, u64)>>,
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

/// The key of an account as typed: 64 hex or `npub1…`, as hex.
pub fn parse_npub(input: &str) -> Result<String, CloudError> {
    let text = input.trim();
    let text = text.strip_prefix("nostr:").unwrap_or(text).to_ascii_lowercase();
    if unhex32(&text).is_some() {
        return Ok(text);
    }
    npub_bech32_to_hex(&text)
        .ok_or_else(|| CloudError::new(words::BAD_NPUB, "type the key of the account: npub1… or 64 hex"))
}

/// The link to a trusted person.
pub fn link_of(npub: &str, code: &str) -> String {
    format!("{LINK_PREFIX}{npub}?c={code}")
}

/// The one who asks, over the seams.
#[derive(Clone)]
pub struct Requester {
    api: Arc<dyn CloudApi>,
    notes: Arc<dyn PeerNotes>,
    sessions: Arc<Sessions>,
    clock: Clock,
    /// The least difficulty worked; the server may ask more (3.4).
    difficulty: u8,
}

impl Requester {
    pub fn new(api: Arc<dyn CloudApi>, notes: Arc<dyn PeerNotes>, sessions: Arc<Sessions>) -> Self {
        Self {
            api,
            notes,
            sessions,
            clock: Arc::new(|| now().unwrap_or(0)),
            difficulty: REQUEST_DIFFICULTY,
        }
    }

    /// The same with another clock and a difficulty of its own: the tests.
    pub fn with(mut self, clock: Clock, difficulty: u8) -> Self {
        self.clock = clock;
        self.difficulty = difficulty;
        self
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    /// Steps 1-3: T, the proof, the request, the code; T listens until the
    /// request expires. An earlier request of this device is forgotten.
    pub async fn start(&self, npub_input: &str) -> Result<RequestView, CloudError> {
        let npub = parse_npub(npub_input)?;
        let npub_bytes = unhex32(&npub).ok_or_else(|| crypto("the key is not 64 hex"))?;
        self.cancel();
        *guard(&self.sessions.preparing) = Some((npub.clone(), 0));
        let prepared = self.prepare(&npub).await;
        *guard(&self.sessions.preparing) = None;
        let (t, made) = match prepared {
            Ok(done) => done,
            Err(e) => {
                let round = self.next_round();
                *guard(&self.sessions.current) = Some(Session {
                    round,
                    npub,
                    npub_bytes,
                    t: TempKey::generate()?,
                    request_id: String::new(),
                    k: 0,
                    n: 0,
                    created_at: 0,
                    expires_at: 0,
                    code: String::new(),
                    answers: Vec::new(),
                    confirmed: Vec::new(),
                    phase: RequestPhase::Failed,
                    words: None,
                    error: Some(e.clone()),
                });
                return Err(e);
            }
        };
        let t_pub = nip98::temp_pub(&t)?;
        let t_bytes = unhex32(&t_pub).ok_or_else(|| crypto("T is not 64 hex"))?;
        let code = pow::check_code(&t_bytes);
        let listener = TempKey::new(Zeroizing::new(*t.secret_bytes()));
        let heard = self.notes.listen_as(listener).await?;
        let round = self.next_round();
        let expires_at = made.expires_at;
        *guard(&self.sessions.current) = Some(Session {
            round,
            npub,
            npub_bytes,
            t,
            request_id: made.request_id.clone(),
            k: made.k.unwrap_or(0),
            n: made.n.unwrap_or(0),
            created_at: made.created_at,
            expires_at,
            code,
            answers: Vec::new(),
            confirmed: Vec::new(),
            phase: RequestPhase::Waiting,
            words: None,
            error: None,
        });
        tokio::spawn(self.clone().hear(round, heard, made.request_id, t_pub, expires_at));
        Ok(self.poll())
    }

    fn next_round(&self) -> u64 {
        let mut rounds = guard(&self.sessions.rounds);
        *rounds += 1;
        *rounds
    }

    /// The proof and the request: T and what the server answered.
    async fn prepare(&self, npub: &str) -> Result<(TempKey, crate::api::RecoveryRequest), CloudError> {
        let t = TempKey::generate()?;
        let challenge = self.api.challenge(Purpose::RecoveryRequest).await?;
        let sessions = self.sessions.clone();
        let pow = crate::activate::solve(
            &challenge,
            self.difficulty,
            move |attempts| {
                if let Some((_, n)) = guard(&sessions.preparing).as_mut() {
                    *n = attempts;
                }
            },
            None,
        )
        .await?;
        let made = self.api.recovery_request(npub, &t, pow).await?;
        Ok((t, made))
    }

    /// Step 8: the answers to T until the key is together or the time is
    /// over. An answer of another request or T, a share of another owner or
    /// not of the form, is dropped; a second answer of one person is not
    /// counted twice.
    async fn hear(
        self,
        round: u64,
        mut heard: mpsc::Receiver<Note>,
        request_id: String,
        t_pub: String,
        expires_at: u64,
    ) {
        let left = Duration::from_secs(expires_at.saturating_sub(self.now()));
        let deadline = tokio::time::Instant::now() + left;
        loop {
            let note = match tokio::time::timeout_at(deadline, heard.recv()).await {
                Ok(Some(note)) => note,
                Ok(None) => return,
                Err(_) => {
                    self.expire(round);
                    return;
                }
            };
            let Some(answer) = answer_of(&note, &request_id, &t_pub) else {
                continue;
            };
            if self.take(round, answer) {
                return;
            }
        }
    }

    /// One answer into the session of `round`; whether the key is together.
    fn take(&self, round: u64, answer: Answer) -> bool {
        let mut current = guard(&self.sessions.current);
        let Some(s) = current.as_mut().filter(|s| s.round == round) else {
            return true;
        };
        if s.phase != RequestPhase::Waiting {
            return true;
        }
        if answer.share.npub != s.npub_bytes || s.answers.iter().any(|a| a.from == answer.from) {
            return false;
        }
        s.confirmed.push(Confirmation {
            npub: answer.from.clone(),
            index: answer.share.index,
            forged: false,
        });
        s.answers.push(answer);
        // The generation most people answered with; a revoked one, or one
        // person's stale share, does not stand in the way.
        let mut generations: Vec<u32> = s.answers.iter().map(|a| a.share.generation).collect();
        generations.sort_unstable();
        generations.dedup();
        for generation in generations {
            let group: Vec<&Answer> = s
                .answers
                .iter()
                .filter(|a| a.share.generation == generation)
                .collect();
            let mut indexes: Vec<u8> = group.iter().map(|a| a.share.index).collect();
            indexes.sort_unstable();
            indexes.dedup();
            if indexes.len() < usize::from(s.k.max(shamir::MIN_THRESHOLD)) {
                continue;
            }
            let shares: Vec<Share> = group.iter().map(|a| a.share.clone()).collect();
            let Ok(recovered) = shamir::combine(&shares) else {
                continue;
            };
            if recovered.npub != s.npub_bytes {
                continue;
            }
            for position in &recovered.forged {
                if let Some(bad) = group.get(*position) {
                    if let Some(c) = s.confirmed.iter_mut().find(|c| c.npub == bad.from) {
                        c.forged = true;
                    }
                }
            }
            s.words = Some(recovered.rk.words());
            s.phase = RequestPhase::Collected;
            return true;
        }
        false
    }

    /// The time is over with fewer than `k` answers: T is wiped.
    fn expire(&self, round: u64) {
        let mut current = guard(&self.sessions.current);
        if let Some(s) = current.as_mut().filter(|s| s.round == round) {
            if s.phase == RequestPhase::Waiting {
                s.phase = RequestPhase::Expired;
                s.t = TempKey::new(Zeroizing::new([0u8; 32]));
            }
        }
    }

    /// Where the request stands.
    pub fn poll(&self) -> RequestView {
        if let Some((npub, attempts)) = guard(&self.sessions.preparing).clone() {
            return RequestView {
                phase: RequestPhase::Preparing,
                npub: Some(npub),
                request_id: None,
                code: None,
                link: None,
                k: 0,
                n: 0,
                created_at: None,
                expires_at: None,
                confirmed: Vec::new(),
                attempts,
                error: None,
            };
        }
        let mut current = guard(&self.sessions.current);
        let Some(s) = current.as_mut() else {
            return RequestView {
                phase: RequestPhase::Idle,
                npub: None,
                request_id: None,
                code: None,
                link: None,
                k: 0,
                n: 0,
                created_at: None,
                expires_at: None,
                confirmed: Vec::new(),
                attempts: 0,
                error: None,
            };
        };
        if s.phase == RequestPhase::Waiting && self.now() >= s.expires_at {
            s.phase = RequestPhase::Expired;
        }
        let open = !matches!(s.phase, RequestPhase::Failed);
        RequestView {
            phase: s.phase,
            npub: Some(s.npub.clone()),
            request_id: open.then(|| s.request_id.clone()),
            code: open.then(|| s.code.clone()),
            link: open.then(|| link_of(&s.npub, &s.code)),
            k: s.k,
            n: s.n,
            created_at: open.then_some(s.created_at),
            expires_at: open.then_some(s.expires_at),
            confirmed: s.confirmed.clone(),
            attempts: 0,
            error: s.error.clone(),
        }
    }

    /// Step 9: the key is together — the restore of §7 with the new secret
    /// of the lock (without the vault when its storage is out of reach and
    /// the person said so), then `close` signed by T; the words once more.
    pub async fn finish(
        &self,
        secret: NewSecret,
        vault: Vault,
        restorer: &dyn Restorer,
    ) -> Result<Recovered, CloudError> {
        let (words, request_id) = {
            let current = guard(&self.sessions.current);
            let s = current
                .as_ref()
                .ok_or_else(|| CloudError::new(words::REQUEST_STATE, "no request is under way on this device"))?;
            match s.phase {
                RequestPhase::Collected | RequestPhase::Restored => {}
                RequestPhase::Expired => {
                    return Err(CloudError::new(words::REQUEST_EXPIRED, "the request expired"))
                }
                _ => return Err(CloudError::new(words::NOT_COLLECTED, "fewer than k people answered")),
            }
            let words = s
                .words
                .as_ref()
                .map(|w| Zeroizing::new(w.to_string()))
                .ok_or_else(|| CloudError::new(words::NOT_COLLECTED, "the key is not together"))?;
            (words, s.request_id.clone())
        };
        let restored = restorer.restore(&words, secret, vault).await?;
        let closed = {
            let t = {
                let mut current = guard(&self.sessions.current);
                current.as_mut().map(|s| {
                    s.phase = RequestPhase::Restored;
                    TempKey::new(Zeroizing::new(*s.t.secret_bytes()))
                })
            };
            match t {
                Some(t) => self.api.recovery_close(&request_id, &t).await.is_ok(),
                None => false,
            }
        };
        // T goes; the words stay for the screen that shows them once.
        if let Some(s) = guard(&self.sessions.current).as_mut() {
            s.t = TempKey::new(Zeroizing::new([0u8; 32]));
        }
        Ok(Recovered {
            restored,
            words: words.to_string(),
            closed,
        })
    }

    /// «Отменить»: the request is forgotten here and expires on the server
    /// by itself (only the owner's own key may cancel it); T is wiped.
    pub fn cancel(&self) {
        *guard(&self.sessions.current) = None;
        *guard(&self.sessions.preparing) = None;
    }
}

/// The share of a `recovery.answer` to this request and T, with its sealer.
fn answer_of(note: &Note, request_id: &str, t_pub: &str) -> Option<Answer> {
    if note.t != T_ANSWER {
        return None;
    }
    let from = note.from.as_deref()?;
    unhex32(from)?;
    if note.fields.get("request_id")?.as_str()? != request_id
        || note.fields.get("t_pub")?.as_str()? != t_pub
    {
        return None;
    }
    let generation = note.fields.get("generation")?.as_u64()?;
    let bytes = Zeroizing::new(B64.decode(note.fields.get("share")?.as_str()?).ok()?);
    let share = Share::from_bytes(&bytes).ok()?;
    if u64::from(share.generation) != generation {
        return None;
    }
    Some(Answer {
        from: from.to_string(),
        share,
    })
}

// ── The app ──────────────────────────────────────────────────────────────

fn stopped(what: &str) -> CloudError {
    CloudError::new(
        code::MESSENGER_STOPPED,
        format!("the messenger gives no {what} in this product"),
    )
}

/// The requester of the app.
fn requester_of<R: Runtime>(app: &AppHandle<R>) -> Result<Requester, CloudError> {
    let notes = veydan_shell::seam::<dyn PeerNotes, _, _>(app).ok_or_else(|| stopped("notes"))?;
    let sessions = app
        .try_state::<Arc<Sessions>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| CloudError::new(words::REQUEST_STATE, "the module cloud is not set up"))?;
    Ok(Requester::new(crate::client(app), notes, sessions))
}

/// [`Restorer`] of the running app: the restore of C3.
struct AppRestorer(AppHandle);

impl Restorer for AppRestorer {
    fn restore<'a>(
        &'a self,
        words: &'a str,
        secret: NewSecret,
        vault: Vault,
    ) -> SeamFuture<'a, Restored> {
        Box::pin(async move {
            crate::recovery::cloud_restore_with_words(
                words.to_string(),
                secret.password,
                secret.kind,
                secret.hint,
                Some(vault == Vault::Skip),
                self.0.clone(),
            )
            .await
        })
    }
}

/// «Через доверенных людей» → the key of the account: the proof, the
/// request, the code and the link; T listens.
#[tauri::command]
pub async fn cloud_recover_start<R: Runtime>(
    npub: String,
    app: AppHandle<R>,
) -> Result<RequestView, CloudError> {
    requester_of(&app)?.start(&npub).await
}

/// Where the request stands: the count of the people who confirmed.
#[tauri::command]
pub async fn cloud_recover_poll<R: Runtime>(app: AppHandle<R>) -> Result<RequestView, CloudError> {
    Ok(requester_of(&app)?.poll())
}

/// The key is together: the restore of §7 with the new secret of the
/// lock, `close`, the words once more. `without_vault` is the person's
/// answer to `vault_unreachable`.
#[tauri::command]
pub async fn cloud_recover_finish(
    password: String,
    kind: Option<String>,
    hint: Option<String>,
    without_vault: Option<bool>,
    app: AppHandle,
) -> Result<Recovered, CloudError> {
    let requester = requester_of(&app)?;
    requester
        .finish(
            NewSecret {
                password,
                kind,
                hint,
            },
            Vault::of(without_vault),
            &AppRestorer(app.clone()),
        )
        .await
}

/// «Отменить»: the request is forgotten here.
#[tauri::command]
pub async fn cloud_recover_cancel<R: Runtime>(app: AppHandle<R>) -> Result<(), CloudError> {
    requester_of(&app)?.cancel();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    #[test]
    fn a_key_is_read_as_hex_or_npub_and_the_link_is_written() {
        let hex = "ab".repeat(32);
        assert_eq!(parse_npub(&format!(" {} ", hex.to_ascii_uppercase())).expect("hex"), hex);
        assert_eq!(
            parse_npub("npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg").expect("npub"),
            "7e7e9c42a91bfef19fa929e5fda1b72e0ebc1a4c1141673e2794234d86addf4e"
        );
        assert_eq!(parse_npub("nobody").err().map(|e| e.code).as_deref(), Some(words::BAD_NPUB));
        assert_eq!(link_of(&hex, "AB7K-2QZD-9MXP"), format!("veydan://recover/{hex}?c=AB7K-2QZD-9MXP"));
    }

    #[test]
    fn an_answer_is_taken_only_for_this_request_and_t() {
        let rk = veydan_recovery::RecoveryKey::generate().expect("rk");
        let owner = [0x3f; 32];
        let shares = shamir::split(&rk, &owner, 2, 2, 3).expect("split");
        let t_pub = "ab".repeat(32);
        let note = |request_id: &str, t: &str, generation: u64, share: &Share| {
            let mut fields = Map::new();
            fields.insert("request_id".into(), Value::String(request_id.into()));
            fields.insert("t_pub".into(), Value::String(t.into()));
            fields.insert("generation".into(), Value::from(generation));
            fields.insert("share".into(), Value::String(B64.encode(share.to_bytes().as_slice())));
            let mut n = Note::new(T_ANSWER, fields);
            n.from = Some("cd".repeat(32));
            n
        };
        assert!(answer_of(&note("9c", &t_pub, 2, &shares[0]), "9c", &t_pub).is_some());
        assert!(answer_of(&note("9d", &t_pub, 2, &shares[0]), "9c", &t_pub).is_none());
        assert!(answer_of(&note("9c", &"ef".repeat(32), 2, &shares[0]), "9c", &t_pub).is_none());
        assert!(answer_of(&note("9c", &t_pub, 3, &shares[0]), "9c", &t_pub).is_none(), "the generation lies");
        let mut other = note("9c", &t_pub, 2, &shares[0]);
        other.t = "device.link".into();
        assert!(answer_of(&other, "9c", &t_pub).is_none());
        let mut nobody = note("9c", &t_pub, 2, &shares[0]);
        nobody.from = None;
        assert!(answer_of(&nobody, "9c", &t_pub).is_none());
    }

    #[tokio::test]
    async fn without_the_seams_the_commands_say_so() {
        let app = tauri::test::mock_app();
        let e = cloud_recover_start("ab".repeat(32), app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_recover_poll(app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_recover_cancel(app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let json = serde_json::to_value(RequestPhase::Collected).expect("json");
        assert_eq!(json, "collected");
    }
}

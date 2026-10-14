// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Social Recovery end to end on loopback: the owner and three trusted
//! people on a relay in memory (the notes, their sealers and the rule of
//! the approved contacts are modelled here; the wraps themselves are the
//! messenger's, tested in messenger-runtime), a clean device that asks,
//! and the fake of `/cloud/v1` (fake.rs) as the meeting point. The shares,
//! the key put together and the check code are the real crates/recovery.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::mpsc;
use veydan_recovery::RecoveryKey;
use veydan_shell::seams::{
    code, ApprovedContact, CloudError, Contacts, Note, PeerNotes, SeamFuture, TempKey, PEER_TYPES,
    TO_KEY_TYPES,
};

use crate::api::CloudApi;
use crate::escrow;
use crate::fake::{Fake, MemoryBox};
use crate::reissue::ShareDelivery;
use crate::request::{Recovered, RequestPhase, Requester, Restorer, Sessions};
use crate::restore::{LockOutcome, NewSecret, Restored, Vault};
use crate::social::{self, Health, Social};
use crate::trustee::{self, Helper, TrusteeSide};

const NOW: u64 = 1_791_000_000;

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|p| p.into_inner())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The relay and the address books of everyone.
#[derive(Default)]
struct Relay {
    /// The notes to an identity, by its devices.
    inboxes: HashMap<String, Vec<mpsc::Sender<Note>>>,
    /// The notes to a key T.
    keys: HashMap<String, Vec<mpsc::Sender<Note>>>,
    /// Who chose to talk with whom, both ways.
    approved: HashSet<(String, String)>,
    names: HashMap<String, String>,
    /// Every note that went, `(t, from, to)`.
    log: Vec<(String, String, String)>,
}

#[derive(Clone)]
struct World {
    relay: Arc<Mutex<Relay>>,
    clock: Arc<AtomicU64>,
}

impl World {
    fn new() -> Self {
        Self {
            relay: Arc::default(),
            clock: Arc::new(AtomicU64::new(NOW)),
        }
    }
    fn later(&self, secs: u64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
    }
    fn clock(&self) -> social::Clock {
        let c = self.clock.clone();
        Arc::new(move || c.load(Ordering::SeqCst))
    }
    fn befriend(&self, a: &str, b: &str) {
        let mut r = guard(&self.relay);
        r.approved.insert((a.into(), b.into()));
        r.approved.insert((b.into(), a.into()));
    }
    fn sent(&self, t: &str) -> usize {
        guard(&self.relay).log.iter().filter(|(kind, _, _)| kind == t).count()
    }
}

/// One device: its identity (none on a clean install), its boxes.
struct Device {
    world: World,
    me: Option<String>,
    boxed: Arc<MemoryBox>,
    shares: Arc<MemoryBox>,
}

impl Device {
    fn new(world: &World, nsec: Option<[u8; 32]>, name: &str) -> Arc<Self> {
        let me = nsec.and_then(|n| veydan_recovery::rk::npub_of(&n)).map(|p| hex(&p));
        if let Some(me) = &me {
            guard(&world.relay).names.insert(me.clone(), name.to_string());
        }
        Arc::new(Self {
            world: world.clone(),
            me,
            boxed: Arc::new(MemoryBox::default()),
            shares: Arc::new(MemoryBox::default()),
        })
    }
    fn me(&self) -> String {
        self.me.clone().unwrap_or_default()
    }
    fn deliver(&self, mut note: Note, to_identity: &str) {
        note.from = Some(self.me());
        let mut r = guard(&self.world.relay);
        r.log.push((note.t.clone(), self.me(), to_identity.to_string()));
        if let Some(inboxes) = r.inboxes.get_mut(to_identity) {
            inboxes.retain(|tx| tx.try_send(note.clone()).is_ok());
        }
    }
}

impl PeerNotes for Device {
    fn send<'a>(&'a self, note: Note, peer: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            if !PEER_TYPES.contains(&note.t.as_str()) {
                return Err(CloudError::new(code::BAD_REQUEST, "not a note to a contact"));
            }
            let allowed = guard(&self.world.relay)
                .approved
                .contains(&(self.me(), peer.to_string()));
            if !allowed {
                return Err(CloudError::new(code::BAD_REQUEST, "not an approved contact"));
            }
            self.deliver(note, peer);
            Ok(())
        })
    }
    fn send_own(&self, note: Note) -> SeamFuture<'_, ()> {
        Box::pin(async move {
            let me = self.me();
            self.deliver(note, &me);
            Ok(())
        })
    }
    fn send_to_key<'a>(&'a self, mut note: Note, pubkey: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            if !TO_KEY_TYPES.contains(&note.t.as_str()) {
                return Err(CloudError::new(code::BAD_REQUEST, "not a note to a key"));
            }
            note.from = Some(self.me());
            let mut r = guard(&self.world.relay);
            r.log.push((note.t.clone(), self.me(), pubkey.to_string()));
            if let Some(listeners) = r.keys.get_mut(pubkey) {
                listeners.retain(|tx| tx.try_send(note.clone()).is_ok());
            }
            Ok(())
        })
    }
    fn listen_as(&self, t: TempKey) -> SeamFuture<'_, mpsc::Receiver<Note>> {
        Box::pin(async move {
            let t_pub = crate::nip98::temp_pub(&t)?;
            let (tx, rx) = mpsc::channel(32);
            guard(&self.world.relay).keys.entry(t_pub).or_default().push(tx);
            Ok(rx)
        })
    }
    fn incoming(&self) -> SeamFuture<'_, mpsc::Receiver<Note>> {
        Box::pin(async move {
            let (tx, rx) = mpsc::channel(64);
            guard(&self.world.relay)
                .inboxes
                .entry(self.me())
                .or_default()
                .push(tx);
            Ok(rx)
        })
    }
}

impl Contacts for Device {
    fn approved(&self) -> SeamFuture<'_, Vec<ApprovedContact>> {
        Box::pin(async move {
            let r = guard(&self.world.relay);
            let me = self.me();
            let mut out: Vec<ApprovedContact> = r
                .approved
                .iter()
                .filter(|(a, _)| *a == me)
                .map(|(_, b)| ApprovedContact {
                    pubkey: b.clone(),
                    name: r.names.get(b).cloned().unwrap_or_default(),
                })
                .collect();
            out.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
            Ok(out)
        })
    }
}

/// A trusted person's device: keeps the shares, answers the notes.
async fn serve_trustee(device: Arc<Device>) -> TrusteeSide {
    let side = TrusteeSide::new(device.clone(), device.shares.clone()).with_clock(device.world.clock());
    let mut rx = device.incoming().await.expect("incoming");
    let served = side.clone();
    tokio::spawn(async move {
        while let Some(note) = rx.recv().await {
            let _ = served.on_note(&note).await;
        }
    });
    side
}

/// The owner's device: hears the acks and the pongs.
async fn serve_owner(device: Arc<Device>, api: Arc<dyn CloudApi>) -> Social {
    let social = Social::new(device.clone(), device.boxed.clone(), api, device.me())
        .with(device.world.clock(), Duration::from_secs(5));
    let mut rx = device.incoming().await.expect("incoming");
    let heard = social.clone();
    tokio::spawn(async move {
        while let Some(note) = rx.recv().await {
            let from = note.from.clone().unwrap_or_default();
            match note.t.as_str() {
                social::T_ACK => {
                    let _ = heard.on_ack(&from, &note.fields).await;
                }
                social::T_PONG => {
                    let _ = heard.on_pong(&from, &note.fields).await;
                }
                _ => {}
            }
        }
    });
    social
}

async fn until<F: Fn() -> bool>(what: &str, f: F) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{what}: did not happen in time");
}

async fn until_async<F, Fut>(what: &str, f: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{what}: did not happen in time");
}

/// The restore of §7, as far as this test goes: the words are recorded.
struct Recorder(Mutex<Vec<String>>);

impl Restorer for Recorder {
    fn restore<'a>(
        &'a self,
        words: &'a str,
        secret: NewSecret,
        vault: Vault,
    ) -> SeamFuture<'a, Restored> {
        Box::pin(async move {
            assert_eq!(secret.password, "4821");
            guard(&self.0).push(words.to_string());
            Ok(Restored {
                npub: "restored".into(),
                lock: LockOutcome::Kept,
                vault_joined: vault == Vault::Join,
                k: 2,
                n: 3,
                generation: 1,
                escrow_refreshed: None,
            })
        })
    }
}

struct Stage {
    world: World,
    fake: Fake,
    api: Arc<dyn CloudApi>,
    owner: Arc<Device>,
    social: Social,
    trustees: Vec<Arc<Device>>,
    rk: RecoveryKey,
    people: Vec<ApprovedContact>,
}

/// An owner with the fake's identity, a key issued, three trusted people
/// who approved each other, the shares delivered and acked.
async fn stage() -> Stage {
    let world = World::new();
    let fake = Fake::start(2).await;
    fake.add_account();
    fake.require_config();
    let api: Arc<dyn CloudApi> = Arc::new(fake.client());
    let owner = Device::new(&world, Some([0x3f; 32]), "Owner");
    assert_eq!(owner.me(), fake.npub());
    let trustees: Vec<Arc<Device>> = [([0x11; 32], "Ann"), ([0x22; 32], "Ben"), ([0x33; 32], "Cid")]
        .into_iter()
        .map(|(nsec, name)| Device::new(&world, Some(nsec), name))
        .collect();
    for t in &trustees {
        world.befriend(&owner.me(), &t.me());
        serve_trustee(t.clone()).await;
    }
    let rk = RecoveryKey::generate().expect("rk");
    escrow::save_rk(owner.boxed.as_ref(), &rk, 1).await.expect("rk kept");
    let social = serve_owner(owner.clone(), api.clone()).await;
    let people = owner.approved().await.expect("approved");
    assert_eq!(people.len(), 3);
    let status = social.setup(2, people.clone(), &people).await.expect("setup");
    assert!(status.configured && status.k == 2 && status.n == 3 && status.generation == 1);
    assert_eq!(fake.config().map(|c| c["k"].as_u64()), Some(Some(2)));
    let s = social.clone();
    until_async("every trusted person acked", || async { s.status().await.map(|s| s.acked).unwrap_or(0) == 3 }).await;
    Stage {
        world,
        fake,
        api,
        owner,
        social,
        trustees,
        rk,
        people,
    }
}

fn helper(stage: &Stage, device: &Arc<Device>) -> Helper {
    Helper::new(stage.api.clone(), device.clone(), Some(device.clone()), device.shares.clone())
}

fn requester_of(stage: &Stage, clean: &Arc<Device>) -> Requester {
    Requester::new(stage.api.clone(), clean.clone(), Arc::new(Sessions::default()))
        .with(stage.world.clock(), 2)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_shares_are_delivered_silently_and_the_set_is_known_by_person() {
    let stage = stage().await;
    for t in &stage.trustees {
        let (generation, share) = trustee::share_of(t.shares.as_ref(), &stage.owner.me())
            .await
            .expect("box")
            .expect("a share");
        assert_eq!((generation, share.k, share.n), (1, 2, 3));
        assert_eq!(hex(&share.npub), stage.owner.me());
    }
    assert_eq!(stage.world.sent(social::T_SHARE), 3);
    assert_eq!(stage.world.sent(social::T_ACK), 3);
    let status = stage.social.status().await.expect("status");
    assert!(status.people.iter().all(|p| p.health == Health::Delivered));
    let mut names: Vec<&str> = status.people.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Ann", "Ben", "Cid"]);
    assert_eq!(status.people.iter().map(|p| p.index).collect::<Vec<_>>(), [1, 2, 3]);
    // The escrow carries the numbers of this set.
    let params = social::recovery_params(stage.owner.boxed.as_ref(), 1).await.expect("params");
    assert_eq!((params.k, params.n), (2, 3));

    // A share from a stranger, a share of another owner, an older share: dropped.
    let stranger = Device::new(&stage.world, Some([0x44; 32]), "Dan");
    let ann = &stage.trustees[0];
    let shares = veydan_recovery::shamir::split(&stage.rk, &social::unhex32(&stranger.me()).expect("hex"), 1, 2, 3).expect("split");
    let note = |share: &veydan_recovery::Share| {
        let mut f = Map::new();
        f.insert("share".into(), Value::String(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, share.to_bytes().as_slice())));
        Note::new(social::T_SHARE, f)
    };
    let side = TrusteeSide::new(ann.clone(), ann.shares.clone());
    let mut from_owner = note(&shares[0]);
    from_owner.from = Some(stage.owner.me());
    assert!(!side.on_note(&from_owner).await.expect("checked"), "the owner inside is not the sealer");
    let mut from_stranger = note(&shares[0]);
    from_stranger.from = Some(stranger.me());
    assert_eq!(side.on_note(&from_stranger).await.err().map(|e| e.code).as_deref(), Some(code::BAD_REQUEST), "the ack cannot go to a stranger");
    let older = veydan_recovery::shamir::split(&stage.rk, &social::unhex32(&stage.owner.me()).expect("hex"), 0, 2, 3).expect("split");
    let mut old = note(&older[0]);
    old.from = Some(stage.owner.me());
    assert!(!side.on_note(&old).await.expect("checked"), "an older generation is dropped");
    assert_eq!(trustee::generation_of(ann.shares.as_ref(), &stage.owner.me()).await.expect("box"), Some(1));

    // A person who is not approved cannot be chosen; the bounds hold.
    let mut with_stranger = stage.people.clone();
    with_stranger[0] = ApprovedContact { pubkey: stranger.me(), name: "Dan".into() };
    let e = stage.social.setup(2, with_stranger, &stage.people).await.err().map(|e| e.code);
    assert_eq!(e.as_deref(), Some(social::words::NOT_APPROVED));
    let e = stage.social.setup(1, stage.people.clone(), &stage.people).await.err().map(|e| e.code);
    assert_eq!(e.as_deref(), Some(social::words::THRESHOLD));
    let e = stage.social.setup(2, stage.people[..2].to_vec(), &stage.people).await.err().map(|e| e.code);
    assert_eq!(e.as_deref(), Some(social::words::TRUSTEES_COUNT));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_clean_device_asks_two_people_confirm_and_the_key_is_put_together() {
    let stage = stage().await;
    let clean = Device::new(&stage.world, None, "");
    let requester = requester_of(&stage, &clean);
    let view = requester.start(&stage.owner.me()).await.expect("request");
    assert_eq!(view.phase, RequestPhase::Waiting);
    assert_eq!((view.k, view.n), (2, 3));
    let code = view.code.clone().expect("code");
    assert_eq!(code.len(), 14);
    let link = view.link.clone().expect("link");
    assert_eq!(link, format!("veydan://recover/{}?c={code}", stage.owner.me()));
    assert_eq!(stage.fake.request_state().as_deref(), Some("open"));
    // A second request while one is open: the server says so.
    let other = requester.start(&stage.owner.me()).await.err().map(|e| e.code);
    assert_eq!(other.as_deref(), Some("request_open"));
    // The one under way is forgotten by that start; it goes again below.
    assert_eq!(requester.poll().phase, RequestPhase::Failed);
    let clean = Device::new(&stage.world, None, "");
    let requester = requester_of(&stage, &clean);
    stage.fake.expire_request();
    let view = requester.start(&stage.owner.me()).await.expect("request");
    let code = view.code.clone().expect("code");
    let link = view.link.clone().expect("link");

    // Ann comes by the link: the code of the link is the code of the request.
    let ann = helper(&stage, &stage.trustees[0]);
    let seen = ann.lookup(&link).await.expect("looked up");
    assert_eq!(seen.name.as_deref(), Some("Owner"));
    assert_eq!((seen.code.as_str(), seen.code_matches, seen.can_confirm), (code.as_str(), Some(true), true));
    assert_eq!(seen.state, "open");
    assert_eq!(ann.confirm(false).await.err().map(|e| e.code).as_deref(), Some(trustee::words::CONFIRM_REQUIRED));
    let sent = ann.confirm(true).await.expect("sent");
    assert_eq!(sent.expires_at, seen.expires_at);
    assert_eq!(ann.confirm(true).await.err().map(|e| e.code).as_deref(), Some(trustee::words::NOT_LOOKED_UP));
    until("one confirmation", || requester.poll().confirmed.len() == 1).await;
    assert_eq!(requester.poll().confirmed[0].npub, stage.trustees[0].me());
    assert_eq!(requester.poll().phase, RequestPhase::Waiting);

    // Ben comes by the key alone: no code to compare, the talk decides.
    let ben = helper(&stage, &stage.trustees[1]);
    let seen = ben.lookup(&stage.owner.me()).await.expect("looked up");
    assert_eq!(seen.code_matches, None);
    ben.confirm(true).await.expect("sent");
    until("the key is together", || requester.poll().phase == RequestPhase::Collected).await;
    let view = requester.poll();
    assert_eq!(view.confirmed.len(), 2);
    assert!(view.confirmed.iter().all(|c| !c.forged));

    // A third answer after the key is together changes nothing.
    let cid = helper(&stage, &stage.trustees[2]);
    cid.lookup(&link).await.expect("looked up");
    cid.confirm(true).await.expect("sent");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(requester.poll().confirmed.len(), 2);

    // The restore of §7 with the words, then close by T.
    let recorder = Recorder(Mutex::new(Vec::new()));
    let Recovered { words, closed, restored } = requester
        .finish(
            NewSecret {
                password: "4821".into(),
                kind: Some("pin".into()),
                hint: None,
            },
            Vault::Join,
            &recorder,
        )
        .await
        .expect("restored");
    assert_eq!(words, stage.rk.words().to_string());
    assert!(closed);
    assert_eq!(restored.lock, LockOutcome::Kept);
    assert_eq!(stage.fake.request_state().as_deref(), Some("closed"));
    assert_eq!(requester.poll().phase, RequestPhase::Restored);
    assert_eq!(guard(&recorder.0).len(), 1);
    // The words are what the owner wrote down: the key did not change.
    assert_eq!(RecoveryKey::from_words(&words).expect("rk"), stage.rk);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_code_a_cancel_and_a_lost_share_are_refused_by_the_trustee() {
    let stage = stage().await;
    let clean = Device::new(&stage.world, None, "");
    let requester = requester_of(&stage, &clean);
    let view = requester.start(&stage.owner.me()).await.expect("request");
    let code = view.code.expect("code");

    // «Запрос подменён»: the code of the link is not the code of the request.
    let wrong = format!("veydan://recover/{}?c=AAAA-AAAA-AAAA", stage.owner.me());
    assert_ne!(code, "AAAA-AAAA-AAAA");
    let ann = helper(&stage, &stage.trustees[0]);
    let seen = ann.lookup(&wrong).await.expect("looked up");
    assert_eq!((seen.code_matches, seen.can_confirm), (Some(false), false));
    assert_eq!(ann.confirm(true).await.err().map(|e| e.code).as_deref(), Some(trustee::words::CODE_MISMATCH));
    assert_eq!(stage.world.sent(trustee::T_ANSWER), 0);

    // The owner says «это не я»: the trustee sees it cancelled.
    let cancelled = stage.api.recovery_cancel(&view.request_id.expect("id")).await.expect("cancelled");
    assert_eq!(cancelled.state, "cancelled");
    let ben = helper(&stage, &stage.trustees[1]);
    let seen = ben.lookup(&stage.owner.me()).await.expect("looked up");
    assert_eq!((seen.state.as_str(), seen.can_confirm), ("cancelled", false));
    assert!(seen.closed_at.is_some());
    assert_eq!(ben.confirm(true).await.err().map(|e| e.code).as_deref(), Some(trustee::words::REQUEST_CLOSED));

    // Looked up while open, cancelled after: confirm reads the request again,
    // sends nothing and forgets the session.
    let clean2 = Device::new(&stage.world, None, "");
    let requester2 = requester_of(&stage, &clean2);
    let view2 = requester2.start(&stage.owner.me()).await.expect("request");
    let dan = helper(&stage, &stage.trustees[1]);
    let seen = dan.lookup(&stage.owner.me()).await.expect("looked up");
    assert_eq!((seen.state.as_str(), seen.can_confirm), ("open", true));
    stage.api.recovery_cancel(&view2.request_id.expect("id")).await.expect("cancelled");
    assert_eq!(dan.confirm(true).await.err().map(|e| e.code).as_deref(), Some(trustee::words::REQUEST_CLOSED));
    assert_eq!(stage.world.sent(trustee::T_ANSWER), 0);
    assert_eq!(dan.confirm(true).await.err().map(|e| e.code).as_deref(), Some(trustee::words::NOT_LOOKED_UP));

    // Without a share there is nothing to look up; without a request either.
    let stranger = Device::new(&stage.world, Some([0x55; 32]), "Eve");
    let eve = helper(&stage, &stranger);
    assert_eq!(eve.lookup(&stage.owner.me()).await.err().map(|e| e.code).as_deref(), Some(trustee::words::NO_SHARE));
    assert_eq!(ann.lookup("nobody").await.err().map(|e| e.code).as_deref(), Some(trustee::words::BAD_NPUB));

    // A later request runs out: the trustee sees it expired, the requester too.
    let clean = Device::new(&stage.world, None, "");
    let requester = requester_of(&stage, &clean);
    let view = requester.start(&stage.owner.me()).await.expect("request");
    stage.fake.expire_request();
    let cid = helper(&stage, &stage.trustees[2]);
    let seen = cid.lookup(&stage.owner.me()).await.expect("looked up");
    assert_eq!(seen.state, "expired");
    // The fake stamps the request by the real clock; the world's goes past it.
    stage.world.clock.store(view.expires_at.expect("end") + 1, Ordering::SeqCst);
    assert_eq!(requester.poll().phase, RequestPhase::Expired);
    let e = requester
        .finish(NewSecret { password: "4821".into(), kind: None, hint: None }, Vault::Join, &Recorder(Mutex::new(Vec::new())))
        .await
        .err()
        .map(|e| e.code);
    assert_eq!(e.as_deref(), Some(crate::request::words::REQUEST_EXPIRED));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_generation_is_dropped_and_a_stale_answer_is_ignored() {
    let stage = stage().await;
    let ann = &stage.trustees[0];
    // A reissue delivers generation 2 and waits for the acks (step 5).
    let rk2 = RecoveryKey::generate().expect("rk");
    stage.social.deliver(&rk2, 2).await.expect("delivered and acked");
    assert_eq!(trustee::generation_of(ann.shares.as_ref(), &stage.owner.me()).await.expect("box"), Some(2));
    assert_eq!(stage.social.status().await.expect("status").generation, 2);
    // The reissue is rolled back before step 6: generation 2 revoked, the
    // shares of generation 1 out again, acked again.
    stage.social.revoke(2).await.expect("revoked");
    let s = stage.social.clone();
    until_async("generation 1 acked again", || async {
        s.status().await.map(|s| s.generation == 1 && s.acked == 3).unwrap_or(false)
    })
    .await;
    assert_eq!(trustee::generation_of(ann.shares.as_ref(), &stage.owner.me()).await.expect("box"), Some(1));
    assert_eq!(stage.world.sent(social::T_REVOKE), 3);

    // Ann keeps a stale share of generation 2 (she missed the revoke): her
    // answer does not stand in the way of two answers of generation 1.
    let stale = veydan_recovery::shamir::split(&rk2, &social::unhex32(&stage.owner.me()).expect("hex"), 2, 2, 3).expect("split");
    let clean = Device::new(&stage.world, None, "");
    let requester = requester_of(&stage, &clean);
    let view = requester.start(&stage.owner.me()).await.expect("request");
    let open = stage.api.recovery_request_get(&stage.owner.me()).await.expect("get").expect("open");
    let mut fields = Map::new();
    fields.insert("request_id".into(), Value::String(view.request_id.clone().expect("id")));
    fields.insert("t_pub".into(), Value::String(open.temp_pub.clone().expect("t")));
    fields.insert("generation".into(), Value::from(2));
    fields.insert("share".into(), Value::String(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, stale[0].to_bytes().as_slice())));
    ann.send_to_key(Note::new(trustee::T_ANSWER, fields), open.temp_pub.as_deref().expect("t")).await.expect("sent");
    until("the stale answer is counted as a confirmation", || requester.poll().confirmed.len() == 1).await;
    assert_eq!(requester.poll().phase, RequestPhase::Waiting);
    let ben = helper(&stage, &stage.trustees[1]);
    ben.lookup(&stage.owner.me()).await.expect("looked up");
    ben.confirm(true).await.expect("sent");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(requester.poll().phase, RequestPhase::Waiting, "one of generation 1 is not enough");
    let cid = helper(&stage, &stage.trustees[2]);
    cid.lookup(&stage.owner.me()).await.expect("looked up");
    cid.confirm(true).await.expect("sent");
    until("the key is together from generation 1", || requester.poll().phase == RequestPhase::Collected).await;
    let recorder = Recorder(Mutex::new(Vec::new()));
    let recovered = requester
        .finish(NewSecret { password: "4821".into(), kind: None, hint: None }, Vault::Join, &recorder)
        .await
        .expect("restored");
    assert_eq!(RecoveryKey::from_words(&recovered.words).expect("rk"), stage.rk);

    // A revoke of the generation in force: the people forget, and a
    // look-up finds nothing.
    stage.social.revoke(1).await.expect("revoked");
    until_async("the shares are gone", || async {
        trustee::generation_of(ann.shares.as_ref(), &stage.owner.me()).await.expect("box").is_none()
    })
    .await;
    let eve = helper(&stage, ann);
    assert_eq!(eve.lookup(&stage.owner.me()).await.err().map(|e| e.code).as_deref(), Some(trustee::words::NO_SHARE));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_health_is_asked_with_pings_and_a_lost_share_is_sent_again() {
    let stage = stage().await;
    let status = stage.social.check(true).await.expect("pinged");
    assert_eq!(stage.world.sent(social::T_PING), 3);
    assert!(status.people.iter().all(|p| p.health == Health::Delivered), "the pongs are on their way");
    let s = stage.social.clone();
    until_async("every pong", || async { s.status().await.map(|s| s.alive).unwrap_or(0) == 3 }).await;
    let status = stage.social.status().await.expect("status");
    assert_eq!(status.checked_at, Some(NOW));
    assert!(status.people.iter().all(|p| p.health == Health::Alive));

    // Ben's devices lost the share: silence to the next ping, a week later
    // «не подтверждена» and the share goes again.
    let ben = &stage.trustees[1];
    ben.shares.clear();
    stage.world.later(social::PING_EVERY_SECS);
    stage.social.check(false).await.expect("pinged");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let status = stage.social.status().await.expect("status");
    assert_eq!(status.people[1].health, Health::Delivered, "a week to answer");
    assert_eq!(status.alive, 2);
    stage.world.later(social::PONG_WAIT_SECS);
    let status = stage.social.status().await.expect("status");
    assert_eq!(status.people[1].health, Health::Unconfirmed);
    stage.social.check(false).await.expect("sent again");
    let s = stage.social.clone();
    until_async("Ben acks the share again", || async {
        s.status().await.map(|s| s.people[1].acked_at.is_some()).unwrap_or(false)
    })
    .await;
    assert_eq!(trustee::generation_of(ben.shares.as_ref(), &stage.owner.me()).await.expect("box"), Some(1));

    // «Заменить» one person: a fresh set with the newcomer, a revoke to Ben.
    let dan = Device::new(&stage.world, Some([0x66; 32]), "Dan");
    stage.world.befriend(&stage.owner.me(), &dan.me());
    serve_trustee(dan.clone()).await;
    let approved = stage.owner.approved().await.expect("approved");
    let mut people = stage.people.clone();
    people[1] = ApprovedContact { pubkey: dan.me(), name: "Dan".into() };
    let status = stage.social.setup(2, people, &approved).await.expect("replaced");
    assert_eq!(status.people[1].name, "Dan");
    assert_eq!(stage.world.sent(social::T_REVOKE), 1);
    let s = stage.social.clone();
    until_async("the new set acked", || async { s.status().await.map(|s| s.acked).unwrap_or(0) == 3 }).await;
    until_async("Ben forgot the share", || async {
        trustee::generation_of(ben.shares.as_ref(), &stage.owner.me()).await.expect("box").is_none()
    })
    .await;

    // «Выключить»: revokes to everyone, the config gone.
    let revoked = stage.social.disable().await.expect("disabled");
    assert_eq!(revoked, 3);
    assert!(stage.fake.config().is_none());
    assert!(!stage.social.status().await.expect("status").configured);
    assert_eq!(stage.social.disable().await.err().map(|e| e.code).as_deref(), Some(social::words::NOT_SET_UP));
    assert_eq!(stage.social.revoke_all().await.expect("nothing"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reissue_without_enough_acks_fails_in_time() {
    let stage = stage().await;
    // Two of three people go off: fewer than k acks within the patience.
    for t in &stage.trustees[..2] {
        guard(&stage.world.relay).inboxes.remove(&t.me());
    }
    let social = stage.social.clone().with(stage.world.clock(), Duration::from_millis(300));
    let rk2 = RecoveryKey::generate().expect("rk");
    let e = social.deliver(&rk2, 2).await.err().map(|e| e.code);
    assert_eq!(e.as_deref(), Some(social::words::SHARES_NOT_ACKED));
    let status = social.status().await.expect("status");
    assert_eq!((status.generation, status.acked), (2, 1));
}

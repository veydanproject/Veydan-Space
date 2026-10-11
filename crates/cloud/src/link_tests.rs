// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The whole link between two devices over a relay in memory: the notes,
//! the codes, the deletions and the expiration of the relay are modelled
//! here, the crypto of both wraps' inner layer is the real one (the outer
//! NIP-44 and the relay itself are the messenger's, tested on a local relay
//! in `messenger-runtime::tempkey`).

use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use veydan_shell::seams::LinkCodeFound;

const FAST: KdfParams = KdfParams {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
};
const NOW: u64 = 1_791_000_000;

/// A note on the relay.
struct Wrap {
    to: String,
    note: Note,
    expires_at: u64,
    deleted: bool,
}

/// A code on the relay.
struct Code {
    id: String,
    d: String,
    t_pub: String,
    created_at: u64,
    expires_at: u64,
    deleted: bool,
}

#[derive(Default)]
struct Relay {
    wraps: Vec<Wrap>,
    codes: Vec<Code>,
    /// Notes of my own devices: (my key, note).
    own: Vec<(String, Note)>,
    listeners: HashMap<String, mpsc::Sender<Note>>,
    ids: u64,
}

/// The relay and the clock both devices share.
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
    fn now(&self) -> u64 {
        self.clock.load(Ordering::SeqCst)
    }
    fn later(&self, secs: u64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
    }
    fn clock(&self) -> Clock {
        let c = self.clock.clone();
        Arc::new(move || c.load(Ordering::SeqCst))
    }
    fn relay(&self) -> MutexGuard<'_, Relay> {
        guard(&self.relay)
    }
    /// Live wraps to `to`.
    fn wraps_to(&self, to: &str) -> usize {
        let now = self.now();
        self.relay()
            .wraps
            .iter()
            .filter(|w| w.to == to && !w.deleted && w.expires_at > now)
            .count()
    }
}

/// One device: its identity, its lock, its messenger on the relay.
struct Device {
    world: World,
    nsec: Mutex<Option<[u8; 32]>>,
    lock: Mutex<Option<String>>,
    unlocked: AtomicBool,
    offline: AtomicBool,
}

fn fail<T>(c: &str, m: &str) -> Result<T, CloudError> {
    Err(CloudError::new(c, m))
}

impl Device {
    fn new(world: &World, nsec: Option<[u8; 32]>, lock: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            world: world.clone(),
            nsec: Mutex::new(nsec),
            lock: Mutex::new(lock.map(str::to_string)),
            unlocked: AtomicBool::new(false),
            offline: AtomicBool::new(false),
        })
    }
    fn me(&self) -> Option<String> {
        guard(&self.nsec)
            .and_then(|n| rk::npub_of(&n))
            .map(|p| hex(&p))
    }
    fn online(&self) -> Result<(), CloudError> {
        if self.offline.load(Ordering::SeqCst) {
            return fail(code::NO_RELAY, "no relay connected");
        }
        Ok(())
    }
    /// The secret of the lock, counted as an unlock.
    fn check(&self, password: Option<&str>, missing: &str) -> Result<(), CloudError> {
        let lock = guard(&self.lock).clone();
        match (lock, password) {
            (None, _) => Ok(()),
            (Some(_), None) => fail(missing, "the secret of the lock is needed"),
            (Some(l), Some(p)) if l == p => {
                self.unlocked.store(true, Ordering::SeqCst);
                Ok(())
            }
            (Some(_), Some(_)) => fail(code::WRONG_PASSWORD, "wrong"),
        }
    }
    fn linker(self: &Arc<Self>) -> Linker {
        Linker::new(self.clone(), self.clone(), None, Arc::default()).with(FAST, self.world.clock())
    }
}

impl PeerNotes for Device {
    fn send<'a>(&'a self, _: Note, _: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async { fail(code::NOT_SUPPORTED, "not here") })
    }
    fn send_own(&self, mut note: Note) -> SeamFuture<'_, ()> {
        Box::pin(async move {
            self.online()?;
            let me = self
                .me()
                .ok_or_else(|| CloudError::new(code::NO_IDENTITY, "none"))?;
            note.from = Some(me.clone());
            self.world.relay().own.push((me, note));
            Ok(())
        })
    }
    fn send_to_key<'a>(&'a self, mut note: Note, pubkey: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            self.online()?;
            assert_eq!(note.t, T_DEVICE_LINK);
            note.from = self.me();
            let expires_at = note.expires_at.unwrap_or(0);
            assert!(
                expires_at <= self.world.now() + LINK_TTL_SECS,
                "a link expires within ten minutes"
            );
            if expires_at <= self.world.now() {
                return fail(code::NO_RELAY, "the relay refuses what expired");
            }
            let mut relay = self.world.relay();
            if let Some(tx) = relay.listeners.get(pubkey) {
                let _ = tx.try_send(note.clone());
            }
            relay.wraps.push(Wrap {
                to: pubkey.to_string(),
                note,
                expires_at,
                deleted: false,
            });
            Ok(())
        })
    }
    fn listen_as(&self, t: TempKey) -> SeamFuture<'_, mpsc::Receiver<Note>> {
        Box::pin(async move {
            self.online()?;
            let t_pub = hex(&rk::npub_of(t.secret_bytes()).expect("key"));
            let (tx, rx) = mpsc::channel(8);
            let now = self.world.now();
            let mut relay = self.world.relay();
            for w in relay
                .wraps
                .iter()
                .filter(|w| w.to == t_pub && !w.deleted && w.expires_at > now)
            {
                let _ = tx.try_send(w.note.clone());
            }
            relay.listeners.insert(t_pub, tx);
            Ok(rx)
        })
    }
}

impl DeviceLink for Device {
    fn hand_over<'a>(&'a self, password: Option<&'a str>) -> SeamFuture<'a, Zeroizing<[u8; 32]>> {
        Box::pin(async move {
            self.check(password, code::WRONG_PASSWORD)?;
            let nsec =
                guard(&self.nsec).ok_or_else(|| CloudError::new(code::NO_IDENTITY, "none"))?;
            Ok(Zeroizing::new(nsec))
        })
    }
    fn take_over<'a>(
        &'a self,
        nsec: Zeroizing<[u8; 32]>,
        password: Option<&'a str>,
    ) -> SeamFuture<'a, String> {
        Box::pin(async move {
            let key = hex(&rk::npub_of(&nsec).expect("key"));
            if let Some(me) = self.me() {
                return if me == key {
                    Ok(me)
                } else {
                    fail(code::IDENTITY_EXISTS, "another")
                };
            }
            if !self.unlocked.load(Ordering::SeqCst) {
                self.check(password, code::LOCKED)?;
            }
            *guard(&self.nsec) = Some(*nsec);
            Ok(key)
        })
    }
    fn publish_code<'a>(
        &'a self,
        t: &'a TempKey,
        d: &'a str,
        expires_at: u64,
    ) -> SeamFuture<'a, String> {
        Box::pin(async move {
            self.online()?;
            let t_pub = hex(&rk::npub_of(t.secret_bytes()).expect("key"));
            let mut relay = self.world.relay();
            relay.ids += 1;
            let id = format!("{:064x}", relay.ids);
            let created_at = self.world.now();
            relay.codes.push(Code {
                id: id.clone(),
                d: d.into(),
                t_pub,
                created_at,
                expires_at,
                deleted: false,
            });
            Ok(id)
        })
    }
    fn find_code<'a>(&'a self, d: &'a str) -> SeamFuture<'a, Option<LinkCodeFound>> {
        Box::pin(async move {
            self.online()?;
            let now = self.world.now();
            Ok(self
                .world
                .relay()
                .codes
                .iter()
                .filter(|c| c.d == d && !c.deleted && c.expires_at > now)
                .max_by_key(|c| c.created_at)
                .map(|c| LinkCodeFound {
                    t_pub: c.t_pub.clone(),
                    expires_at: Some(c.expires_at),
                }))
        })
    }
    fn delete_code<'a>(&'a self, t: &'a TempKey, event_id: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let t_pub = hex(&rk::npub_of(t.secret_bytes()).expect("key"));
            for c in self.world.relay().codes.iter_mut() {
                if c.id == event_id && c.t_pub == t_pub {
                    c.deleted = true;
                }
            }
            Ok(())
        })
    }
    fn await_done<'a>(&'a self, t_pub: &'a str, until: u64) -> SeamFuture<'a, bool> {
        Box::pin(async move {
            let me = self.me().unwrap_or_default();
            for _ in 0..500 {
                let came = self.world.relay().own.iter().any(|(who, n)| {
                    *who == me
                        && n.t == T_DEVICE_LINK_DONE
                        && n.fields.get("t_pub").and_then(Value::as_str) == Some(t_pub)
                });
                if came {
                    return Ok(true);
                }
                if self.world.now() >= until {
                    return Ok(false);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(false)
        })
    }
    fn withdraw<'a>(&'a self, t_pub: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let me = self.me();
            for w in self.world.relay().wraps.iter_mut() {
                if w.to == t_pub && w.note.from == me {
                    w.deleted = true;
                }
            }
            Ok(())
        })
    }
    fn qr_svg<'a>(&'a self, text: &'a str) -> SeamFuture<'a, String> {
        Box::pin(async move { Ok(format!("<svg data-text=\"{text}\"/>")) })
    }
}

const NSEC: [u8; 32] = [0x42; 32];

async fn until_state(b: &Linker, want: LinkState) -> LinkPoll {
    for _ in 0..500 {
        let poll = b.poll().await;
        if poll.state == want {
            return poll;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the link never reached {want:?}: {:?}", b.poll().await);
}

async fn until<F: Fn() -> bool>(what: &str, f: F) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} never happened");
}

/// Without a camera: B shows the words, A types them, both see the same
/// fingerprint, the nsec comes over, B takes the identity, A deletes its
/// wrap on B's word and B its code; nothing of the link stays.
#[tokio::test]
async fn a_device_is_linked_by_the_words() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), Some("4821")),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());

    let shown = b.start().await.expect("start");
    assert_eq!(shown.words.len(), 8);
    assert_eq!(shown.fingerprint.len(), 4);
    assert_eq!(shown.expires_at, NOW + LINK_TTL_SECS);
    assert!(shown.uri.starts_with("veydan://link/") && shown.qr_svg.contains(&shown.uri));
    assert_eq!(b.poll().await.state, LinkState::Waiting);

    let typed = shown.words.join("  ").to_uppercase();
    let found = a.resolve(&typed).await.expect("resolve");
    assert_eq!(found.t_pub, shown.t_pub);
    assert_eq!(
        found.fingerprint, shown.fingerprint,
        "both screens show the same 4 words"
    );

    let sent = a.approve(Some("4821")).await.expect("approve");
    assert_eq!(sent.t_pub, shown.t_pub);
    assert_eq!(world.wraps_to(&shown.t_pub), 1);
    let got = until_state(&b, LinkState::Received).await;
    let me = hex(&rk::npub_of(&NSEC).expect("key"));
    assert_eq!(got.npub.as_deref(), Some(me.as_str()));

    let done = b.receive(None, Vault::Join).await.expect("receive");
    assert_eq!(done.npub, me);
    assert!(done.done_sent && !done.vault_joined);
    assert_eq!(
        new.me().as_deref(),
        Some(me.as_str()),
        "the new device holds the identity"
    );
    assert_eq!(b.poll().await.state, LinkState::Idle, "T is gone");

    // A heard `device.link.done` and deleted the wrap with the nsec.
    until("the wrap deleted", || world.wraps_to(&shown.t_pub) == 0).await;
    // B deleted its code: the words lead nowhere now.
    let again = a
        .resolve(&shown.words.join(" "))
        .await
        .err()
        .map(|e| e.code);
    assert_eq!(again.as_deref(), Some(link_code::LINK_NOT_FOUND));
    assert!(world.relay().codes.iter().all(|c| c.deleted));
}

/// With a camera: the QR carries T, no lookup on the relays.
#[tokio::test]
async fn a_device_is_linked_by_the_qr() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    // The code under the words is gone, the QR still leads to T.
    for c in world.relay().codes.iter_mut() {
        c.deleted = true;
    }
    let found = a.resolve(&shown.uri).await.expect("resolve");
    assert_eq!(found.fingerprint, shown.fingerprint);
    // A device without a lock gives the identity without a secret.
    a.approve(None).await.expect("approve");
    until_state(&b, LinkState::Received).await;
    assert_eq!(
        b.receive(None, Vault::Join).await.expect("receive").npub,
        hex(&rk::npub_of(&NSEC).expect("key"))
    );
}

/// A wrong code gives a blob B cannot open: B drops it and keeps waiting.
#[tokio::test]
async fn a_wrong_code_does_not_open() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    let mut words = shown.words.clone();
    words[0] = if words[0] == "zoo" {
        "abandon".into()
    } else {
        "zoo".into()
    };
    let wrong = format!("veydan://link/{}/{}", shown.t_pub, words.join("-"));
    let found = a.resolve(&wrong).await.expect("resolve");
    assert_eq!(
        found.fingerprint, shown.fingerprint,
        "the same T: the fingerprint cannot tell"
    );
    a.approve(None).await.expect("sent");
    assert_eq!(world.wraps_to(&shown.t_pub), 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        b.poll().await.state,
        LinkState::Waiting,
        "the blob did not open with the code of B"
    );
    assert_eq!(
        b.receive(None, Vault::Join).await.err().map(|e| e.code).as_deref(),
        Some(link_code::LINK_STATE)
    );
    assert!(new.me().is_none());
}

/// A blob whose npub is not its sealer's is dropped (messenger-wire.md §3).
#[tokio::test]
async fn a_blob_from_another_sealer_is_dropped() {
    let world = World::new();
    let new = Device::new(&world, None, None);
    let b = new.linker();
    let shown = b.start().await.expect("start");
    let t_pub = unhex32(&shown.t_pub).expect("t");
    let code = LinkCode::parse(&shown.words.join(" ")).expect("code");
    let payload = LinkPayload {
        npub: rk::npub_of(&NSEC).expect("key"),
        nsec: NSEC,
        created_at: NOW,
        sync: None,
    };
    let blob = devicelink::seal_with(&code, &t_pub, &payload, &FAST, [9; 24]).expect("seal");
    let mut fields = Map::new();
    fields.insert("blob".into(), Value::String(B64.encode(&blob)));
    let mut forged = Note::new(T_DEVICE_LINK, fields);
    forged.from = Some(hex(&rk::npub_of(&[0x43; 32]).expect("key")));
    let tx = world
        .relay()
        .listeners
        .get(&shown.t_pub)
        .cloned()
        .expect("T listens");
    tx.send(forged.clone()).await.expect("delivered");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(b.poll().await.state, LinkState::Waiting);
    // The same blob from its own key opens.
    forged.from = Some(hex(&payload.npub));
    tx.send(forged).await.expect("delivered");
    until_state(&b, LinkState::Received).await;
}

/// Ten minutes: B says the code is out of date, A no longer sends, and a
/// wrap that comes late is not taken.
#[tokio::test]
async fn the_ten_minutes_are_kept() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    world.later(LINK_TTL_SECS);
    assert_eq!(b.poll().await.state, LinkState::Expired);
    // T is wiped and the code deleted at once, not at the next start.
    assert!(guard(&b.sessions.new_side).is_none(), "T is gone");
    assert!(world.relay().codes.iter().all(|c| c.deleted));
    let again = b.poll().await;
    assert_eq!(
        (again.state, again.expires_at),
        (LinkState::Expired, Some(shown.expires_at))
    );
    assert_eq!(
        b.receive(None, Vault::Join).await.err().map(|e| e.code).as_deref(),
        Some(link_code::LINK_EXPIRED)
    );
    let late = a.approve(None).await.err().map(|e| e.code);
    assert_eq!(late.as_deref(), Some(link_code::LINK_EXPIRED));
    assert_eq!(world.wraps_to(&shown.t_pub), 0, "nothing went");
    // The code is gone from the relay with its expiration.
    assert_eq!(
        a.resolve(&shown.words.join(" "))
            .await
            .err()
            .map(|e| e.code)
            .as_deref(),
        Some(link_code::LINK_NOT_FOUND)
    );

    // A new start is a new T and a new code.
    let next = b.start().await.expect("again");
    assert_ne!(next.t_pub, shown.t_pub);
    assert_ne!(next.words, shown.words);
    assert_eq!(b.poll().await.state, LinkState::Waiting);
}

/// The secret of the lock on A: without it, or wrong, the nsec stays.
#[tokio::test]
async fn the_lock_of_the_old_device_guards_the_nsec() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), Some("4821")),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    for wrong in [None, Some("0000")] {
        let e = a.approve(wrong).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::WRONG_PASSWORD));
    }
    assert_eq!(world.wraps_to(&shown.t_pub), 0, "the nsec did not go");
    a.approve(Some("4821")).await.expect("approve");
    let twice = a.approve(Some("4821")).await.err().map(|e| e.code);
    assert_eq!(twice.as_deref(), Some(link_code::LINK_STATE));
}

/// The lock of the new device (the row the vault brought): the identity
/// waits for its secret, and the link is kept between the tries.
#[tokio::test]
async fn the_lock_of_the_new_device_is_opened_first() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, Some("4821")),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    a.approve(None).await.expect("approve");
    until_state(&b, LinkState::Received).await;
    assert_eq!(
        b.receive(None, Vault::Join).await.err().map(|e| e.code).as_deref(),
        Some(code::LOCKED)
    );
    assert_eq!(
        b.receive(Some("0000"), Vault::Join)
            .await
            .err()
            .map(|e| e.code)
            .as_deref(),
        Some(code::WRONG_PASSWORD)
    );
    assert_eq!(
        b.poll().await.state,
        LinkState::Received,
        "kept for another try"
    );
    b.receive(Some("4821"), Vault::Join).await.expect("receive");
    assert!(new.me().is_some());
}

/// No relay: B publishes nothing and keeps nothing; A finds nothing.
#[tokio::test]
async fn without_the_relays_nothing_is_published() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    new.offline.store(true, Ordering::SeqCst);
    let e = new.linker().start().await.err().map(|e| e.code);
    assert_eq!(e.as_deref(), Some(code::NO_RELAY));
    assert!(world.relay().codes.is_empty());
    assert_eq!(new.linker().poll().await.state, LinkState::Idle);
    old.offline.store(true, Ordering::SeqCst);
    let words = "abandon ability able about above absent absorb abstract";
    assert_eq!(
        old.linker()
            .resolve(words)
            .await
            .err()
            .map(|e| e.code)
            .as_deref(),
        Some(code::NO_RELAY)
    );
}

/// The vault of a device in these tests: what it is bound to, what it was
/// asked to enter, and whether its storage is within reach.
#[derive(Default)]
struct TestVault {
    bound: Option<SyncKeys>,
    entered: Mutex<Vec<SyncKeys>>,
    away: bool,
}

impl VaultEntry for TestVault {
    fn keys(&self) -> SeamFuture<'_, Option<SyncKeys>> {
        Box::pin(async move { Ok(self.bound.clone()) })
    }
    fn enter<'a>(
        &'a self,
        nsec: &'a Zeroizing<[u8; 32]>,
        sync: &'a SyncKeys,
        _: Option<&'a str>,
    ) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(**nsec, NSEC, "the vault is entered with the nsec that came");
            if self.away {
                return fail(crate::escrow::words::VAULT_UNREACHABLE, "no such folder");
            }
            guard(&self.entered).push(sync.clone());
            Ok(())
        })
    }
}

fn linker_with(device: &Arc<Device>, world: &World, vault: Arc<TestVault>) -> Linker {
    Linker::new(device.clone(), device.clone(), Some(vault), Arc::default())
        .with(FAST, world.clock())
}

fn s3_vault() -> SyncKeys {
    SyncKeys {
        vault_id: "3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c".into(),
        vmk: [0x77; 32],
        storage: veydan_recovery::Storage::S3 {
            endpoint: "https://s3.example.com".into(),
            region: "eu-central-1".into(),
            bucket: "notes".into(),
            prefix: "veydan".into(),
            access_key: "AKIAEXAMPLE".into(),
            secret_key: "s3-secret".into(),
            path_style: false,
        },
    }
}

/// The vault of A goes along with the nsec, with where it lives and the
/// access to it: B enters that very vault before it takes the identity.
#[tokio::test]
async fn the_vault_of_the_old_device_is_entered_on_the_new_one() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let of_a = Arc::new(TestVault {
        bound: Some(s3_vault()),
        ..TestVault::default()
    });
    let of_b = Arc::new(TestVault::default());
    let a = linker_with(&old, &world, of_a.clone());
    let b = linker_with(&new, &world, of_b.clone());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    a.approve(None).await.expect("approve");
    until_state(&b, LinkState::Received).await;
    let done = b.receive(None, Vault::Join).await.expect("receive");
    assert!(done.vault_joined && done.done_sent);
    assert_eq!(*guard(&of_b.entered), [s3_vault()]);
    assert!(guard(&of_a.entered).is_empty());
    assert_eq!(new.me(), old.me());
}

/// A device bound to no vault sends none: the identity comes alone.
#[tokio::test]
async fn a_device_without_a_vault_sends_the_identity_alone() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let of_b = Arc::new(TestVault::default());
    let a = linker_with(&old, &world, Arc::new(TestVault::default()));
    let b = linker_with(&new, &world, of_b.clone());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    a.approve(None).await.expect("approve");
    until_state(&b, LinkState::Received).await;
    let done = b.receive(None, Vault::Join).await.expect("receive");
    assert!(!done.vault_joined);
    assert!(guard(&of_b.entered).is_empty());
    assert_eq!(new.me(), old.me());
}

/// A storage the new device cannot reach is said, the link is kept, and
/// the person goes on without the vault: the identity alone.
#[tokio::test]
async fn a_storage_out_of_reach_keeps_the_link_until_the_person_goes_on_without_it() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let of_a = Arc::new(TestVault {
        bound: Some(s3_vault()),
        ..TestVault::default()
    });
    let of_b = Arc::new(TestVault {
        away: true,
        ..TestVault::default()
    });
    let a = linker_with(&old, &world, of_a);
    let b = linker_with(&new, &world, of_b.clone());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    a.approve(None).await.expect("approve");
    until_state(&b, LinkState::Received).await;
    assert_eq!(
        b.receive(None, Vault::Join).await.err().map(|e| e.code).as_deref(),
        Some(crate::escrow::words::VAULT_UNREACHABLE)
    );
    assert_eq!(new.me(), None, "nothing was taken over");
    assert_eq!(b.poll().await.state, LinkState::Received, "the link is kept");
    let done = b.receive(None, Vault::Skip).await.expect("receive");
    assert!(!done.vault_joined && done.done_sent);
    assert_eq!(new.me(), old.me());
}

/// What A types: the words of the list, or a link; anything else is said so.
#[tokio::test]
async fn what_is_typed_is_checked() {
    let world = World::new();
    let a = Device::new(&world, Some(NSEC), None).linker();
    for (input, word) in [
        ("abandon about", link_code::LINK_CODE),
        (
            "abandon about zoo zone zero yellow year veydan",
            link_code::LINK_CODE,
        ),
        ("veydan://link/xx/abandon", link_code::LINK_CODE),
        (
            "abandon ability able about above absent absorb abstract",
            link_code::LINK_NOT_FOUND,
        ),
    ] {
        assert_eq!(
            a.resolve(input).await.err().map(|e| e.code).as_deref(),
            Some(word),
            "{input}"
        );
    }
    assert_eq!(
        a.approve(None).await.err().map(|e| e.code).as_deref(),
        Some(link_code::LINK_STATE)
    );
    assert!(a.cancel().await.is_ok());
    assert_eq!(unhex32(&"ab".repeat(32)), Some([0xab; 32]));
    assert_eq!(unhex32(&"AB".repeat(32)), None);
}

/// «Не совпадает» on A after it sent (a late change of mind): the wrap goes.
#[tokio::test]
async fn a_cancel_deletes_what_was_sent() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    a.resolve(&shown.uri).await.expect("resolve");
    a.approve(None).await.expect("approve");
    a.cancel().await.expect("cancel");
    assert_eq!(world.wraps_to(&shown.t_pub), 0);
    b.cancel().await.expect("cancel");
    assert_eq!(b.poll().await.state, LinkState::Idle);
    assert!(world.relay().codes.iter().all(|c| c.deleted));
}

/// The timer of B's listener runs out: T and the code go at once, and a
/// timer of an earlier start changes nothing.
#[tokio::test]
async fn the_end_of_the_listening_wipes_t() {
    let world = World::new();
    let b = Device::new(&world, None, None).linker();
    b.start().await.expect("start");
    let round = *guard(&b.sessions.rounds);
    b.expire(Some(round + 1)).await;
    assert_eq!(b.poll().await.state, LinkState::Waiting, "another round");
    b.expire(Some(round)).await;
    assert!(guard(&b.sessions.new_side).is_none());
    assert!(world.relay().codes.iter().all(|c| c.deleted));
    assert_eq!(b.poll().await.state, LinkState::Expired);
    // «Новый код»: a new start forgets the expired one.
    b.start().await.expect("again");
    assert_eq!(b.poll().await.state, LinkState::Waiting);
    b.cancel().await.expect("cancel");
    assert_eq!(b.poll().await.state, LinkState::Idle);
}

/// A finds the code late: it may send only until the code's own end on
/// the relays, not ten minutes from its own clock.
#[tokio::test]
async fn a_code_found_late_keeps_its_own_end() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    world.later(500);
    let found = a.resolve(&shown.words.join(" ")).await.expect("resolve");
    assert_eq!(
        found.expires_at, shown.expires_at,
        "the code's end, not now + 600"
    );
    world.later(150);
    assert_eq!(
        a.approve(None).await.err().map(|e| e.code).as_deref(),
        Some(link_code::LINK_EXPIRED)
    );
    assert_eq!(world.wraps_to(&shown.t_pub), 0, "nothing went");
}

/// The link typed by hand with the scheme in capitals still opens.
#[tokio::test]
async fn the_scheme_of_a_link_is_read_in_any_case() {
    let world = World::new();
    let (old, new) = (
        Device::new(&world, Some(NSEC), None),
        Device::new(&world, None, None),
    );
    let (a, b) = (old.linker(), new.linker());
    let shown = b.start().await.expect("start");
    let rest = &shown.uri[LINK_URI_PREFIX.len()..];
    for typed in [format!("VEYDAN://LINK/{rest}"), shown.uri.to_uppercase()] {
        let found = a.resolve(&typed).await.expect("resolve");
        assert_eq!(found.t_pub, shown.t_pub, "{typed}");
        assert_eq!(found.fingerprint, shown.fingerprint);
    }
    a.approve(None).await.expect("approve");
    until_state(&b, LinkState::Received).await;
}

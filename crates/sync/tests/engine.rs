// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Engine behaviour over a LocalDir vault: log chains, compaction, own-log
//! collisions, lost-result adoption, envelope binding and manifest hardening.

use veydan_sync::envelope::{self, Kind};
use veydan_sync::{Engine, Hlc, LocalDir, LocalState, Manifest, Op, Storage, SyncError, Vmk};

const PASS: &str = "correct horse battery staple";

fn storage(dir: &std::path::Path) -> Box<dyn Storage> {
    Box::new(LocalDir::new(dir))
}

fn op(id: &str, wall_ms: u64, device: &str, value: &str) -> Op {
    Op {
        entity_type: "note".into(),
        entity_id: id.into(),
        hlc: Hlc {
            wall_ms,
            counter: 0,
            device_id: device.into(),
        },
        deleted: false,
        payload: serde_json::json!({ "v": value }),
    }
}

async fn pull_ops(engine: &Engine, state: &mut LocalState) -> Vec<Op> {
    let res = engine.pull(state, |_, _, _| {}).await.expect("pull");
    assert!(res.errors.is_empty(), "pull errors: {:?}", res.errors);
    res.ops
}

/// Vault with one engine per device id, all sharing the master key.
async fn vault(dir: &std::path::Path, devices: &[&str]) -> (Vec<Engine>, Vmk) {
    let (first, vmk) = Engine::create(storage(dir), PASS, devices[0])
        .await
        .expect("create");
    let vault_id = first.vault_id().to_string();
    let mut engines = vec![first];
    for d in &devices[1..] {
        engines.push(Engine::with_key(storage(dir), &vmk, &vault_id, d));
    }
    (engines, vmk)
}

#[tokio::test]
async fn two_devices_exchange_ops_in_chain_order() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a", "b"]).await;
    let (a, b) = (&engines[0], &engines[1]);
    let (mut sa, mut sb) = (LocalState::default(), LocalState::default());

    a.push(&mut sa, vec![op("n1", 100, "a", "one")])
        .await
        .unwrap();
    a.push(&mut sa, vec![op("n2", 200, "a", "two")])
        .await
        .unwrap();
    assert_eq!(sa.own_seq, 2);

    let got = pull_ops(b, &mut sb).await;
    assert_eq!(
        got.iter().map(|o| o.entity_id.as_str()).collect::<Vec<_>>(),
        ["n1", "n2"]
    );
    assert_eq!(sb.peers["a"].seq, 2);
    assert_eq!(sb.peers["a"].hash, sa.own_head_hash);

    // Nothing new: no ops, head unchanged.
    assert!(pull_ops(b, &mut sb).await.is_empty());
    assert_eq!(sb.peers["a"].seq, 2);
}

#[tokio::test]
async fn compact_folds_log_and_peer_catches_up_through_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a", "b"]).await;
    let (a, b) = (&engines[0], &engines[1]);
    let (mut sa, mut sb) = (LocalState::default(), LocalState::default());

    a.push(&mut sa, vec![op("n1", 100, "a", "v1")])
        .await
        .unwrap();
    // Peer sees the first chunk before compaction.
    assert_eq!(pull_ops(b, &mut sb).await.len(), 1);

    a.push(&mut sa, vec![op("n1", 200, "a", "v2")])
        .await
        .unwrap();
    a.push(&mut sa, vec![op("n2", 300, "a", "x")])
        .await
        .unwrap();
    a.compact(&sa).await.unwrap();
    assert_eq!(a.own_chunk_count().await.unwrap(), 0);

    // Chunks 2..3 are gone; the snapshot must carry the latest per entity.
    let got = pull_ops(b, &mut sb).await;
    let mut ids: Vec<(String, String)> = got
        .iter()
        .map(|o| {
            (
                o.entity_id.clone(),
                o.payload["v"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            ("n1".to_string(), "v2".to_string()),
            ("n2".to_string(), "x".to_string())
        ]
    );
    assert_eq!(sb.peers["a"].seq, 3);
    assert_eq!(sb.peers["a"].hash, sa.own_head_hash);

    // Pushing after compaction chains onto the snapshot head.
    a.push(&mut sa, vec![op("n3", 400, "a", "y")])
        .await
        .unwrap();
    let got = pull_ops(b, &mut sb).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].entity_id, "n3");
}

#[tokio::test]
async fn second_writer_with_same_device_id_gets_collision_and_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a", "a", "b"]).await;
    let (a1, a2, b) = (&engines[0], &engines[1], &engines[2]);
    let (mut s1, mut s2, mut sb) = (
        LocalState::default(),
        LocalState::default(),
        LocalState::default(),
    );

    a1.push(&mut s1, vec![op("n1", 100, "a", "first")])
        .await
        .unwrap();
    let err = a2
        .push(&mut s2, vec![op("n1", 100, "a", "second")])
        .await
        .unwrap_err();
    assert!(matches!(err, SyncError::OwnLogCollision(1)), "{err}");
    assert_eq!(s2.own_seq, 0, "collision must not advance the loser");

    let got = pull_ops(b, &mut sb).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload["v"], "first");
}

#[tokio::test]
async fn replaying_a_push_whose_result_was_lost_adopts_existing_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a"]).await;
    let a = &engines[0];
    let mut state = LocalState::default();
    let ops = vec![op("n1", 100, "a", "one")];

    a.push(&mut state, ops.clone()).await.unwrap();
    let head = state.own_head_hash.clone();

    // Simulate "written, but the confirmation never made it back".
    let mut replay = LocalState::default();
    a.push(&mut replay, ops).await.unwrap();
    assert_eq!(replay.own_seq, 1);
    assert_eq!(replay.own_head_hash, head);
    assert_eq!(a.own_chunk_count().await.unwrap(), 1);
}

#[test]
fn envelope_is_bound_to_its_address() {
    let key = [7u8; 32];
    let sealed = envelope::seal(&key, "vault", Kind::Chunk, "a/1", b"payload").unwrap();

    assert_eq!(
        envelope::open(&key, "vault", Kind::Chunk, "a/1", &sealed).unwrap(),
        b"payload"
    );
    assert!(matches!(
        envelope::open(&key, "vault", Kind::Chunk, "a/2", &sealed),
        Err(SyncError::Integrity(_))
    ));
    assert!(matches!(
        envelope::open(&key, "other", Kind::Chunk, "a/1", &sealed),
        Err(SyncError::Integrity(_))
    ));
    assert!(matches!(
        envelope::open(&key, "vault", Kind::Snapshot, "a/1", &sealed),
        Err(SyncError::Integrity(_))
    ));
}

#[test]
fn manifest_with_absurd_kdf_cost_is_rejected_before_hashing() {
    let (mut manifest, _) = Manifest::create(PASS).unwrap();
    manifest.kdf.m_cost_kib = u32::MAX;
    let started = std::time::Instant::now();
    let Err(err) = manifest.unlock(PASS) else {
        panic!("huge m_cost accepted")
    };
    assert!(matches!(err, SyncError::Format(_)), "{err}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "must fail without deriving"
    );

    let (mut manifest, _) = Manifest::create(PASS).unwrap();
    manifest.kdf.t_cost = 1_000;
    assert!(matches!(manifest.unlock(PASS), Err(SyncError::Format(_))));

    let (manifest, vmk) = Manifest::create(PASS).unwrap();
    assert_eq!(manifest.unlock(PASS).unwrap().0, vmk.0);
    assert!(matches!(
        manifest.unlock("nope"),
        Err(SyncError::WrongPassphrase)
    ));
}

#[tokio::test]
async fn compact_keeps_a_tombstone_for_ever() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a", "b", "c"]).await;
    let (a, b, c) = (&engines[0], &engines[1], &engines[2]);
    let (mut sa, mut sb, mut sc) = (
        LocalState::default(),
        LocalState::default(),
        LocalState::default(),
    );

    a.push(&mut sa, vec![op("n1", 100, "a", "v1")])
        .await
        .unwrap();
    assert_eq!(pull_ops(b, &mut sb).await.len(), 1);
    let mut gone = op("n1", 200, "b", "");
    gone.deleted = true;
    b.push(&mut sb, vec![gone]).await.unwrap();
    // Far later than any limit a tombstone used to have (200 days).
    b.compact(&sb).await.unwrap();
    assert_eq!(b.own_chunk_count().await.unwrap(), 0);

    // A device that joins now folds both logs and ends with the tombstone.
    let got = pull_ops(c, &mut sc).await;
    let latest = veydan_sync::log::fold_latest(got);
    assert_eq!(latest.len(), 1);
    assert!(latest[0].deleted);
    assert_eq!(latest[0].hlc.wall_ms, 200);
}

/// Names in `dir` (recursively) that are temp files of a write.
fn temp_files(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            out.extend(temp_files(&entry.path()));
        } else {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".tmp") {
                out.push(name);
            }
        }
    }
    out
}

#[tokio::test]
async fn put_if_absent_of_one_key_in_a_folder_has_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let (one, two) = (LocalDir::new(dir.path()), LocalDir::new(dir.path()));
    let (a, b) = tokio::join!(
        one.put_if_absent("devices/a/log/00000001", b"first"),
        two.put_if_absent("devices/a/log/00000001", b"second"),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(a ^ b, "exactly one writer wins: {a} {b}");
    let kept = one.get("devices/a/log/00000001").await.unwrap().unwrap();
    assert_eq!(
        kept,
        if a {
            b"first".to_vec()
        } else {
            b"second".to_vec()
        }
    );
    assert!(temp_files(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn put_if_absent_races_on_threads_keep_one_whole_winner_per_key() {
    let dir = tempfile::tempdir().unwrap();
    let writers: Vec<_> = (0..8u8)
        .map(|w| {
            let root = dir.path().to_path_buf();
            tokio::spawn(async move {
                let s = LocalDir::new(root);
                let mut won = Vec::new();
                for k in 0..40 {
                    let body = vec![w; 4096];
                    if s.put_if_absent(&format!("log/{k:08}"), &body)
                        .await
                        .unwrap()
                    {
                        won.push(k);
                    }
                }
                (w, won)
            })
        })
        .collect();
    let mut winner = std::collections::HashMap::new();
    for writer in writers {
        let (w, won) = writer.await.unwrap();
        for k in won {
            assert!(winner.insert(k, w).is_none(), "two winners of log/{k:08}");
        }
    }
    assert_eq!(winner.len(), 40, "every key has its winner");
    let s = LocalDir::new(dir.path());
    for (k, w) in winner {
        let body = s.get(&format!("log/{k:08}")).await.unwrap().unwrap();
        assert_eq!(body, vec![w; 4096], "log/{k:08} is the winner's, whole");
    }
    assert!(temp_files(dir.path()).is_empty());
}

#[tokio::test]
async fn put_replaces_a_key_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let s = LocalDir::new(dir.path());
    s.put("manifest.json", b"one").await.unwrap();
    s.put("manifest.json", b"two").await.unwrap();
    assert_eq!(s.get("manifest.json").await.unwrap().unwrap(), b"two");
    assert!(temp_files(dir.path()).is_empty());
    assert_eq!(s.list("").await.unwrap(), vec!["manifest.json".to_string()]);
}

/// A folder whose manifest another device writes between our probe and our
/// write: the probe sees nothing, the write sees it.
struct LateManifest(LocalDir);

#[async_trait::async_trait]
impl Storage for LateManifest {
    async fn list(&self, _: &str) -> veydan_sync::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
        if key == "manifest.json" {
            return Ok(None);
        }
        self.0.get(key).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
        self.0.put(key, data).await
    }
    async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
        self.0.delete(key).await
    }
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> veydan_sync::Result<bool> {
        self.0.put_if_absent(key, data).await
    }
}

/// A storage that answers every write and keeps none.
struct Forgetful(LocalDir);

#[async_trait::async_trait]
impl Storage for Forgetful {
    async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
        self.0.list(prefix).await
    }
    async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
        self.0.get(key).await
    }
    async fn put(&self, _: &str, _: &[u8]) -> veydan_sync::Result<()> {
        Ok(())
    }
    async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
        self.0.delete(key).await
    }
    async fn put_if_absent(&self, _: &str, _: &[u8]) -> veydan_sync::Result<bool> {
        Ok(true)
    }
}

#[tokio::test]
async fn create_refuses_a_folder_that_is_not_empty() {
    let dir = tempfile::tempdir().unwrap();
    Engine::create(storage(dir.path()), PASS, "a")
        .await
        .unwrap();
    let again = Engine::create(storage(dir.path()), PASS, "b").await;
    assert!(matches!(again, Err(SyncError::VaultExists)));

    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("notes.txt"), b"mine").unwrap();
    let foreign = Engine::create(storage(other.path()), PASS, "a").await;
    assert!(matches!(foreign, Err(SyncError::ForeignStorage)));
}

#[tokio::test]
async fn create_does_not_replace_a_manifest_written_after_its_probe() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = Engine::create(storage(dir.path()), PASS, "a")
        .await
        .unwrap();
    let before = std::fs::read(dir.path().join("manifest.json")).unwrap();

    let late = Box::new(LateManifest(LocalDir::new(dir.path())));
    let second = Engine::create(late, "another passphrase", "b").await;
    assert!(matches!(second, Err(SyncError::VaultExists)));
    assert_eq!(
        std::fs::read(dir.path().join("manifest.json")).unwrap(),
        before
    );
    first.verify_manifest().await.unwrap();
}

#[tokio::test]
async fn a_manifest_the_storage_does_not_keep_fails_create_and_change_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let lost = Engine::create(Box::new(Forgetful(LocalDir::new(dir.path()))), PASS, "a").await;
    assert!(matches!(lost, Err(SyncError::Storage(_))));

    let (engine, vmk) = Engine::create(storage(dir.path()), PASS, "a")
        .await
        .unwrap();
    let forgetful = Engine::with_key(
        Box::new(Forgetful(LocalDir::new(dir.path()))),
        &vmk,
        engine.vault_id(),
        "a",
    );
    let changed = forgetful
        .change_passphrase(&vmk, PASS, "new passphrase")
        .await;
    assert!(matches!(changed, Err(SyncError::Storage(_))), "{changed:?}");

    engine
        .change_passphrase(&vmk, PASS, "new passphrase")
        .await
        .unwrap();
    let bytes = std::fs::read(dir.path().join("manifest.json")).unwrap();
    let manifest = Manifest::from_json(&bytes).unwrap();
    assert_eq!(manifest.unlock("new passphrase").unwrap().0, vmk.0);
}

/// A storage whose answer to the next conditional write is lost: the write
/// lands, the writer hears an error (a timeout, a dropped connection).
struct LostAnswer {
    inner: LocalDir,
    lose: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl Storage for LostAnswer {
    async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
        self.inner.list(prefix).await
    }
    async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
        self.inner.put(key, data).await
    }
    async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
        self.inner.delete(key).await
    }
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> veydan_sync::Result<bool> {
        let written = self.inner.put_if_absent(key, data).await?;
        if self.lose.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(SyncError::Storage("the answer was lost".into()));
        }
        Ok(written)
    }
}

/// The write went through, the answer did not: the state stays where it
/// was, the storage holds the chunk, and a push of the same ops from the
/// same state takes it as its own (what the host's kept chunk replays).
#[tokio::test]
async fn a_push_whose_answer_was_lost_is_in_the_storage_and_its_replay_adopts_it() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, vmk) = vault(dir.path(), &["a"]).await;
    let lossy = Engine::with_key(
        Box::new(LostAnswer {
            inner: LocalDir::new(dir.path()),
            lose: true.into(),
        }),
        &vmk,
        engines[0].vault_id(),
        "a",
    );
    let mut state = LocalState::default();
    let ops = vec![op("n1", 100, "a", "one")];
    lossy.push(&mut state, ops.clone()).await.unwrap_err();
    assert_eq!(state.own_seq, 0, "no answer, no move");

    let stored = lossy.own_log_status(0, "").await.unwrap();
    assert_eq!(stored.seq, 1);
    assert_eq!(stored.prev_hash.as_deref(), Some(""));
    assert_eq!(stored.newest.as_ref().map(|h| h.wall_ms), Some(100));

    lossy.push(&mut state, ops).await.unwrap();
    assert_eq!(
        (state.own_seq, &state.own_head_hash),
        (1, &stored.head_hash)
    );
    // Other ops in the same slot are another writer's.
    let err = lossy
        .push(&mut LocalState::default(), vec![op("n1", 101, "a", "one")])
        .await
        .unwrap_err();
    assert!(matches!(err, SyncError::OwnLogCollision(1)), "{err}");

    let mut sb = LocalState::default();
    let b = Engine::with_key(storage(dir.path()), &vmk, engines[0].vault_id(), "b");
    assert_eq!(pull_ops(&b, &mut sb).await.len(), 1);
}

/// After a compaction the storage says how far the log goes even with no
/// chunk left: a state below it (a restored data file) is behind, and the
/// chunk at the tip is not known.
#[tokio::test]
async fn the_status_of_a_compacted_log_is_its_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a"]).await;
    let a = &engines[0];
    let mut state = LocalState::default();
    for (i, wall) in [100, 200, 300].into_iter().enumerate() {
        a.push(&mut state, vec![op(&format!("n{i}"), wall, "a", "v")])
            .await
            .unwrap();
    }
    let before = a.own_log_status(0, "").await.unwrap();
    assert_eq!(before.seq, 3);
    assert!(before.prev_hash.is_some());
    // From the saved position (and the one before it, with a kept chunk)
    // the status is read from one or two chunks, and is the same.
    let at_tip = a.own_log_status(3, &state.own_head_hash).await.unwrap();
    assert_eq!(at_tip, before);
    let prev = before.prev_hash.clone().unwrap();
    assert_eq!(a.own_log_status(2, &prev).await.unwrap(), before);
    a.compact(&state).await.unwrap();
    assert_eq!(a.own_chunk_count().await.unwrap(), 0);

    let stored = a.own_log_status(3, &state.own_head_hash).await.unwrap();
    assert_eq!(stored.seq, 3);
    assert_eq!(stored.head_hash, state.own_head_hash);
    assert_eq!(stored.prev_hash, None);
    assert_eq!(stored.newest.as_ref().map(|h| h.wall_ms), Some(300));

    // The next chunk continues the head the snapshot carries.
    a.push(&mut state, vec![op("n9", 400, "a", "after")])
        .await
        .unwrap();
    let stored = a.own_log_status(3, &before.head_hash).await.unwrap();
    assert_eq!(stored.seq, 4);
    assert_eq!(stored.prev_hash.as_deref(), Some(before.head_hash.as_str()));
    assert_eq!(stored.newest.as_ref().map(|h| h.wall_ms), Some(400));
}

/// At the same seq the storage may hold another hash. When its chunk
/// continues the head the device saved, the device wrote it and saved only
/// its seq (a save before it was one transaction); otherwise the chunk is
/// another writer's.
#[tokio::test]
async fn a_chunk_at_the_saved_seq_tells_whose_it_is_by_its_prev_hash() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a", "a"]).await;
    let (a1, a2) = (&engines[0], &engines[1]);
    let mut s1 = LocalState::default();
    a1.push(&mut s1, vec![op("n1", 100, "a", "one")])
        .await
        .unwrap();
    let first = s1.own_head_hash.clone();
    a1.push(&mut s1, vec![op("n2", 200, "a", "two")])
        .await
        .unwrap();

    let stored = a2.own_log_status(2, &first).await.unwrap();
    assert_eq!(stored.seq, 2);
    assert_eq!(stored.head_hash, s1.own_head_hash);
    // The seq saved, the head not: the chunk continues it.
    assert_eq!(stored.prev_hash.as_deref(), Some(first.as_str()));
    // A copy that wrote its own second chunk elsewhere has another head.
    assert_ne!(stored.prev_hash.as_deref(), Some("another head"));
}

/// A listing that lacks the chunks of the log (a WebDAV server answering
/// 404 for the folder) shows the log shorter than it is; a GET of the saved
/// chunk by its key shows it is there.
#[tokio::test]
async fn a_chunk_a_listing_lacks_is_found_by_its_key() {
    struct NoLog(LocalDir);

    #[async_trait::async_trait]
    impl Storage for NoLog {
        async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
            if prefix.contains("/log") {
                return Ok(Vec::new());
            }
            self.0.list(prefix).await
        }
        async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
            self.0.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
            self.0.put(key, data).await
        }
        async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
            self.0.delete(key).await
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (engines, vmk) = vault(dir.path(), &["a"]).await;
    let mut state = LocalState::default();
    for wall in [100, 200] {
        engines[0]
            .push(&mut state, vec![op("n1", wall, "a", "v")])
            .await
            .unwrap();
    }
    let blind = Engine::with_key(
        Box::new(NoLog(LocalDir::new(dir.path()))),
        &vmk,
        engines[0].vault_id(),
        "a",
    );
    let stored = blind
        .own_log_status(state.own_seq, &state.own_head_hash)
        .await
        .unwrap();
    assert_eq!(stored.seq, 0);
    assert_eq!(
        blind.own_chunk_hash(2).await.unwrap().as_deref(),
        Some(state.own_head_hash.as_str())
    );
    assert_eq!(blind.own_chunk_hash(3).await.unwrap(), None);
    assert_eq!(blind.own_chunk_hash(0).await.unwrap(), None);
}

/// A join reads every log from its start, as a pull of a new device would:
/// a log that stops at a file that does not open or does not continue its
/// chain is named and taken up to that file (here: nothing before it), one
/// that is not all there yet fails the whole read.
#[tokio::test]
async fn every_log_is_read_from_its_start_and_a_broken_one_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, _) = vault(dir.path(), &["a", "b", "reader"]).await;
    let (a, b, reader) = (&engines[0], &engines[1], &engines[2]);
    let (mut sa, mut sb) = (LocalState::default(), LocalState::default());
    a.push(&mut sa, vec![op("n1", 100, "a", "one")])
        .await
        .unwrap();
    a.push(&mut sa, vec![op("n1", 300, "a", "three")])
        .await
        .unwrap();
    a.compact(&sa).await.unwrap();
    a.push(&mut sa, vec![op("n3", 400, "a", "four")])
        .await
        .unwrap();
    b.push(&mut sb, vec![op("n2", 200, "b", "two")])
        .await
        .unwrap();

    let read = reader.read_every_log().await.unwrap();
    assert!(read.broken.is_empty(), "{:?}", read.broken);
    let mut latest: Vec<(String, u64)> = read
        .ops
        .iter()
        .map(|o| (o.entity_id.clone(), o.hlc.wall_ms))
        .collect();
    latest.sort();
    assert_eq!(
        latest,
        [("n1".into(), 300), ("n2".into(), 200), ("n3".into(), 400)]
    );

    // The chunk of b no longer opens: b is named, a is read.
    let chunk = dir.path().join("devices/b/log/00000001.bin");
    let mut bytes = std::fs::read(&chunk).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    std::fs::write(&chunk, &bytes).unwrap();
    let read = reader.read_every_log().await.unwrap();
    assert_eq!(read.broken.len(), 1);
    assert_eq!(read.broken[0].0, "b");
    assert!(read.ops.iter().all(|o| o.entity_id != "n2"));
    assert!(read.ops.iter().any(|o| o.entity_id == "n3"));

    // A chunk of b listed after one that is not there yet: the read waits.
    std::fs::rename(&chunk, dir.path().join("devices/b/log/00000002.bin")).unwrap();
    let err = reader.read_every_log().await.unwrap_err();
    assert!(matches!(err, SyncError::Storage(_)), "{err}");
}

/// Audit A2-17: a chunk that is not an envelope (an empty file in its place)
/// stops the log with an error that names it, not in silence; what came
/// before it is taken and the head stands just before it. Mended, the log
/// goes on from there. A chunk whose chain breaks keeps what came before
/// it the same way.
#[tokio::test]
async fn a_broken_chunk_stops_its_log_with_an_error_and_keeps_what_came_before() {
    let dir = tempfile::tempdir().unwrap();
    let (engines, vmk) = vault(dir.path(), &["a", "b"]).await;
    let (a, b) = (&engines[0], &engines[1]);
    let (mut sa, mut sb) = (LocalState::default(), LocalState::default());
    for (id, wall) in [("n1", 100), ("n2", 200), ("n3", 300)] {
        a.push(&mut sa, vec![op(id, wall, "a", "v")]).await.unwrap();
    }
    let third = dir.path().join("devices/a/log/00000003.bin");
    let good = std::fs::read(&third).unwrap();
    std::fs::write(&third, b"").unwrap();

    let pulled = b.pull(&mut sb, |_, _, _| {}).await.unwrap();
    let ids: Vec<&str> = pulled.ops.iter().map(|o| o.entity_id.as_str()).collect();
    assert_eq!(ids, ["n1", "n2"]);
    assert_eq!(pulled.errors.len(), 1);
    assert_eq!(pulled.errors[0].0, "a");
    assert!(
        pulled.errors[0].1.contains("00000003"),
        "{:?}",
        pulled.errors
    );
    assert!(pulled.peers[0].broken);
    assert_eq!(sb.peers["a"].seq, 2);
    // A join reads it the same way: up to the broken file, which it names.
    let every = b.read_every_log().await.unwrap();
    let mut ids: Vec<&str> = every.ops.iter().map(|o| o.entity_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["n1", "n2"]);
    assert_eq!(every.broken.len(), 1);

    // The same again, without the ops already taken.
    let again = b.pull(&mut sb, |_, _, _| {}).await.unwrap();
    assert!(again.ops.is_empty());
    assert_eq!(again.errors.len(), 1);

    std::fs::write(&third, &good).unwrap();
    let ids: Vec<String> = pull_ops(b, &mut sb)
        .await
        .into_iter()
        .map(|o| o.entity_id)
        .collect();
    assert_eq!(ids, ["n3"]);
    assert_eq!(sb.peers["a"].seq, 3);

    // A chain break at the next chunk: one that opens, with the right seq,
    // but continues another chunk 4 (written by a copy of this device into
    // another folder). The chunk before it is taken.
    a.push(&mut sa, vec![op("n4", 400, "a", "v")])
        .await
        .unwrap();
    a.push(&mut sa, vec![op("n5", 500, "a", "v")])
        .await
        .unwrap();
    let other = tempfile::tempdir().unwrap();
    let copy = Engine::with_key(storage(other.path()), &vmk, a.vault_id(), "a");
    let mut sc = LocalState::default();
    for wall in [1, 2, 3, 4, 5] {
        copy.push(&mut sc, vec![op("x", wall, "a", "other")])
            .await
            .unwrap();
    }
    std::fs::copy(
        other.path().join("devices/a/log/00000005.bin"),
        dir.path().join("devices/a/log/00000005.bin"),
    )
    .unwrap();
    let pulled = b.pull(&mut sb, |_, _, _| {}).await.unwrap();
    let ids: Vec<&str> = pulled.ops.iter().map(|o| o.entity_id.as_str()).collect();
    assert_eq!(ids, ["n4"]);
    assert!(pulled.peers[0].broken);
    assert!(
        pulled.errors[0].1.contains("chain break"),
        "{:?}",
        pulled.errors
    );
    assert_eq!(sb.peers["a"].seq, 4);
}

/// Audit A2-22: a chunk of the own log that is listed and does not read
/// fails the compaction before anything is written or deleted: a snapshot
/// without its ops would stand for them.
#[tokio::test]
async fn compact_leaves_the_log_as_it_is_when_a_listed_chunk_does_not_read() {
    struct Unreadable(LocalDir, &'static str);

    #[async_trait::async_trait]
    impl Storage for Unreadable {
        async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
            self.0.list(prefix).await
        }
        async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
            if key == self.1 {
                return Ok(None);
            }
            self.0.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
            self.0.put(key, data).await
        }
        async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
            self.0.delete(key).await
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (engines, vmk) = vault(dir.path(), &["a", "b"]).await;
    let (a, b) = (&engines[0], &engines[1]);
    let (mut sa, mut sb) = (LocalState::default(), LocalState::default());
    for (id, wall) in [("n1", 100), ("n2", 200), ("n3", 300)] {
        a.push(&mut sa, vec![op(id, wall, "a", "v")]).await.unwrap();
    }
    let flaky = Engine::with_key(
        Box::new(Unreadable(
            LocalDir::new(dir.path()),
            "devices/a/log/00000002.bin",
        )),
        &vmk,
        a.vault_id(),
        "a",
    );
    let err = flaky.compact(&sa).await.unwrap_err();
    assert!(matches!(err, SyncError::Storage(_)), "{err}");
    assert_eq!(a.own_chunk_count().await.unwrap(), 3);
    assert!(!dir.path().join("devices/a/snapshot.bin").exists());

    let got = pull_ops(b, &mut sb).await;
    assert_eq!(got.len(), 3);
}

/// Audit A2-22: a chunk past the snapshot that the listing lacks fails the
/// compaction too: the snapshot would stand for its ops, and a device whose
/// head is below it would jump over it.
#[tokio::test]
async fn compact_leaves_the_log_as_it_is_when_the_listing_lacks_a_chunk() {
    struct Unlisted(LocalDir, &'static str);

    #[async_trait::async_trait]
    impl Storage for Unlisted {
        async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
            let mut keys = self.0.list(prefix).await?;
            keys.retain(|k| k != self.1);
            Ok(keys)
        }
        async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
            self.0.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
            self.0.put(key, data).await
        }
        async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
            self.0.delete(key).await
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (engines, vmk) = vault(dir.path(), &["a", "b"]).await;
    let (a, b) = (&engines[0], &engines[1]);
    let (mut sa, mut sb) = (LocalState::default(), LocalState::default());
    a.push(&mut sa, vec![op("n1", 100, "a", "v")])
        .await
        .unwrap();
    a.compact(&sa).await.unwrap();
    for (id, wall) in [("n2", 200), ("n3", 300), ("n4", 400)] {
        a.push(&mut sa, vec![op(id, wall, "a", "v")]).await.unwrap();
    }
    let blind = Engine::with_key(
        Box::new(Unlisted(
            LocalDir::new(dir.path()),
            "devices/a/log/00000003.bin",
        )),
        &vmk,
        a.vault_id(),
        "a",
    );
    let err = blind.compact(&sa).await.unwrap_err();
    assert!(matches!(err, SyncError::Storage(_)), "{err}");
    assert!(err.to_string().contains("00000003"), "{err}");
    assert_eq!(a.own_chunk_count().await.unwrap(), 3);

    let mut ids: Vec<String> = pull_ops(b, &mut sb)
        .await
        .into_iter()
        .map(|o| o.entity_id)
        .collect();
    ids.sort();
    assert_eq!(ids, ["n1", "n2", "n3", "n4"]);
}

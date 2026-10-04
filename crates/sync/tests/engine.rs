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
    a.compact(&sa, 1_000, 0).await.unwrap();
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

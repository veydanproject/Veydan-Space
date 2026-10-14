// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The sweep of large files v2 against a reference published while it runs,
//! and an upload that takes up a chunk another device's GC deletes.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use veydan_sync::large_files::{self, GcRoots};
use veydan_sync::{
    CancelFlag, Engine, LargeFileConfig, LargeFileRef, LocalDir, PathSink, PathSource, Storage, Vmk,
};

const PASS: &str = "correct horse battery staple";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const CHUNKS: &str = "large-files/v2/chunks/";
const MANIFESTS: &str = "large-files/v2/manifests/";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn upload(engine: &Engine, dir: &Path, name: &str, data: &[u8]) -> LargeFileRef {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    let source = PathSource::new(&path).unwrap();
    engine
        .large_files(LargeFileConfig::default())
        .unwrap()
        .upload(&source, &|_| {}, &CancelFlag::default())
        .await
        .unwrap()
}

async fn keys(storage: &dyn Storage, prefix: &str) -> BTreeSet<String> {
    storage.list(prefix).await.unwrap().into_iter().collect()
}

/// The keys of one uploaded file: its manifest and its chunks.
async fn objects_of(dir: &Path, before: &BTreeSet<String>) -> BTreeSet<String> {
    let s = LocalDir::new(dir);
    let mut now = keys(&s, CHUNKS).await;
    now.extend(keys(&s, MANIFESTS).await);
    now.difference(before).cloned().collect()
}

async fn all_objects(dir: &Path) -> BTreeSet<String> {
    objects_of(dir, &BTreeSet::new()).await
}

/// A folder whose `delete` tells the test which key went first.
struct Watched {
    inner: LocalDir,
    first_delete: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl Storage for Watched {
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
        self.first_delete
            .lock()
            .unwrap()
            .get_or_insert_with(|| key.to_string());
        self.inner.delete(key).await
    }
    async fn exists(&self, key: &str) -> veydan_sync::Result<bool> {
        self.inner.exists(key).await
    }
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> veydan_sync::Result<bool> {
        self.inner.put_if_absent(key, data).await
    }
}

/// Roots read again before every delete: after the first delete another
/// device has published a reference to the file that delete did not touch.
struct Republished {
    first_delete: Arc<Mutex<Option<String>>>,
    files: Vec<(LargeFileRef, BTreeSet<String>)>,
    reads: usize,
}

#[async_trait::async_trait]
impl GcRoots for Republished {
    async fn recheck(&mut self) -> veydan_sync::Result<Option<BTreeSet<String>>> {
        self.reads += 1;
        let first = self.first_delete.lock().unwrap().clone();
        let Some(first) = first else {
            return Ok(Some(BTreeSet::new()));
        };
        Ok(Some(
            self.files
                .iter()
                .filter(|(_, objects)| !objects.contains(&first))
                .map(|(r, _)| r.manifest_id.clone())
                .collect(),
        ))
    }
}

async fn vault(dir: &Path) -> (Engine, Vmk) {
    Engine::create(Box::new(LocalDir::new(dir)), PASS, "a")
        .await
        .unwrap()
}

/// Two orphans a day old are due. The first delete takes one of them; then
/// another device publishes a reference to the other, and the read before
/// the next delete finds it: that file stays whole and still downloads.
#[tokio::test]
async fn a_reference_published_during_the_sweep_keeps_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let vault_dir = dir.path().join("vault");
    let files_dir = tempfile::tempdir().unwrap();
    let (engine, vmk) = vault(&vault_dir).await;
    let x = upload(&engine, files_dir.path(), "x.bin", &vec![7u8; 4096]).await;
    let x_objects = all_objects(&vault_dir).await;
    let y = upload(&engine, files_dir.path(), "y.bin", &vec![9u8; 4096]).await;
    let y_objects = objects_of(&vault_dir, &x_objects).await;
    assert_eq!((x_objects.len(), y_objects.len()), (2, 2));

    let first_delete = Arc::new(Mutex::new(None));
    let watched = Engine::with_key(
        Box::new(Watched {
            inner: LocalDir::new(&vault_dir),
            first_delete: first_delete.clone(),
        }),
        &vmk,
        engine.vault_id(),
        "b",
    );
    // A day later than the uploads, so this process's own touches are old.
    let now = now_ms() + 2 * DAY_MS;
    let candidates: HashMap<String, i64> = all_objects(&vault_dir)
        .await
        .into_iter()
        .map(|k| (k, now - DAY_MS - 1))
        .collect();
    let mut again = Republished {
        first_delete: first_delete.clone(),
        files: vec![
            (x.clone(), x_objects.clone()),
            (y.clone(), y_objects.clone()),
        ],
        reads: 0,
    };
    let outcome = watched
        .large_files(LargeFileConfig::default())
        .unwrap()
        .gc_with_complete_root_set(&BTreeSet::new(), &candidates, now, DAY_MS, &mut again)
        .await
        .unwrap();

    let first = first_delete.lock().unwrap().clone().expect("a delete");
    let (gone, kept, kept_ref) = if x_objects.contains(&first) {
        (&x_objects, &y_objects, &y)
    } else {
        (&y_objects, &x_objects, &x)
    };
    let left = all_objects(&vault_dir).await;
    assert!(kept.is_subset(&left), "{kept:?} / {left:?}");
    assert!(gone.is_disjoint(&left), "{gone:?} / {left:?}");
    assert_eq!(
        outcome.removed.iter().cloned().collect::<BTreeSet<_>>(),
        *gone
    );
    // Before every delete; the one at the start finds nothing new.
    assert_eq!(again.reads, gone.len() + kept.len());
    for key in kept {
        assert!(outcome.forgotten.contains(key), "{key}");
    }

    let dest = files_dir.path().join("kept.bin");
    watched
        .large_files(LargeFileConfig::default())
        .unwrap()
        .download(
            kept_ref,
            &PathSink::new(&dest),
            &|_| {},
            &CancelFlag::default(),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::metadata(&dest).unwrap().len(), 4096);
}

/// Roots that never change: the sweep deletes what is due.
struct Same;

#[async_trait::async_trait]
impl GcRoots for Same {
    async fn recheck(&mut self) -> veydan_sync::Result<Option<BTreeSet<String>>> {
        Ok(None)
    }
}

/// An object this process wrote or took up within the grace is not deleted,
/// however old its candidate; its grace starts again.
#[tokio::test]
async fn an_object_touched_lately_starts_its_grace_again() {
    let dir = tempfile::tempdir().unwrap();
    let files_dir = tempfile::tempdir().unwrap();
    let (engine, _) = vault(dir.path()).await;
    upload(&engine, files_dir.path(), "z.bin", &vec![3u8; 4096]).await;
    let objects = all_objects(dir.path()).await;
    let now = now_ms();
    let candidates: HashMap<String, i64> = objects
        .iter()
        .map(|k| (k.clone(), now - 2 * DAY_MS))
        .collect();

    let outcome = engine
        .large_files(LargeFileConfig::default())
        .unwrap()
        .gc_with_complete_root_set(&BTreeSet::new(), &candidates, now, DAY_MS, &mut Same)
        .await
        .unwrap();
    assert!(outcome.removed.is_empty(), "{:?}", outcome.removed);
    assert_eq!(all_objects(dir.path()).await, objects);
    for key in &objects {
        assert!(outcome.forgotten.contains(key), "{key}");
        assert!(outcome.remembered.contains(&(key.clone(), now)), "{key}");
    }
    assert!(large_files::touched_since(
        objects.iter().next().unwrap(),
        now - DAY_MS
    ));
}

/// A folder where another device's GC deletes a chunk just after this
/// upload found it in place.
struct Vanishing {
    inner: LocalDir,
    armed: Mutex<Option<String>>,
}

#[async_trait::async_trait]
impl Storage for Vanishing {
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
    async fn exists(&self, key: &str) -> veydan_sync::Result<bool> {
        let found = self.inner.exists(key).await?;
        let armed = self.armed.lock().unwrap().take_if(|k| k == key);
        if armed.is_some() {
            self.inner.delete(key).await?;
        }
        Ok(found)
    }
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> veydan_sync::Result<bool> {
        self.inner.put_if_absent(key, data).await
    }
}

/// The file is uploaded again: its chunk is in place, so the upload sends
/// nothing, and is deleted right after. Before the manifest the upload looks
/// at the chunk again and writes it back; the file downloads whole.
#[tokio::test]
async fn a_chunk_taken_by_a_foreign_gc_during_the_upload_is_written_again() {
    let dir = tempfile::tempdir().unwrap();
    let files_dir = tempfile::tempdir().unwrap();
    let (engine, vmk) = vault(dir.path()).await;
    let data = vec![5u8; 4096];
    let first = upload(&engine, files_dir.path(), "w.bin", &data).await;
    let chunks = keys(&LocalDir::new(dir.path()), CHUNKS).await;
    let chunk = chunks.iter().next().unwrap().clone();
    assert_eq!(chunks.len(), 1);

    let vanishing = Engine::with_key(
        Box::new(Vanishing {
            inner: LocalDir::new(dir.path()),
            armed: Mutex::new(Some(chunk.clone())),
        }),
        &vmk,
        engine.vault_id(),
        "b",
    );
    let again = upload(&vanishing, files_dir.path(), "w.bin", &data).await;
    assert_eq!(again.manifest_id, first.manifest_id);
    assert!(LocalDir::new(dir.path()).exists(&chunk).await.unwrap());

    let dest = files_dir.path().join("w.out");
    engine
        .large_files(LargeFileConfig::default())
        .unwrap()
        .download(
            &again,
            &PathSink::new(&dest),
            &|_| {},
            &CancelFlag::default(),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

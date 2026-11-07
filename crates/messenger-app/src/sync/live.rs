// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The archive and the runtime of the messenger that runs, as the handler
//! sees them (stage Э5): `messenger_archive::Archive` over the runtime's
//! store is the [`ArchiveStore`] of the handler, call for call, and
//! [`LiveRuntime`] is the running `MessengerRuntime` together with that
//! archive, which it installs as the runtime's hooks and takes away again.
//! The fakes of the tests stand in for both.

use super::{ArchiveRuntime, ArchiveStore, Built, Enqueued, Merged, QueueItem, QueueProgress, ReplayReport};
use async_trait::async_trait;
use messenger_archive::{Archive, ArchiveScope, Replayer, SegmentId, SegmentMeta};
use messenger_core::{MessengerError, PubKey, Result, Timestamp};
use messenger_runtime::archive::ArchiveHooks;
use messenger_runtime::MessengerRuntime;
use std::sync::Arc;
use tauri::AppHandle;
use veydan_core::BoxFuture;

#[async_trait]
impl ArchiveStore for Archive {
    async fn owner(&self) -> Result<Option<PubKey>> {
        Archive::owner(self).await
    }
    async fn due(&self, now: Timestamp, force: bool) -> Result<Vec<SegmentId>> {
        Archive::due(self, now, force).await
    }
    async fn build(&self, seg: &SegmentId) -> Result<Built> {
        Archive::build(self, seg).await
    }
    async fn compact(&self, seg: &SegmentId) -> Result<u64> {
        Archive::compact(self, seg).await
    }
    async fn merge(&self, seg: &SegmentId, remote: &[u8], replayer: &dyn Replayer) -> Result<Merged> {
        Archive::merge(self, seg, remote, replayer).await
    }
    async fn merge_held(&self, seg: &SegmentId, hash: &str, replayer: &dyn Replayer) -> Result<Option<Merged>> {
        Archive::merge_held(self, seg, hash, replayer).await
    }
    async fn holds(&self, seg: &SegmentId, hash: &str) -> Result<bool> {
        Archive::holds(self, seg, hash).await
    }
    async fn written(&self, seg: &SegmentId, meta: &SegmentMeta) -> Result<()> {
        Archive::written(self, seg, meta).await
    }
    async fn defer(&self, seg: &SegmentId) -> Result<()> {
        Archive::defer(self, seg).await
    }
    async fn forget_written(&self) -> Result<()> {
        Archive::forget_written(self).await
    }
    async fn high_water(&self, scope: &ArchiveScope) -> Result<Option<Timestamp>> {
        Archive::high_water(self, scope).await
    }
    async fn enqueue(&self, entity_id: &str, meta: &SegmentMeta) -> Result<Enqueued> {
        Archive::enqueue(self, entity_id, meta).await
    }
    async fn dequeue(&self, open: Option<&ArchiveScope>) -> Result<Option<QueueItem>> {
        Archive::dequeue(self, open).await
    }
    async fn done(&self, entity_id: &str, hash: &str) -> Result<()> {
        Archive::done(self, entity_id, hash).await
    }
    async fn failed(&self, entity_id: &str, hash: &str, error: &str) -> Result<()> {
        Archive::failed(self, entity_id, hash, error).await
    }
    async fn park(&self, entity_id: &str, hash: &str, why: &str) -> Result<()> {
        Archive::park(self, entity_id, hash, why).await
    }
    async fn refuse(&self, entity_id: &str, hash: &str, error: &str, build: &str) -> Result<()> {
        Archive::refuse(self, entity_id, hash, error, build).await
    }
    async fn retry_failed(&self) -> Result<u64> {
        Archive::retry_failed(self).await
    }
    async fn retry_parked(&self) -> Result<u64> {
        Archive::retry_parked(self).await
    }
    async fn retry_refused(&self, build: &str) -> Result<u64> {
        Archive::retry_refused(self, build).await
    }
    async fn blocked(&self) -> Result<Vec<String>> {
        Archive::blocked(self).await
    }
    async fn restoring_scopes(&self) -> Result<Vec<String>> {
        Archive::restoring_scopes(self).await
    }
    async fn refused_too_large(&self) -> Result<Vec<String>> {
        Archive::refused_too_large(self).await
    }
    async fn unwritable(&self) -> Result<Vec<String>> {
        Archive::unwritable(self).await
    }
    async fn clear_done(&self) -> Result<()> {
        Archive::clear_done(self).await
    }
    async fn progress(&self) -> Result<QueueProgress> {
        Archive::progress(self).await
    }
    async fn replay_segment(&self, item: &QueueItem, bytes: &[u8], replayer: &dyn Replayer) -> Result<ReplayReport> {
        Archive::replay_segment(self, item, bytes, replayer).await
    }
    async fn replay_held_of(&self, item: &QueueItem, replayer: &dyn Replayer) -> Result<Option<ReplayReport>> {
        Archive::replay_held_of(self, item, replayer).await
    }
    async fn remove_scope(&self, scope: &ArchiveScope) -> Result<Vec<String>> {
        Archive::remove_scope(self, scope).await
    }
    async fn has_server(&self, id: &str) -> Result<bool> {
        Archive::has_server(self, id).await
    }
    async fn take_forgotten_scopes(&self) -> Result<Vec<ArchiveScope>> {
        Archive::take_forgotten_scopes(self).await
    }
    async fn group_ids(&self) -> Result<Vec<String>> {
        Archive::group_ids(self).await
    }
}

/// The running runtime and the archive it records into.
pub(crate) struct LiveRuntime {
    rt: Arc<MessengerRuntime>,
    archive: Arc<Archive>,
    app: AppHandle,
}

impl LiveRuntime {
    pub(crate) fn new(rt: Arc<MessengerRuntime>, archive: Arc<Archive>, app: AppHandle) -> Self {
        Self { rt, archive, app }
    }
}

#[async_trait]
impl ArchiveRuntime for LiveRuntime {
    fn replayer(&self) -> Arc<dyn Replayer> {
        self.rt.replayer()
    }

    /// The archive goes into the runtime as its sink and marks, with the
    /// handler as who to wake when a group or a server comes, or comes out
    /// of it.
    fn set_archive(&self, installed: bool) {
        let hooks: Option<Arc<dyn ArchiveHooks>> = installed.then(|| self.archive.clone() as Arc<dyn ArchiveHooks>);
        self.rt.set_archive(hooks);
        let waker: Option<messenger_runtime::archive::Waker> = installed.then(|| {
            let app = self.app.clone();
            Arc::new(move || super::woke(&app)) as messenger_runtime::archive::Waker
        });
        self.rt.set_archive_waker(waker);
    }

    /// The runtime walks its past in the background; the handler hears how
    /// it went and asks again after a walk that broke off.
    fn archive_backfill(&self) -> Result<BoxFuture<'static, Result<u64>>> {
        let handle = self.rt.archive_backfill()?;
        Ok(Box::pin(async move {
            match handle.await {
                Ok(result) => result,
                Err(e) => Err(MessengerError::Other(format!("backfill: {e}"))),
            }
        }))
    }

    fn archive_restored(&self) {
        self.rt.archive_restored();
    }

    async fn group_secret_arrived(&self, name: &str) -> Result<()> {
        self.rt.group_secret_arrived(name).await
    }
}

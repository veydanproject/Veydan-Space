// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A file of mine on its way into a channel as the messenger's transfer
// line shows it (TransferStatus, through the seams): the progress the
// runtime tells (`channel.media`), or, before it moves (a row read after
// a start), one made of the row; and what each action of the line does.

import type { TransferAction, TransferProgress } from '$lib/messenger/seams/shared';
import type { UploadProgress } from './channelStore.svelte';

/** The progress of a file of mine: as told, else as its row says it. */
export function uploadProgress(u: UploadProgress): TransferProgress {
  if (u.progress) return u.progress;
  return {
    transfer_id: u.id,
    message_id: u.id,
    chat_id: u.communityId,
    direction: 'up',
    // A row `uploading` with no word yet waits for its run.
    status: u.status === 'paused' || u.status === 'failed' ? u.status : 'queued',
    done_bytes: u.done,
    total_bytes: u.total,
    failure_reason: null,
    local_path: null,
    stage: 'queued',
    chunks_done: 0,
    chunks_total: 0,
    chunk_size: 0,
    rate_bps: 0,
    eta_secs: null,
    retry_at_ms: null,
    attempt: 0,
    file_name: u.name,
    mime: '',
  };
}

/**
 * What the line of a file of mine calls it: a recording by its kind, as a chat says it
 * (`title`: the messenger's `mediaTitle`), a file by its name, one not read yet by `fallback`.
 */
export function uploadLabel(u: Pick<UploadProgress, 'name' | 'kind'>, title: (kind: string, name: string) => string, fallback: string): string {
  if (u.kind === 'voice' || u.kind === 'circle') return title(u.kind, u.name);
  return u.name || fallback;
}

/** The files of mine on their way under one post (its comments), or as posts (`postId` empty). */
export function uploadsUnder<T extends Pick<UploadProgress, 'communityId' | 'postId'>>(all: T[], communityId: string, postId: string): T[] {
  return all.filter((u) => u.communityId === communityId && (u.postId ?? '') === postId);
}

/** What the store does of each action, for one file of mine. */
export interface UploadActions {
  pauseUpload(id: string): Promise<unknown>;
  wakeUpload(id: string): Promise<unknown>;
  retryUpload(id: string): Promise<unknown>;
  cancelUpload(id: string): Promise<unknown>;
}

/** An action of the line on a file of mine: a resume and a retry both go on from what the hosts hold. */
export function uploadAction(store: UploadActions, id: string, a: TransferAction): Promise<unknown> {
  switch (a) {
    case 'pause':
      return store.pauseUpload(id);
    case 'retry_now':
      return store.wakeUpload(id);
    case 'cancel':
      return store.cancelUpload(id);
    default:
      return store.retryUpload(id);
  }
}

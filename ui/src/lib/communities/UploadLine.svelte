<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A file of mine on its way into a channel: the line of a chat's transfer
  (stage, speed, time left, pause, resume, retry, cancel), its name (a
  voice message or a circle by its kind), and why it did not go. At the
  foot of the feed for posts, under the post for its comments.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { TransferStatus, mediaTitle } from '$lib/messenger/seams/shared';
  import { channelStore, type UploadProgress } from './channelStore.svelte';
  import { channelError } from './errors';
  import { uploadAction, uploadLabel, uploadProgress } from './uploads';

  let { u }: { u: UploadProgress } = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const failed = $derived(u.status === 'failed');
  const label = $derived(uploadLabel(u, (kind, name) => mediaTitle(kind, name, $t), $t('chan_uploading')));

  /** Why a file did not go, in words: a refusal of the module, or a send a closed app broke off. */
  function why(e: string | undefined): string {
    if (!e) return '';
    if (e.includes('err.interrupted')) return tr('chan_upload_interrupted');
    return channelError(e, tr);
  }
</script>

<div class="upload" class:failed role="status">
  <Icon name={failed ? 'alert-triangle' : u.kind === 'voice' ? 'mic' : u.kind === 'circle' ? 'circle-video' : 'upload'} size={14} />
  <div class="up-body">
    <span class="up-name">{label}</span>
    <TransferStatus p={uploadProgress(u)} look="full" name={label} size={u.total} act={(a) => uploadAction(channelStore, u.id, a)} />
    {#if failed && u.error}<span class="up-why">{$t('chan_upload_failed', { error: why(u.error) })}</span>{/if}
  </div>
</div>

<style>
  .upload {
    display: flex; align-items: center; gap: var(--sp-2); width: min(680px, 100%); align-self: center; padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-lg); background: var(--surface-2); border: 1px dashed var(--border); color: var(--text-2); font-size: var(--fs-xs);
  }
  .upload.failed { border-color: var(--danger-border); color: var(--danger-text); }
  .up-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .up-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-body); font-weight: var(--fw-semibold); }
  .up-why { color: var(--danger-text); line-height: 1.4; }
</style>

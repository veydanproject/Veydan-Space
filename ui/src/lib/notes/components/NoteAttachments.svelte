<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { NoteAttachment } from '$lib/notes/types';
  import { api, downloadNoteAttachment, type AttachmentTransfer } from '$lib/notes/api';
  import { transferKey, transferPercent } from '$lib/notes/attachmentTransfer';
  import Icon from '$lib/core/Icon.svelte';
  import { locale, t } from '$lib/core/i18n';
  import { mediaKindOf, mediaMimeOf } from '$lib/notes/media/kind';
  import { mediaT } from '$lib/notes/media/strings';
  import type { MediaKind } from '$lib/notes/media/types';

  interface Props {
    noteId: string;
    /** Absolute dir of the note file; media players load `rel_path` against it. */
    baseDir: string;
    attachments: NoteAttachment[];
    readonly?: boolean;
    /** Vault uploads/downloads in flight for this note, keyed by `note_id/name`. */
    transfers?: Map<string, AttachmentTransfer>;
    error?: string | null;
    oninsert: (a: NoteAttachment) => void;
    onchanged: () => void;
    /** Opens the native file picker. */
    onpick: () => void;
    /** Opens the audio / video recorder. */
    onrecord: (kind: MediaKind) => void;
    /** Downloads a vault-only attachment. */
    onfetch: (a: NoteAttachment) => void;
  }

  let {
    noteId, baseDir, attachments, readonly = false, transfers = new Map(), error = null,
    oninsert, onchanged, onpick, onrecord, onfetch,
  }: Props = $props();

  let gcResult = $state<string | null>(null);
  let gcBusy = $state(false);
  const mt = $derived(mediaT($locale));
  /** Attachment whose inline player is open, and its blob URL. */
  let playing = $state<string | null>(null);
  let playSrc = $state<string | null>(null);

  function stopPlay() {
    if (playSrc) URL.revokeObjectURL(playSrc);
    playSrc = null;
    playing = null;
  }

  // Close the player when the note changes or the file disappears.
  $effect(() => {
    if (playing && !attachments.some((a) => a.name === playing)) stopPlay();
  });

  function iconOf(a: NoteAttachment): string {
    if (a.is_image) return 'image';
    const kind = mediaKindOf(a.name);
    return kind === 'audio' ? 'mic' : kind === 'video' ? 'video' : 'file-text';
  }

  /**
   * Bytes for the player. WebKitGTK's media stack fetches `asset.localhost`
   * outside the webview and fails, so the file is replayed from a blob.
   */
  async function mediaBytes(a: NoteAttachment): Promise<Uint8Array> {
    try {
      const res = await fetch(convertFileSrc(`${baseDir}/${a.rel_path}`));
      if (res.ok) return new Uint8Array(await res.arrayBuffer());
    } catch {
      // Asset URL blocked; the IPC read below covers it.
    }
    return api.notes.attachmentRead(noteId, a.name);
  }

  /** Media plays inline; everything else opens in the system app. */
  async function open(a: NoteAttachment) {
    if (!a.present) return onfetch(a);
    if (!mediaKindOf(a.name)) return api.notes.attachmentOpen(noteId, a.name);
    if (playing === a.name) return stopPlay();
    stopPlay();
    playing = a.name;
    const bytes = await mediaBytes(a);
    const mime = mediaMimeOf(a.name) ?? 'application/octet-stream';
    const copy = new ArrayBuffer(bytes.byteLength);
    new Uint8Array(copy).set(bytes);
    const url = URL.createObjectURL(new Blob([copy], { type: mime }));
    if (playing !== a.name) return URL.revokeObjectURL(url);
    playSrc = url;
  }

  const transferOf = (name: string) => transfers.get(transferKey(noteId, name));
  /** Transfers of files not on disk yet (remote downloads in progress). */
  const pendingDownloads = $derived([...transfers.values()].filter((x) => !attachments.some((a) => a.name === x.name)));

  function transferLabel(x: AttachmentTransfer): string {
    if (x.error) return $t('note_att_transfer_failed', { error: x.error });
    const pct = transferPercent(x);
    return $t(x.direction === 'up' ? 'note_att_uploading' : 'note_att_downloading', { pct: pct === null ? '…' : `${pct}%` });
  }

  async function cancel(name: string) {
    await api.sync.attachmentCancel(noteId, name);
  }

  function fmtSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  async function remove(a: NoteAttachment) {
    await api.notes.attachmentDelete(noteId, a.name);
    onchanged();
  }

  /** Remove attachments no note references (across all notes). */
  async function gc() {
    gcBusy = true;
    try {
      const orphans = await api.notes.attachmentsGc(false);
      if (orphans.length === 0) {
        gcResult = $t('note_att_gc_done', { n: '0' });
        return;
      }
      const names = orphans.map((o) => o.name).join('\n');
      if (!(await ask({ title: $t('note_att_gc'), message: names }))) return;
      const removed = await api.notes.attachmentsGc(true);
      gcResult = $t('note_att_gc_done', { n: String(removed.length) });
      onchanged();
    } finally {
      gcBusy = false;
    }
  }

</script>

<div class="att-panel">
  <div class="att-header">
    <span class="att-title"><Icon name="paperclip" size={12} /> {$t('note_att_title')} ({attachments.length})</span>
    <div class="att-actions">
      {#if !readonly}
        <button class="att-btn" title={$t('note_att_add')} onclick={onpick}>
          <Icon name="plus" size={12} />
        </button>
        <button class="att-btn" title={mt('record')} onclick={() => onrecord('audio')}>
          <Icon name="mic" size={12} />
        </button>
        <button class="att-btn" title={mt('camera')} onclick={() => onrecord('video')}>
          <Icon name="video" size={12} />
        </button>
      {/if}
      <button class="att-btn" title={$t('note_att_gc')} onclick={gc} disabled={gcBusy}>
        <Icon name="trash-2" size={12} />
      </button>
    </div>
  </div>
  {#if gcResult}
    <p class="att-hint">{gcResult}</p>
  {/if}
  {#if error}
    <p class="att-hint att-error">{error}</p>
  {/if}
  {#if attachments.length === 0 && pendingDownloads.length === 0}
    <p class="att-hint">{$t('note_att_empty')}</p>
  {:else}
    <ul class="att-list">
      {#each attachments as a (a.name)}
        {@const x = transferOf(a.name)}
        {@const kind = mediaKindOf(a.name)}
        <li class="att-item" class:att-busy={(x && !x.error) || !a.present}>
          <Icon name={iconOf(a)} size={13} />
          <button class="att-name" title={a.name} onclick={() => open(a)}>{a.name}</button>
          {#if kind && a.present}
            <button class="att-btn" title={mt(playing === a.name ? 'stop_play' : 'play')} onclick={() => open(a)}>
              <Icon name={playing === a.name ? 'square' : 'play'} size={12} />
            </button>
          {/if}
          {#if x}
            <span class="att-size" class:att-error={!!x.error} title={x.error ?? ''}>{transferLabel(x)}</span>
            {#if !x.finished}
              <button class="att-btn" title={$t('note_att_cancel')} onclick={() => cancel(a.name)}><Icon name="x" size={12} /></button>
            {/if}
          {:else if !a.present}
            <span class="att-size">{$t('note_att_not_downloaded', { size: fmtSize(a.size) })}</span>
          {:else}
            <span class="att-size">{fmtSize(a.size)}</span>
          {/if}
          {#if !a.present}
            {#if !x || x.finished}
              <button class="att-btn" title={$t('note_att_fetch')} onclick={() => onfetch(a)}>
                <Icon name="download" size={12} />
              </button>
            {/if}
          {:else}
            <button class="att-btn" title={$t('note_att_download')} onclick={() => downloadNoteAttachment(noteId, a.name)}>
              <Icon name="download" size={12} />
            </button>
          {/if}
          {#if !readonly}
            <button class="att-btn" title={$t('note_att_insert')} onclick={() => oninsert(a)}><Icon name="link" size={12} /></button>
            <button class="att-btn att-danger" title={$t('note_att_delete')} onclick={() => remove(a)}><Icon name="x" size={12} /></button>
          {/if}
        </li>
        {#if kind && playing === a.name && playSrc}
          <li class="att-player">
            {#if kind === 'audio'}
              <audio controls autoplay src={playSrc} onended={stopPlay}></audio>
            {:else}
              <!-- svelte-ignore a11y_media_has_caption -->
              <video controls autoplay src={playSrc} onended={stopPlay}></video>
            {/if}
          </li>
        {/if}
      {/each}
      {#each pendingDownloads as x (x.name)}
        <li class="att-item att-busy">
          <Icon name="download" size={13} />
          <span class="att-name" title={x.name}>{x.name}</span>
          <span class="att-size" class:att-error={!!x.error} title={x.error ?? ''}>{transferLabel(x)}</span>
          {#if !x.finished}
            <button class="att-btn" title={$t('note_att_cancel')} onclick={() => cancel(x.name)}><Icon name="x" size={12} /></button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .att-panel {
    border-top: 1px solid var(--border);
    background: var(--surface);
    padding: var(--sp-2) var(--sp-3);
    max-height: min(45%, 180px);
    overflow-y: auto;
    flex-shrink: 0;
  }
  .att-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--sp-1);
  }
  .att-title {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--fs-2xs);
    font-weight: var(--fw-semibold);
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .att-actions { display: flex; gap: 0.2rem; }
  .att-btn {
    background: none;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    border-radius: 4px;
    padding: 0.15rem 0.3rem;
    display: inline-flex;
    align-items: center;
  }
  .att-btn:hover:not(:disabled) { background: var(--surface-2); color: var(--text); }
  .att-btn.att-danger:hover { color: var(--danger); }
  .att-btn:disabled { opacity: 0.4; }
  .att-hint { margin: 0.2rem 0; font-size: var(--fs-xs); color: var(--text-3); }
  .att-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.15rem; }
  .att-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--fs-xs);
    color: var(--text-2);
    padding: 0.15rem 0.2rem;
    border-radius: 4px;
  }
  .att-item:hover { background: var(--surface-2); }
  .att-name {
    flex: 1;
    min-width: 0;
    text-align: left;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding: 0;
  }
  .att-name:hover { text-decoration: underline; }
  .att-size { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .att-player { padding: 0.2rem 0.2rem 0.4rem; }
  .att-player audio { width: 100%; height: 32px; }
  .att-player video { width: 100%; max-height: 150px; border-radius: 6px; background: #000; }
  .att-busy { opacity: 0.8; }
  .att-error { color: var(--danger); }
</style>

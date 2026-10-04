<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { locale, t } from '$lib/core/mobile/i18n';
  import { fmtDateTimeLong, fmtSize } from '$lib/notes/mobile/notes-editor';
  import type { NoteAttachment } from '$lib/notes/mobile/api';
  import type { AttachmentTransfer } from '$lib/notes/api';
  import { transferPercent } from '$lib/notes/attachmentTransfer';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    attachments: NoteAttachment[];
    thumbs: Record<string, string>;
    createdAt: string;
    updatedAt: string;
    folderLabel: string;
    tags: string[];
    /** Vault uploads/downloads in flight, keyed by `note_id/name`. */
    transfers?: Map<string, AttachmentTransfer>;
    onclose: () => void;
    onfile: () => void;
    onimage: () => void;
    oninsert: (a: NoteAttachment) => void;
    onremove: (a: NoteAttachment) => void;
    oncancel: (name: string) => void;
    /** Downloads a vault-only attachment. */
    onfetch: (a: NoteAttachment) => void;
    /** System save dialog. */
    onsave: (a: NoteAttachment) => void;
  }

  let {
    open, attachments, thumbs, createdAt, updatedAt, folderLabel, tags, transfers = new Map(),
    onclose, onfile, onimage, oninsert, onremove, oncancel, onfetch, onsave,
  }: Props = $props();

  /** Attachment whose action menu is open. */
  let menu = $state<NoteAttachment | null>(null);

  const transferOf = (name: string) => [...transfers.values()].find((x) => x.name === name);
  const pendingDownloads = $derived([...transfers.values()].filter((x) => !attachments.some((a) => a.name === x.name)));

  /** remote: only in the vault; busy: transfer running; error: last transfer failed; local: on disk. */
  type Status = 'remote' | 'busy' | 'error' | 'local';

  function statusOf(a: NoteAttachment, x: AttachmentTransfer | undefined): Status {
    if (x && !x.finished) return 'busy';
    if (x?.error) return 'error';
    return a.present ? 'local' : 'remote';
  }

  function statusLabel(a: NoteAttachment, x: AttachmentTransfer | undefined): string {
    switch (statusOf(a, x)) {
      case 'busy':
      case 'error':
        return transferLabel(x!);
      case 'remote':
        return $t('note_att_not_downloaded', { size: fmtSize(a.size) });
      default:
        return $t('notes_attach_downloaded', { size: fmtSize(a.size) });
    }
  }

  function transferLabel(x: AttachmentTransfer): string {
    if (x.error) return $t('note_att_transfer_failed', { error: x.error });
    const pct = transferPercent(x);
    return $t(x.direction === 'up' ? 'note_att_uploading' : 'note_att_downloading', { pct: pct === null ? '…' : `${pct}%` });
  }

  function act(fn: (a: NoteAttachment) => void) {
    const a = menu;
    menu = null;
    if (a) fn(a);
  }
</script>

<BottomSheet {open} title={$t('notes_attachments')} {onclose}>
  <div class="actions">
    <button type="button" class="act" onclick={onimage}>
      <span class="m-doc lg"><Icon name="image" size={20} /></span>
      {$t('notes_attach_image')}
    </button>
    <button type="button" class="act" onclick={onfile}>
      <span class="m-doc lg"><Icon name="file" size={20} /></span>
      {$t('notes_attach_file')}
    </button>
  </div>

  {#if attachments.length || pendingDownloads.length}
    <h3>{$t('notes_attachments_count', { n: String(attachments.length) })}</h3>
    <div class="list">
      {#each attachments as a (a.name)}
        {@const x = transferOf(a.name)}
        {@const st = statusOf(a, x)}
        <div class="row {st}">
          <button type="button" class="open" onclick={() => (a.present ? oninsert(a) : onfetch(a))} aria-label={a.present ? $t('notes_insert') : $t('note_att_fetch')}>
            {#if a.is_image && thumbs[a.name]}
              <img src={thumbs[a.name]} alt="" />
            {:else}
              <span class="m-doc badge"><Icon name={a.present ? 'file' : 'download'} size={16} /></span>
            {/if}
            <span class="meta">
              <span class="name">{a.name}</span>
              <span class="size"><span class="dot"></span>{statusLabel(a, x)}</span>
            </span>
          </button>
          {#if st === 'busy'}
            <button type="button" class="x" onclick={() => oncancel(a.name)} aria-label={$t('note_att_cancel')}>
              <Icon name="x" size={14} />
            </button>
          {:else if !a.present}
            <button type="button" class="x primary" onclick={() => onfetch(a)} aria-label={$t('note_att_fetch')}>
              <Icon name="download" size={14} />
            </button>
          {:else}
            <button type="button" class="x" onclick={() => (menu = a)} aria-label={$t('notes_attach_actions')}>
              <Icon name="more-horizontal" size={14} />
            </button>
          {/if}
        </div>
      {/each}
      {#each pendingDownloads as x (x.name)}
        <div class="row">
          <span class="open">
            <span class="m-doc"><Icon name="download" size={16} /></span>
            <span class="meta">
              <span class="name">{x.name}</span>
              <span class="size" class:err={!!x.error}>{transferLabel(x)}</span>
            </span>
          </span>
          {#if !x.finished}
            <button type="button" class="x" onclick={() => oncancel(x.name)} aria-label={$t('note_att_cancel')}>
              <Icon name="x" size={14} />
            </button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <dl class="m-meta">
    <dt>{$t('notes_meta_created')}</dt><dd>{createdAt ? fmtDateTimeLong(createdAt, $locale) : '—'}</dd>
    <dt>{$t('notes_meta_updated')}</dt><dd>{updatedAt ? fmtDateTimeLong(updatedAt, $locale) : '—'}</dd>
    <dt>{$t('notes_meta_folder')}</dt><dd>{folderLabel || '—'}</dd>
    <dt>{$t('notes_meta_tags')}</dt>
    <dd>
      {#if tags.length}
        <span class="m-chips">{#each tags as name (name)}<span class="m-chip small">#{name}</span>{/each}</span>
      {:else}—{/if}
    </dd>
  </dl>
</BottomSheet>

<BottomSheet open={menu !== null} title={menu?.name ?? ''} onclose={() => (menu = null)}>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => act(oninsert)}>
      <Icon name="link" size={20} /><span class="m-row-label">{$t('notes_insert')}</span>
    </button>
    <button type="button" class="m-row" onclick={() => act(onsave)}>
      <Icon name="save" size={20} /><span class="m-row-label">{$t('notes_attach_save')}</span>
    </button>
  </div>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => act(onremove)}>
      <Icon name="trash-2" size={20} /><span class="m-row-label danger">{$t('notes_attach_delete')}</span>
    </button>
  </div>
</BottomSheet>

<style>
  .actions { display: flex; gap: var(--sp-2); }
  .act {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    min-height: 88px;
    padding: var(--sp-3);
    border: 0;
    border-radius: var(--m-radius);
    background: var(--m-field);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
  }
  h3 { margin: var(--sp-2) 0 0; font-size: 15px; font-weight: 700; }
  .list { display: flex; flex-direction: column; gap: 8px; }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 4px 4px 8px;
    border-radius: var(--m-radius);
    background: var(--m-card);
    border: 1px solid var(--m-card-border);
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 0;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
  }
  .open img { width: 36px; height: 36px; object-fit: cover; border-radius: 8px; flex-shrink: 0; }
  .meta { display: flex; flex-direction: column; min-width: 0; gap: 2px; }
  .name { font-size: 13px; font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .size { display: inline-flex; align-items: center; gap: 6px; font-size: 11px; color: var(--text-3); }
  .size.err { color: var(--danger); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--success-text); flex-shrink: 0; }
  .row.remote .dot { background: var(--text-3); }
  .row.remote .badge { border: 1px dashed var(--m-card-border); background: transparent; color: var(--text-3); }
  .row.remote .name { color: var(--text-2); }
  .row.busy .dot { background: var(--accent); }
  .row.busy .size { color: var(--accent); }
  .row.error .dot { background: var(--danger-text); }
  .row.error .size { color: var(--danger-text); }
  .x.primary { background: var(--accent); color: #fff; }
  .danger { color: var(--danger-text); }
  .x {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    margin-right: 4px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--m-seg);
    color: var(--text-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
</style>

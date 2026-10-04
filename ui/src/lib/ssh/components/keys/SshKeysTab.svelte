<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/ssh/api';
  import type { SshKey } from '$lib/ssh/types';
  import Icon from '$lib/core/Icon.svelte';
  import Modal from '$lib/core/ui/Modal.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import SshKeyForm from './SshKeyForm.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { formatDate, formatError } from '$lib/core/utils';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';

  let keys = $state<SshKey[]>([]);
  let loading = $state(false);
  let error = $state('');
  let search = $state('');

  // form drawer: undefined = closed
  let formMode = $state<'import' | 'generate' | undefined>(undefined);
  // view/rename drawer
  let viewKey = $state<SshKey | null>(null);
  let revealPrivate = $state(false);
  // Fetched from the backend only when the user reveals or copies it.
  let privateKeyText = $state('');
  let editName = $state('');
  let editComment = $state('');
  let savingEdit = $state(false);
  let deleteModal = $state({ open: false, id: '', name: '', usage: 0 });
  let copiedId = $state('');
  let copyTimer: ReturnType<typeof setTimeout>;

  onMount(load);

  async function load() {
    loading = true;
    try {
      keys = await api.sshKeys.list();
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  let filtered = $derived(keys.filter((k) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return (
      k.name.toLowerCase().includes(q) ||
      k.algorithm.toLowerCase().includes(q) ||
      (k.comment ?? '').toLowerCase().includes(q) ||
      (k.fingerprint ?? '').toLowerCase().includes(q)
    );
  }));

  function algoLabel(k: SshKey): string {
    return k.bits ? `${k.algorithm} ${k.bits}` : k.algorithm;
  }

  async function copyPublic(k: SshKey) {
    try {
      await navigator.clipboard.writeText(k.public_key);
      copiedId = k.id;
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copiedId = ''), 2000);
    } catch {}
  }

  function openView(k: SshKey) {
    viewKey = k;
    revealPrivate = false;
    privateKeyText = '';
    editName = k.name;
    editComment = k.comment ?? '';
  }

  async function loadPrivate(): Promise<string> {
    if (!viewKey) return '';
    if (!privateKeyText) privateKeyText = await api.sshKeys.exportPrivate(viewKey.id);
    return privateKeyText;
  }

  async function toggleReveal() {
    try {
      if (!revealPrivate) await loadPrivate();
      revealPrivate = !revealPrivate;
    } catch (e) {
      error = formatError(e);
    }
  }

  async function copyPrivate() {
    try {
      await navigator.clipboard.writeText(await loadPrivate());
    } catch (e) {
      error = formatError(e);
    }
  }

  async function saveEdit() {
    if (!viewKey || !editName.trim()) return;
    savingEdit = true;
    try {
      const updated = await api.sshKeys.update(viewKey.id, {
        name: editName.trim(),
        comment: editComment.trim() || null,
      });
      keys = keys.map((k) => (k.id === updated.id ? updated : k));
      viewKey = null;
    } catch (e) {
      error = formatError(e);
    } finally {
      savingEdit = false;
    }
  }

  async function confirmDelete() {
    try {
      await api.sshKeys.delete(deleteModal.id);
      keys = keys.filter((k) => k.id !== deleteModal.id);
      // Connections referencing the key had ssh_key_id cleared server-side.
      if (deleteModal.usage > 0) await sshStore.loadConnections();
    } catch (e) {
      error = formatError(e);
    } finally {
      deleteModal = { open: false, id: '', name: '', usage: 0 };
    }
  }

  function onFormSaved(key: SshKey) {
    keys = [key, ...keys.filter((k) => k.id !== key.id)];
    formMode = undefined;
  }
</script>

{#if error}<div class="error-msg" style="margin-bottom:1rem">{error}</div>{/if}

<!-- Toolbar -->
<div class="filter-bar">
  <div class="search-field">
    <Icon name="search" size={13} />
    <input type="text" bind:value={search} placeholder={$t('ssh_keys_search_placeholder')} />
    {#if search}
      <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={11} /></button>
    {/if}
  </div>

  <span class="count-badge">{$t('ssh_keys_count', { n: String(filtered.length) })}</span>

  <div class="toolbar-actions">
    <button class="btn btn-ghost" onclick={() => (formMode = 'import')}>
      <Icon name="download" size={14} />{$t('ssh_keys_add')}
    </button>
    <button class="btn btn-primary" onclick={() => (formMode = 'generate')}>
      <Icon name="zap" size={14} />{$t('ssh_keys_generate')}
    </button>
  </div>
</div>

{#if loading}
  <div class="empty-state">{$t('loading')}</div>
{:else if keys.length === 0}
  <div class="empty-state">
    <div class="empty-icon"><Icon name="key" size={40} strokeWidth={1.5} /></div>
    <p>{$t('ssh_keys_empty')}</p>
    <button class="btn btn-primary" onclick={() => (formMode = 'generate')}>
      <Icon name="zap" size={14} />{$t('ssh_keys_generate')}
    </button>
  </div>
{:else if filtered.length === 0}
  <div class="empty-state">
    <Icon name="search" size={32} strokeWidth={1.5} />
    <p>{$t('terminal_not_found')}</p>
  </div>
{:else}
  <div class="table-wrap">
    <table class="keys-table">
      <thead>
        <tr>
          <th class="col-num">#</th>
          <th>{$t('ssh_key_col_name')}</th>
          <th class="col-algo">{$t('ssh_key_col_type')}</th>
          <th class="col-fp">{$t('ssh_key_col_fingerprint')}</th>
          <th class="col-usage">{$t('ssh_key_col_usage')}</th>
          <th class="col-created">{$t('ssh_key_col_created')}</th>
          <th class="col-actions">{$t('ssh_key_col_actions')}</th>
        </tr>
      </thead>
      <tbody>
        {#each filtered as k, i (k.id)}
          <tr class="key-row" onclick={() => openView(k)}>
            <td class="col-num text-muted">{i + 1}</td>
            <td class="col-name">
              <span class="key-name">{k.name}</span>
              {#if k.comment}<span class="key-comment">{k.comment}</span>{/if}
            </td>
            <td class="col-algo">
              <span class="auth-badge">{algoLabel(k)}</span>
              {#if k.has_passphrase}
                <span class="pass-badge" title={$t('ssh_key_selected_passphrase_hint')}>
                  <Icon name="shield" size={10} />
                </span>
              {/if}
            </td>
            <td class="col-fp">
              {#if k.fingerprint}<code title={k.fingerprint}>{k.fingerprint}</code>{:else}—{/if}
            </td>
            <td class="col-usage">
              {#if k.usage_count > 0}
                <span class="usage-chip">{$t('ssh_key_usage_n', { n: String(k.usage_count) })}</span>
              {:else}
                <span class="text-muted">{$t('ssh_key_unused')}</span>
              {/if}
            </td>
            <td class="col-created">
              <span class="text-muted">{formatDate(k.created_at, $locale)}</span>
            </td>
            <td class="col-actions">
              <div class="row-actions">
                <button
                  class="icon-btn"
                  class:success={copiedId === k.id}
                  title={$t('ssh_key_btn_copy_public')}
                  onclick={(e) => { e.stopPropagation(); copyPublic(k); }}
                >
                  <Icon name={copiedId === k.id ? 'check' : 'copy'} size={13} />
                </button>
                <button
                  class="icon-btn"
                  title={$t('ssh_key_btn_view')}
                  onclick={(e) => { e.stopPropagation(); openView(k); }}
                >
                  <Icon name="eye" size={13} />
                </button>
                <button
                  class="icon-btn danger-soft"
                  title={$t('ssh_key_btn_delete')}
                  onclick={(e) => {
                    e.stopPropagation();
                    deleteModal = { open: true, id: k.id, name: k.name, usage: k.usage_count };
                  }}
                >
                  <Icon name="trash-2" size={13} />
                </button>
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<!-- Add / generate drawer -->
<Drawer
  open={formMode !== undefined}
  width="440px"
  title={formMode === 'generate' ? $t('ssh_keys_generate') : $t('ssh_keys_add')}
  onclose={() => (formMode = undefined)}
>
  {#if formMode !== undefined}
    <SshKeyForm mode={formMode} onSave={onFormSaved} onCancel={() => (formMode = undefined)} />
  {/if}
</Drawer>

<!-- View / rename drawer -->
<Drawer
  open={viewKey !== null}
  width="480px"
  title={viewKey?.name ?? ''}
  onclose={() => (viewKey = null)}
>
  {#if viewKey}
    <div class="view-body">
      <div class="view-meta">
        <span class="auth-badge">{algoLabel(viewKey)}</span>
        {#if viewKey.fingerprint}<code class="fp">{viewKey.fingerprint}</code>{/if}
      </div>

      <div class="form-group">
        <label for="ssh-key-edit-name">{$t('ssh_key_field_name')}</label>
        <input id="ssh-key-edit-name" type="text" bind:value={editName} />
      </div>
      <div class="form-group">
        <label for="ssh-key-edit-comment">{$t('ssh_key_field_comment')}</label>
        <input id="ssh-key-edit-comment" type="text" bind:value={editComment} />
      </div>

      <div class="form-group">
        <span class="field-label">{$t('ssh_key_public_label')}</span>
        <textarea class="mono" readonly rows="4" value={viewKey.public_key}></textarea>
        <button class="btn btn-ghost btn-sm self-start" onclick={() => copyPublic(viewKey!)}>
          <Icon name={copiedId === viewKey.id ? 'check' : 'copy'} size={12} />
          {copiedId === viewKey.id ? $t('ssh_key_copied') : $t('ssh_key_btn_copy_public')}
        </button>
      </div>

      <div class="form-group">
        <span class="field-label">{$t('ssh_key_private_label')}</span>
        {#if revealPrivate}
          <textarea class="mono" readonly rows="9" value={privateKeyText}></textarea>
        {:else}
          <div class="private-hidden">
            <Icon name="eye-off" size={14} />
            {$t('ssh_key_private_hidden')}
          </div>
        {/if}
        <div class="private-actions">
          <button class="btn btn-ghost btn-sm" onclick={toggleReveal}>
            <Icon name={revealPrivate ? 'eye-off' : 'eye'} size={12} />
            {revealPrivate ? $t('ssh_key_hide') : $t('ssh_key_reveal')}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={copyPrivate}>
            <Icon name="copy" size={12} />
            {$t('ssh_key_btn_copy_private')}
          </button>
        </div>
      </div>

      <div class="form-actions">
        <button class="btn btn-ghost" onclick={() => (viewKey = null)}>{$t('ssh_btn_cancel')}</button>
        <button class="btn btn-primary" onclick={saveEdit} disabled={savingEdit || !editName.trim()}>
          {#if savingEdit}<Icon name="loader" size={13} />{/if}
          {$t('ssh_btn_save')}
        </button>
      </div>
    </div>
  {/if}
</Drawer>

<Modal
  open={deleteModal.open}
  title={$t('ssh_key_btn_delete')}
  message={deleteModal.usage > 0
    ? $t('ssh_key_confirm_delete_used', { name: deleteModal.name, n: String(deleteModal.usage) })
    : $t('ssh_key_confirm_delete', { name: deleteModal.name })}
  confirmLabel={$t('ssh_key_btn_delete')}
  cancelLabel={$t('ssh_btn_cancel')}
  variant="danger"
  onconfirm={confirmDelete}
  oncancel={() => (deleteModal = { open: false, id: '', name: '', usage: 0 })}
/>

<style>
  .toolbar-actions { display: flex; gap: var(--sp-2); margin-left: auto; }
  /* The placeholder names three fields; the primitive's 240px cut it. */
  .filter-bar .search-field { width: 300px; }

  .table-wrap {
    flex: 1; min-height: 0; overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .keys-table {
    width: 100%; border-collapse: collapse; font-size: var(--fs-sm);
    table-layout: fixed;
  }
  .keys-table thead {
    position: sticky; top: 0; z-index: 1;
    background: var(--surface); border-bottom: 1px solid var(--border);
  }
  .keys-table th {
    padding: var(--sp-3) var(--sp-4); text-align: left;
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-3);
    text-transform: uppercase; letter-spacing: 0.7px;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .keys-table td { padding: var(--sp-3) var(--sp-4); border-bottom: 1px solid var(--surface-2); vertical-align: middle; }
  .key-row:last-child td { border-bottom: none; }
  .key-row:hover td { background: var(--surface-row-hover); }
  .key-row { cursor: pointer; }

  .col-num { width: 44px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .col-algo { width: 140px; }
  .col-fp { width: 220px; }
  .col-usage { width: 144px; }
  .col-created { width: 110px; }
  /* three 32px buttons, two gaps and the cell's padding */
  .col-actions { width: calc(3 * 32px + 2 * var(--sp-1) + 2 * var(--sp-4)); }

  .key-name { font-weight: var(--fw-semibold); font-size: 0.9rem; color: var(--text); display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .key-comment { display: block; color: var(--text-3); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  code {
    font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-body);
    background: var(--surface-2); padding: 4px 8px; border-radius: 7px;
    display: inline-block; max-width: 100%;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }

  .auth-badge {
    font-family: var(--font-mono);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    padding: 4px 12px; border-radius: var(--radius-sm);
    border: none; background: var(--accent-tint); color: var(--accent-text-2);
  }
  .pass-badge {
    display: inline-flex; align-items: center; margin-left: var(--sp-1);
    color: var(--warn-text);
  }

  .usage-chip {
    font-size: var(--fs-2xs); font-weight: 700; letter-spacing: 0.04em;
    padding: 0.15rem var(--sp-2); border-radius: 999px;
    background: var(--success-bg);
    border: 1px solid color-mix(in srgb, var(--success) 30%, var(--border));
    color: var(--success-text);
    white-space: nowrap;
  }

  .text-muted { color: var(--text-3); font-size: var(--fs-xs); }
  .row-actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }
  .empty-icon { opacity: 0.4; }

  /* View drawer */
  .view-body { display: flex; flex-direction: column; gap: var(--sp-4); }
  .view-meta { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .view-meta .fp { max-width: 100%; }
  /* The fields of the view drawer only: the toolbar's search keeps the
     padding .search-field reserves for its icon. */
  .view-body .form-group { display: flex; flex-direction: column; gap: 8px; }
  .view-body label, .view-body .field-label {
    font-size: 0.72rem; text-transform: uppercase; color: var(--text-faint);
    font-weight: var(--fw-bold); letter-spacing: 0.5px;
  }
  .view-body input, .view-body textarea {
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    color: var(--text);
    padding: 0 14px;
    height: var(--control-h-lg);
    font-size: 0.9rem;
    font-family: inherit;
    resize: vertical;
  }
  .view-body textarea { height: auto; min-height: 72px; padding: 12px 14px; font-size: 0.75rem; }
  .view-body input:focus, .view-body textarea:focus {
    outline: none;
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--accent-bg);
  }
  .private-hidden {
    display: flex; align-items: center; gap: var(--sp-2);
    padding: var(--sp-4);
    background: var(--surface-3); border: 1px dashed var(--border);
    border-radius: var(--radius-field);
    color: var(--text-3); font-size: var(--fs-sm);
  }
  .private-actions { display: flex; gap: var(--sp-2); }
  .self-start { align-self: flex-start; }
  .form-actions { display: flex; gap: 10px; justify-content: flex-end; margin-top: var(--sp-2); }
  .form-actions .btn { height: 42px; border-radius: var(--radius-field); padding: 0 20px; }
</style>

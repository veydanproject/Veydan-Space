<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import Pill from '$lib/core/ui/Pill.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { onMount } from 'svelte';
  import { api } from '$lib/ssh/api';
  import type { SshKey } from '$lib/ssh/types';
  import Icon from '$lib/core/Icon.svelte';
  import { pinnedHead } from '$lib/core/pinnedHead';
  import { ask, confirmTitle } from '$lib/core/ui/confirm.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import SshKeyForm from './SshKeyForm.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { formatDate, formatError } from '$lib/core/utils';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';

  interface Props {
    /** The window of a new key (import / generate), undefined = closed; the buttons that open it are the page's head. */
    formMode?: 'import' | 'generate' | undefined;
  }
  let { formMode = $bindable() }: Props = $props();

  let keys = $state<SshKey[]>([]);
  let loading = $state(false);
  let error = $state('');
  let search = $state('');

  // view/rename drawer
  let viewKey = $state<SshKey | null>(null);
  let revealPrivate = $state(false);
  // Fetched from the backend only when the user reveals or copies it.
  let privateKeyText = $state('');
  let editName = $state('');
  let editComment = $state('');
  let savingEdit = $state(false);

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
      copiedToast();
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
      copiedToast();
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

  async function confirmDelete(k: SshKey) {
    const label = $t('ssh_key_btn_delete');
    const usage = k.usage_count;
    const message = usage > 0
      ? $t('ssh_key_confirm_delete_used', { name: k.name, n: String(usage) })
      : $t('ssh_key_confirm_delete', { name: k.name });
    if (!(await ask({ title: confirmTitle(label), message, confirmLabel: label, variant: 'danger', icon: 'trash-2' }))) return;
    try {
      await api.sshKeys.delete(k.id);
      keys = keys.filter((x) => x.id !== k.id);
      // Connections referencing the key had ssh_key_id cleared server-side.
      if (usage > 0) await sshStore.loadConnections();
    } catch (e) {
      error = formatError(e);
    }
  }

  function onFormSaved(key: SshKey) {
    keys = [key, ...keys.filter((k) => k.id !== key.id)];
    formMode = undefined;
  }
</script>

{#if error}<div style="margin-bottom:1rem"><Notice tone="danger">{error}</Notice></div>{/if}

<!-- Toolbar -->
<div class="filter-bar">
  <div class="search-field">
    <Icon name="search" size={18} />
    <input type="text" bind:value={search} placeholder={$t('ssh_keys_search_placeholder')} />
    {#if search}
      <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={18} /></button>
    {/if}
  </div>

  <span class="count-badge">{$t('ssh_keys_count', { n: String(filtered.length) })}</span>
</div>

{#if loading}
  <div class="empty-state">{$t('loading')}</div>
{:else if keys.length === 0}
  <div class="empty-state">
    <span class="empty-icon"><Icon name="key" size={22} /></span>
    <p>{$t('ssh_keys_empty')}</p>
    <button class="btn btn-primary" onclick={() => (formMode = 'generate')}>
      <Icon name="plus" size={18} />{$t('ssh_keys_generate')}
    </button>
  </div>
{:else if filtered.length === 0}
  <div class="empty-state">
    <span class="empty-icon"><Icon name="search" size={22} /></span>
    <p>{$t('common_nothing_found')}</p>
    <span>{$t('common_nothing_found_hint', { q: search.trim() })}</span>
  </div>
{:else}
  <div class="data-table table-scroll" use:pinnedHead>
    <table class="keys-table pinned-head">
      <thead class="data-table-head">
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
          <tr class="data-table-row key-row" onclick={() => openView(k)}>
            <td class="col-num text-muted">{i + 1}</td>
            <td class="col-name">
              <span class="key-name">{k.name}</span>
              {#if k.comment}<span class="key-comment">{k.comment}</span>{/if}
            </td>
            <td class="col-algo">
              <Pill class="tag" tone="accent" text={algoLabel(k)} />
              {#if k.has_passphrase}
                <span class="pass-badge" title={$t('ssh_key_selected_passphrase_hint')}>
                  <Icon name="lock" size={18} />
                </span>
              {/if}
            </td>
            <td class="col-fp">
              {#if k.fingerprint}<code title={k.fingerprint}>{k.fingerprint}</code>{:else}—{/if}
            </td>
            <td class="col-usage">
              {#if k.usage_count > 0}
                <Pill class="tag" tone="ok" text={$t('ssh_key_usage_n', { n: String(k.usage_count) })} />
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
                  title={$t('ssh_key_btn_copy_public')}
                  onclick={(e) => { e.stopPropagation(); copyPublic(k); }}
                >
                  <Icon name="copy" size={18} />
                </button>
                <button
                  class="icon-btn"
                  title={$t('ssh_key_btn_view')}
                  onclick={(e) => { e.stopPropagation(); openView(k); }}
                >
                  <Icon name="eye" size={18} />
                </button>
                <button
                  class="icon-btn danger-soft"
                  title={$t('ssh_key_btn_delete')}
                  onclick={(e) => {
                    e.stopPropagation();
                    void confirmDelete(k);
                  }}
                >
                  <Icon name="trash-2" size={18} />
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
{#if formMode !== undefined}
  <SshKeyForm mode={formMode} onSave={onFormSaved} onCancel={() => (formMode = undefined)} />
{/if}

<!-- View / rename drawer -->
<Drawer
  open={viewKey !== null}
  width="var(--drawer-w)"
  title={viewKey?.name ?? ''}
  onclose={() => (viewKey = null)}
>
  {#if viewKey}
    <div class="view-body">
      <div class="view-meta">
        <Pill class="tag" tone="accent" text={algoLabel(viewKey)} />
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
        <label for="ssh-key-view-public">{$t('ssh_key_public_label')}</label>
        <textarea id="ssh-key-view-public" class="mono" readonly rows="4" value={viewKey.public_key}></textarea>
        <button class="btn btn-ghost btn-sm self-start" onclick={() => copyPublic(viewKey!)}>
          <Icon name="copy" size={18} />
          {$t('ssh_key_btn_copy_public')}
        </button>
      </div>

      <div class="form-group">
        <label for="ssh-key-view-private">{$t('ssh_key_private_label')}</label>
        {#if revealPrivate}
          <textarea id="ssh-key-view-private" class="mono" readonly rows="9" value={privateKeyText}></textarea>
        {:else}
          <div class="private-hidden">
            <Icon name="eye-off" size={18} />
            {$t('ssh_key_private_hidden')}
          </div>
        {/if}
        <div class="private-actions">
          <button class="btn btn-ghost btn-sm" onclick={toggleReveal}>
            <Icon name={revealPrivate ? 'eye-off' : 'eye'} size={18} />
            {revealPrivate ? $t('ssh_key_hide') : $t('ssh_key_reveal')}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={copyPrivate}>
            <Icon name="copy" size={18} />
            {$t('ssh_key_btn_copy_private')}
          </button>
        </div>
      </div>
    </div>
  {/if}
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (viewKey = null)}>{$t('common_cancel')}</button>
    <button class="btn btn-primary" onclick={saveEdit} disabled={savingEdit || !editName.trim()}>
      {#if savingEdit}<Icon name="loader" size={18} />{/if}
      {$t('common_save')}
    </button>
  {/snippet}
</Drawer>

<style>
  /* The placeholder names three fields; the primitive's 240px cut it. */
  .filter-bar .search-field { width: 300px; }

  /* Table: the common glass .data-table, as high as its rows, scrolled inside when the page is short. */
  .table-scroll { flex: 0 1 auto; min-height: 0; overflow-y: auto; }
  .keys-table { width: 100%; font-size: var(--fs-sm); }
  .keys-table th {
    padding: 0 var(--sp-4); text-align: left;
    font: inherit; white-space: nowrap;
  }
  .keys-table td { padding: var(--sp-3) var(--sp-4); vertical-align: middle; }
  .key-row { cursor: pointer; }

  .col-num { width: 44px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .col-fp { max-width: 220px; }
  /* the buttons stay on one line */
  .col-actions { width: 1%; }

  .key-name { font-weight: var(--fw-semibold); font-size: var(--fs-base); color: var(--text); display: block; overflow-wrap: anywhere; }
  .key-comment { display: block; color: var(--text-3); font-size: var(--fs-xs); overflow-wrap: anywhere; }

  code {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body);
    background: var(--surface-2); padding: 4px 8px; border-radius: var(--radius-sm);
    display: inline-block; max-width: 100%;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; vertical-align: middle;
  }

  /* A word in a chip (the common .badge): 24 high, the interface's font. */
  :is(.data-table, .view-meta) :global(.tag) { height: 24px; padding-block: 0; box-sizing: border-box; }
  .pass-badge {
    display: inline-flex; align-items: center; margin-left: var(--sp-1); vertical-align: middle;
    color: var(--warn-text);
  }

  .text-muted { color: var(--text-3); font-size: var(--fs-xs); }
  .row-actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }

  /* A narrow window: a row folds, as the connections' table — the name on top, the fingerprint under it, the buttons
     at the right; the number, the type, the usage and the date a line below. No column is lost. */
  @media (max-width: 900px) {
    .keys-table thead { display: none; }
    .keys-table tbody { display: block; }
    .key-row {
      display: grid; align-items: center; column-gap: var(--sp-3); row-gap: var(--sp-1); padding: var(--sp-3) var(--sp-4);
      grid-template-columns: 2.5rem auto auto minmax(0, 1fr) auto;
      grid-template-areas: 'name name name name act' 'fp fp fp fp act' 'num algo usage created created';
    }
    .keys-table td { display: block; width: auto; border: 0; max-width: none; padding: 0; min-width: 0; }
    .key-row .col-num { grid-area: num; }
    .key-row .col-name { grid-area: name; }
    .key-row .col-algo { grid-area: algo; }
    .key-row .col-fp { grid-area: fp; }
    .key-row .col-usage { grid-area: usage; }
    .key-row .col-created { grid-area: created; }
    .key-row .col-actions { grid-area: act; }
  }

  /* View drawer: the common captions and fields on the panel's glass, 40 high. */
  .view-body { display: flex; flex-direction: column; gap: var(--sp-4); }
  .view-meta { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .view-meta .fp { max-width: 100%; }
  .view-body input { height: var(--control-h); }
  .view-body textarea { resize: vertical; font-size: var(--fs-xs); }
  .private-hidden {
    display: flex; align-items: center; gap: var(--sp-2);
    padding: var(--sp-4);
    border: 1px dashed var(--border);
    border-radius: var(--radius-field);
    color: var(--text-3); font-size: var(--fs-sm);
  }
  .private-actions { display: flex; gap: var(--sp-2); }
  .self-start { align-self: flex-start; }
</style>

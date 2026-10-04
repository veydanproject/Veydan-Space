<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { directory } from '$lib/core/directory';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { userLabels } from '$lib/core/entity-tags';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import Pane from '$lib/core/ui/Pane.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import PasswordList from './PasswordList.svelte';
  import PasswordCard from './PasswordCard.svelte';
  import PasswordEditModal from './PasswordEditModal.svelte';
  import PasswordLockBadge from './PasswordLockBadge.svelte';
  import { passUi } from '$lib/pass/store/ui.svelte';

  interface Props {
    open?: boolean;
    context?: 'global' | 'workspace';
    contextId?: string;
    /** `pane`: the body of the desktop Pass page instead of a drawer; it answers the palette and the notes only where there are no drawers (Pass). */
    as?: 'drawer' | 'pane';
  }

  let { open = $bindable(false), context = 'global', contextId, as = 'drawer' }: Props = $props();
  const Frame = $derived(as === 'pane' ? Pane : Drawer);
  /** Whether this one answers a request to create or open a password: an open drawer, or the pane where no drawers are. */
  const answers = $derived(as === 'pane' ? !passUi.drawers : open);

  let search = $state('');
  let selectedId = $state<string | null>(null);
  let editingId = $state<string | null>(null);
  let creating = $state(false);
  /** null shows every workspace */
  let activeWorkspace = $state<string | null>(null);

  const canWrite = $derived(!appLock.locked && appLock.status.vault !== 'mismatch');
  const selected = $derived(passwordStore.list.find((entry) => entry.id === selectedId) ?? null);
  const editingEntry = $derived(passwordStore.list.find((entry) => entry.id === editingId) ?? null);

  onMount(() => {
    void appLock.refresh();
    void appLock.listen();
    void passwordStore.ensureLoaded();
    void directory.ensureLoaded();
    void totpStore.ensureLoaded();
  });

  $effect(() => {
    if (!appLock.locked && passwordStore.loaded) {
      void passwordStore.refresh();
    }
  });

  $effect(() => {
    if (open) activeWorkspace = context === 'workspace' && contextId ? contextId : null;
  });

  $effect(() => {
    if (!open) {
      selectedId = null;
      editingId = null;
      creating = false;
      if (as !== 'pane') passwordStore.createRequest = false;
    }
  });

  $effect(() => {
    if (!answers || !passwordStore.createRequest || !appLock.ready) return;
    passwordStore.createRequest = false;
    if (canWrite) creating = true;
  });

  $effect(() => {
    const id = passwordStore.openId;
    if (!answers || !id) return;
    selectedId = id;
    passwordStore.openId = null;
  });

  const filtered = $derived.by(() => {
    let list = passwordStore.list;
    if (activeWorkspace) {
      const profileIds = directory.list('profile').filter((p) => p.parent === `workspace:${activeWorkspace}`).map((p) => p.id);
      list = passwordStore.byWorkspace(activeWorkspace, profileIds);
    }
    const q = search.trim().toLowerCase();
    if (!q) return list;
    return list.filter((entry) => {
      const labels = userLabels(entry.tags).join(' ');
      const profiles = entry.tags
        .filter((tag) => tag.startsWith('profile:'))
        .map((tag) => directory.linkName('profile', tag.slice('profile:'.length)) ?? '')
        .join(' ');
      const notes = entry.tags
        .filter((tag) => tag.startsWith('note:'))
        .map((tag) => directory.linkName('note', tag.slice('note:'.length)) ?? '')
        .join(' ');
      const totp = entry.totp_ids
        .map((id) => totpStore.list.find((t) => t.id === id))
        .map((t) => (t ? `${t.issuer ?? ''} ${t.name}` : ''))
        .join(' ');
      return `${entry.title} ${entry.username ?? ''} ${entry.url ?? ''} ${labels} ${profiles} ${notes} ${totp}`
        .toLowerCase()
        .includes(q);
    });
  });

  function toggleWorkspace(id: string) {
    activeWorkspace = activeWorkspace === id ? null : id;
  }

  function initialTags(): string[] {
    return context === 'workspace' && contextId ? [`workspace:${contextId}`] : [];
  }
</script>

<Frame bind:open title={$t('pw_title')} width="var(--drawer-w-lg)">
  {#snippet titleBadge()}
    <PasswordLockBadge />
  {/snippet}

  {#snippet actions()}
    <span class="count">{filtered.length}</span>
    {#if canWrite && !selected}
      <button class="icon-btn" title={$t('pw_btn_add')} onclick={() => (creating = true)}>
        <Icon name="plus" size={14} />
      </button>
    {/if}
  {/snippet}

  {#snippet subheader()}
    {#if !selected}
      <div class="panel-search">
        <div class="search-wrap">
          <span class="search-icon"><Icon name="search" size={13} /></span>
          <input class="search-input" bind:value={search} placeholder={$t('pw_search')} />
        </div>
      </div>
      {#if directory.list('workspace').length}
        <div class="ws-chips">
          {#if context === 'workspace'}
            <button type="button" class="chip-btn" class:active={activeWorkspace === null} onclick={() => (activeWorkspace = null)}>
              {$t('totp_show_all')}
            </button>
          {/if}
          {#each directory.list('workspace') as ws (ws.id)}
            <button type="button" class="chip-btn" class:active={activeWorkspace === ws.id} onclick={() => toggleWorkspace(ws.id)}>
              <span class="ws-dot" style:background={ws.color}></span>
              {ws.name}
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  {/snippet}

  {#if selected}
    <button type="button" class="btn btn-ghost btn-sm back" onclick={() => (selectedId = null)}>
      <Icon name="chevron-left" size={14} /> {$t('pw_back')}
    </button>
    <PasswordCard entry={selected} onedit={() => (editingId = selected.id)} ondeleted={() => (selectedId = null)} />
  {:else if passwordStore.loading && !passwordStore.loaded}
    <p class="muted">{$t('loading')}</p>
  {:else if filtered.length === 0}
    <div class="empty-state">
      {#if search.trim()}
        <span class="empty-icon"><Icon name="search" size={22} /></span>
        <p>{$t('pass_nothing_found')}</p>
        <span>{$t('pass_nothing_found_hint', { q: search.trim() })}</span>
      {:else}
        <span class="empty-icon"><Icon name="lock" size={22} /></span>
        <p>{$t('pw_empty')}</p>
        {#if canWrite}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (creating = true)}>
            <Icon name="plus" size={13} /> {$t('cmd_passwords_create')}
          </button>
        {/if}
      {/if}
    </div>
  {:else}
    <PasswordList entries={filtered} onopen={(id) => (selectedId = id)} onedit={(entry) => (editingId = entry.id)} />
  {/if}
</Frame>

{#if creating}
  <PasswordEditModal initialTags={initialTags()} onclose={() => (creating = false)} />
{/if}
{#if editingEntry}
  <PasswordEditModal entry={editingEntry} onclose={() => (editingId = null)} />
{/if}

<style>
  .count {
    background: var(--accent-tint);
    color: var(--accent-text-2);
    border: 1px solid var(--accent-tint-border);
    border-radius: var(--radius-sm);
    font-size: var(--fs-2xs);
    font-family: var(--font-mono);
    padding: 0.05rem 0.4rem;
    font-weight: var(--fw-semibold);
  }
  .panel-search { padding: var(--sp-3) var(--sp-4) 0; }
  .search-wrap { position: relative; }
  .search-icon {
    position: absolute;
    left: 0.6rem;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-2);
    display: flex;
    pointer-events: none;
  }
  .search-input {
    width: 100%;
    box-sizing: border-box;
    height: 34px;
    padding: 0 10px 0 2rem;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-3);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
  }
  .search-input:focus { outline: none; border-color: var(--accent-border); }
  .ws-chips {
    display: flex;
    gap: 0.35rem;
    padding: 0.6rem var(--sp-4);
    flex-wrap: wrap;
    border-bottom: 1px solid var(--border);
  }
  .chip-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: var(--sp-1) 0.6rem;
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-3);
    color: var(--text-2);
    cursor: pointer;
  }
  .chip-btn.active {
    background: var(--accent-tint);
    border-color: var(--accent-tint-border);
    color: var(--accent-text);
  }
  .ws-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
  .muted { color: var(--text-2); font-size: 0.85rem; }
  .back { margin-bottom: var(--sp-3); }
</style>

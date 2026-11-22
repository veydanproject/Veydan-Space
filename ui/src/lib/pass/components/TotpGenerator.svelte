<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { directory } from '$lib/core/directory';
  import type { TotpEntry } from '$lib/pass/types';
  import TotpList from './TotpList.svelte';
  import TotpAddModal from './TotpAddModal.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { refocusSearch } from '$lib/core/ui/searchField';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import Pane from '$lib/core/ui/Pane.svelte';
  import { passUi } from '$lib/pass/store/ui.svelte';

  interface Props {
    open?: boolean;
    /** 'global' — no auto-filter; 'workspace' — pre-filter to this workspace with toggle */
    context?: 'global' | 'workspace';
    contextId?: string;
    /** `pane`: the body of the desktop Pass page instead of a drawer; it answers a search from a note's card only where there are no drawers. */
    as?: 'drawer' | 'pane';
  }

  let { open = $bindable(false), context = 'global', contextId, as = 'drawer' }: Props = $props();
  const Frame = $derived(as === 'pane' ? Pane : Drawer);
  const answers = $derived(as === 'pane' ? !passUi.drawers : open);

  let search = $state('');
  let showAdd = $state(false);
  let editing = $state<TotpEntry | null>(null);
  const formOpen = $derived(showAdd || editing !== null);
  /** In a drawer the form takes the list's place in it; over the page's pane it is a side panel of its own. */
  const inPlace = $derived(as === 'drawer' && formOpen);
  let form = $state<ReturnType<typeof TotpAddModal>>();
  let saving = $state(false);

  function closeForm() {
    showAdd = false;
    editing = null;
  }
  /** null = show all; workspace id string = filter to that workspace */
  let activeWorkspace = $state<string | null>(null);

  onMount(() => {
    totpStore.ensureLoaded();
    void directory.ensureLoaded();
  });

  // When panel opens (or context changes), reset workspace filter
  $effect(() => {
    if (open) {
      activeWorkspace = context === 'workspace' && contextId ? contextId : null;
      totpStore.ensureLoaded();
    }
  });

  $effect(() => {
    if (!open) closeForm();
  });

  $effect(() => {
    if (!answers || totpStore.pendingSearch === null) return;
    search = totpStore.pendingSearch;
    totpStore.pendingSearch = null;
  });

  const filteredEntries = $derived.by(() => {
    let list = totpStore.list;

    // Workspace filter
    if (activeWorkspace) {
      const profileIds = directory.list('profile').filter((p) => p.parent === `workspace:${activeWorkspace}`).map((p) => p.id);
      list = totpStore.byWorkspace(activeWorkspace, profileIds);
    }

    // Search filter
    if (search.trim()) {
      const q = search.toLowerCase();
      list = list.filter((e) => {
        if (e.name.toLowerCase().includes(q)) return true;
        if (e.issuer?.toLowerCase().includes(q)) return true;
        if (e.tags.some((tag) => tag.toLowerCase().includes(q))) return true;
        // Also match profile name
        const pTag = e.tags.find((t) => t.startsWith('profile:'));
        if (pTag) {
          const pid = pTag.slice('profile:'.length);
          const pname = directory.linkName('profile', pid) ?? '';
          if (pname.toLowerCase().includes(q)) return true;
        }
        return false;
      });
    }

    return list;
  });

  function formTags(): string[] {
    return !editing && context === 'workspace' && contextId ? [`workspace:${contextId}`] : [];
  }

  function toggleWorkspace(id: string) {
    activeWorkspace = activeWorkspace === id ? null : id;
  }
</script>

<Frame
  bind:open
  title={inPlace ? $t(editing ? 'totp_edit_title' : 'totp_add_title') : $t('totp_title')}
  footer={inPlace ? formActions : undefined}
>
  {#snippet actions()}
    {#if !inPlace}
      <span class="count-pill quiet">{filteredEntries.length}</span>
      <button class="icon-btn create" onclick={() => (showAdd = true)} title={$t('totp_btn_add')} aria-label={$t('totp_btn_add')}>
        <Icon name="plus" size={18} />
      </button>
    {/if}
  {/snippet}

  {#snippet subheader()}
    {#if inPlace}
      <div class="panel-back">
        <button type="button" class="btn btn-ghost btn-sm" onclick={closeForm}>
          <Icon name="chevron-left" size={14} /> {$t('common_back')}
        </button>
      </div>
    {:else}
      <div class="panel-search">
        <div class="search-field fill">
          <Icon name="search" size={16} />
          <input
            type="text"
            bind:value={search}
            placeholder={$t('totp_search_placeholder')}
          />
          {#if search}
            <button type="button" class="search-clear" onclick={(e) => { search = ''; refocusSearch(e); }} aria-label={$t('common_clear')}><Icon name="x" size={16} /></button>
          {/if}
        </div>
      </div>

      <!-- Workspace chips -->
      {#if directory.list('workspace').length > 0}
        <div class="ws-chips">
          {#if context === 'workspace'}
            <button
              class="chip-btn"
              class:active={activeWorkspace === null}
              onclick={() => (activeWorkspace = null)}
            >
              {$t('totp_show_all')}
            </button>
          {/if}
          {#each directory.list('workspace') as ws (ws.id)}
            <button
              class="chip-btn"
              class:active={activeWorkspace === ws.id}
              onclick={() => toggleWorkspace(ws.id)}
              style="--ws-color: {ws.color}"
            >
              <span class="status-dot" style="background:{ws.color}"></span>
              {ws.name}
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  {/snippet}

      {#if inPlace}
        <TotpAddModal bind:this={form} bind:saving frame="host" entry={editing} initialTags={formTags()} onclose={closeForm} />
      {:else if totpStore.loading && !totpStore.loaded}
        <div class="loading-msg">{$t('loading')}</div>
      {:else}
        <!-- No add button in the empty list: the head has it. -->
        <TotpList entries={filteredEntries} showProfileBadge={true} query={search.trim()} onedit={(entry) => (editing = entry)} />
      {/if}

</Frame>

<!-- Keyed by the entry: another TOTP gets a fresh form, no fields carried over. -->
{#if as === 'pane' && formOpen}
  {#key editing?.id}
    <TotpAddModal entry={editing} initialTags={formTags()} onclose={closeForm} />
  {/key}
{/if}

{#snippet formActions()}
  <button type="button" class="btn btn-ghost" onclick={closeForm}>{$t('common_cancel')}</button>
  <button type="button" class="btn btn-primary" onclick={() => form?.save()} disabled={saving}>
    {saving ? $t('loading') : $t('common_save')}
  </button>
{/snippet}

<style>
  .panel-back { padding: var(--sp-3) var(--sp-4) 0; }

  .panel-search {
    padding: var(--sp-3) var(--sp-4) 0;
    flex-shrink: 0;
  }

  .ws-chips {
    display: flex;
    gap: var(--sp-1-5);
    padding: var(--sp-2-5) var(--sp-4);
    flex-wrap: wrap;
    flex-shrink: 0;
    border-bottom: 1px solid var(--border);
  }

  .chip-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-1) var(--sp-2-5);
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-3);
    cursor: pointer;
    color: var(--text-2);
    transition: all var(--dur-fast);
  }

  .chip-btn:hover { border-color: var(--border-2); color: var(--text); }

  .chip-btn.active {
    background: var(--accent-tint);
    border-color: var(--accent-tint-border);
    color: var(--accent-text);
  }

  .loading-msg {
    text-align: center;
    padding: var(--sp-8);
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
</style>

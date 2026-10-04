<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { directory } from '$lib/core/directory';
  import TotpList from './TotpList.svelte';
  import TotpAddModal from './TotpAddModal.svelte';
  import Icon from '$lib/core/Icon.svelte';
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

  function toggleWorkspace(id: string) {
    activeWorkspace = activeWorkspace === id ? null : id;
  }
</script>

<Frame bind:open title={$t('totp_title')}>
  {#snippet actions()}
    <span class="count-badge">{filteredEntries.length}</span>
  {/snippet}

  {#snippet subheader()}
      <div class="panel-search">
        <div class="search-wrap">
          <span class="search-icon"><Icon name="search" size={13} /></span>
          <input
            type="text"
            bind:value={search}
            placeholder={$t('totp_search_placeholder')}
            class="search-input"
          />
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
              <span class="ws-dot" style="background:{ws.color}"></span>
              {ws.name}
            </button>
          {/each}
        </div>
      {/if}
  {/snippet}

      {#if totpStore.loading && !totpStore.loaded}
        <div class="loading-msg">{$t('loading')}</div>
      {:else}
        <!-- No add button in the empty list: the footer has it. -->
        <TotpList entries={filteredEntries} showProfileBadge={true} query={search.trim()} />
      {/if}

  {#snippet footer()}
    <button class="btn-primary" onclick={() => (showAdd = true)}>
      <Icon name="plus" size={13} />
      {$t('totp_btn_add')}
    </button>
  {/snippet}
</Frame>

{#if showAdd}
  <TotpAddModal
    initialTags={context === 'workspace' && contextId ? [`workspace:${contextId}`] : []}
    onclose={() => (showAdd = false)}
  />
{/if}

<style>
  .count-badge {
    background: var(--accent-tint);
    color: var(--accent-text-2);
    border: 1px solid var(--accent-tint-border);
    border-radius: var(--radius-sm);
    font-size: var(--fs-2xs);
    font-family: var(--font-mono);
    padding: 0.05rem 0.4rem;
    font-weight: var(--fw-semibold);
  }

  .panel-search {
    padding: var(--sp-3) var(--sp-4) 0;
    flex-shrink: 0;
  }

  .search-wrap {
    position: relative;
  }

  .search-icon {
    position: absolute;
    left: 0.6rem;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-2);
    pointer-events: none;
    display: flex;
  }

  .search-input {
    width: 100%;
    box-sizing: border-box;
    padding: 0.45rem 0.6rem 0.45rem var(--sp-8);
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-size: var(--fs-sm);
  }

  .search-input:focus {
    outline: none;
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--accent-bg);
  }

  .ws-chips {
    display: flex;
    gap: 0.35rem;
    padding: 0.6rem var(--sp-4);
    flex-wrap: wrap;
    flex-shrink: 0;
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

  .ws-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .loading-msg {
    text-align: center;
    padding: var(--sp-8);
    color: var(--text-2);
    font-size: var(--fs-sm);
  }


  .btn-primary {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--accent-grad);
    color: #fff;
    border: none;
    border-radius: var(--radius-field);
    height: 42px;
    padding: 0 var(--sp-4);
    font-size: 0.9rem;
    font-weight: var(--fw-semibold);
    box-shadow: var(--shadow-accent);
    cursor: pointer;
    width: 100%;
    justify-content: center;
    transition: filter var(--dur-fast);
  }

  .btn-primary:hover { filter: brightness(1.08); }
</style>

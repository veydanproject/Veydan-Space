<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { hasHome } from '$lib/core/mobile/nav';
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import { refocusSearch } from '$lib/core/ui/searchField';
  import Notice from '$lib/core/ui/Notice.svelte';
  import type { TotpEntry, TotpCode } from '$lib/pass/types';
  import { api } from '$lib/pass/api';
  import { copySecret } from '$lib/pass/clipboard';
  import { formatError, hasErrorCode } from '$lib/core/utils';
  import { onSyncChanged } from '$lib/core/mobile/api';
  import { directory } from '$lib/core/directory';
  import { userLabels } from '$lib/pass/totp-tags';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import TotpRow from '$lib/pass/mobile/TotpRow.svelte';
  import TotpKeyRows from '$lib/pass/mobile/TotpKeyRows.svelte';
  import WorkspaceFilter, { inWorkspace } from '$lib/pass/mobile/WorkspaceFilter.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';

  let entries = $state<TotpEntry[]>([]);
  // Unknown until the first answer: no «no codes yet» flash before the list (as MobilePasswordsPage).
  let loaded = $state(false);
  let codes = $state<Map<string, TotpCode>>(new Map());
  let profileNames = $state<Map<string, string>>(new Map());
  let workspaceNames = $state<Map<string, string>>(new Map());
  let tagColors = $state<Map<string, string>>(new Map());
  let error = $state('');
  let menu = $state<TotpEntry | null>(null);
  let search = $state('');
  /** «⋮» of the head: the filter by workspace (design 16); null shows every one. */
  let filterOpen = $state(false);
  let activeWorkspace = $state<string | null>(null);

  /** The same search as the Passwords tab: name, issuer and the labels of the chips; within the chosen workspace. */
  const filtered = $derived.by(() => {
    void workspaceNames;
    const ws = activeWorkspace;
    const shown = ws ? entries.filter((e) => inWorkspace(e.tags, ws)) : entries;
    const q = search.trim().toLowerCase();
    if (!q) return shown;
    return shown.filter((e) =>
      [e.name, e.issuer ?? '', ...entryChips(e).map((c) => c.label)].some((s) => s.toLowerCase().includes(q)),
    );
  });
  let tick: ReturnType<typeof setInterval>;

  function label(entry: TotpEntry): string {
    return entry.issuer || entry.name;
  }

  async function load() {
    try {
      entries = await api.totp.list();
      await refreshCodes();
    } catch (e) {
      error = formatError(e);
    } finally {
      loaded = true;
    }
    try {
      // Names and colours come through the catalog of entities; without their owner the names are the labels'.
      // Only the kinds the chips read are loaded: not the whole notes store.
      await Promise.all(['workspace', 'profile', 'note_tag'].map((kind) => directory.get(kind)?.ensureLoaded().catch(() => {})));
      profileNames = new Map(directory.list('profile').map((profile) => [profile.id, profile.name]));
      workspaceNames = new Map(directory.list('workspace').map((ws) => [ws.id, ws.name]));
      tagColors = new Map(directory.list('note_tag').map((tag) => [tag.name, tag.color]));
    } catch {
      /* names are optional; the codes still list */
    }
  }

  /** A profile: neutral, told by its mark; a workspace and a label in the user's colour where they have one. */
  function entryChips(entry: TotpEntry): { key: string; label: string; kind: string; color?: string }[] {
    const chips: { key: string; label: string; kind: string; color?: string }[] = [];
    for (const tag of entry.tags) {
      if (tag.startsWith('profile:')) {
        const id = tag.slice('profile:'.length);
        // Without the owner of the kind, the name the labels keep (10.3).
        const label = profileNames.get(id) ?? directory.linkName('profile', id);
        if (label) chips.push({ key: tag, label, kind: 'profile' });
      } else if (tag.startsWith('workspace:')) {
        const id = tag.slice('workspace:'.length);
        const label = workspaceNames.get(id) ?? directory.linkName('workspace', id);
        // In its colour where it has one (the owner's, or its label's without Space); neutral otherwise.
        if (label) chips.push({ key: tag, label, kind: 'workspace', color: directory.color('workspace', id) });
      }
    }
    for (const label of userLabels(entry.tags)) {
      chips.push({ key: label, label, kind: 'tag', color: tagColors.get(label) });
    }
    return chips;
  }

  async function refreshCodes() {
    if (entries.length === 0) return;
    try {
      const list = await api.totp.generateCodes(entries.map((e) => e.id));
      codes = new Map(list.map((c) => [c.id, c]));
    } catch (e) {
      // The lock closed between two ticks: the gate takes the page away, no error to show.
      if (!hasErrorCode(e, 'vault_locked')) error = formatError(e);
    }
  }


  async function copy(entry: TotpEntry) {
    const code = codes.get(entry.id);
    if (!code) return;
    await copySecret(code.code);
  }

  async function remove() {
    const entry = menu;
    menu = null;
    if (!entry || !(await ask({ title: $t('totp_delete_confirm', { name: entry.name }) }))) return;
    try {
      await api.totp.delete(entry.id);
      await load();
    } catch (e) {
      error = formatError(e);
    }
  }

  onMount(() => {
    load();
    tick = setInterval(refreshCodes, 1000);
    api.sync.trigger().catch(() => {});
    const unlisten = onSyncChanged(['totp'], load);
    return () => unlisten.then((f) => f());
  });
  onDestroy(() => {
    clearInterval(tick);
  });
</script>

<div class="m-page totp">
  <div class="m-header">
    {#if hasHome()}
      <a class="icon-btn" href="/" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    {/if}
    <h1 class="m-title">{$t('app_totp')}</h1>
    {#if workspaceNames.size}
      <button type="button" class="icon-btn" class:on={!!activeWorkspace} onclick={() => (filterOpen = true)} aria-label={$t('nav_more')}><Icon name="more-vertical" size={22} /></button>
    {/if}
  </div>

  {#if entries.length > 0}
    <div class="m-find-row">
      <div class="search-field fill">
        <Icon name="search" size={16} />
        <input type="search" bind:value={search} placeholder={$t('totp_search_placeholder')} aria-label={$t('totp_search_placeholder')} />
        {#if search}
          <button type="button" class="search-clear" onclick={(e) => { search = ''; refocusSearch(e); }} aria-label={$t('common_clear')}><Icon name="x" size={16} /></button>
        {/if}
      </div>
    </div>
  {/if}

  <div class="m-body">
  {#if error}
    <div class="err"><Notice tone="danger">{error}</Notice></div>
  {/if}

  {#if entries.length === 0 && !loaded}
    <!-- Not answered yet: nothing rather than «no codes». -->
  {:else if entries.length === 0}
    <div class="empty-state">
      <span class="empty-icon"><Icon name="shield-lock" size={22} /></span>
      <p>{$t('totp_empty')}</p>
      <span>{$t('totp_empty_hint')}</span>
      <a class="btn btn-primary cta" href="/totp/add"><Icon name="plus" size={18} />{$t('totp_btn_add')}</a>
    </div>
  {:else if filtered.length === 0}
    <div class="empty-state">
      <span class="empty-icon"><Icon name="search" size={22} /></span>
      <p>{$t('common_nothing_found')}</p>
      {#if search.trim()}<span>{$t('pass_nothing_found_hint', { q: search.trim() })}</span>{/if}
    </div>
  {:else}
    <div class="m-cards divided">
      {#each filtered as entry (entry.id)}
        <TotpRow
          {entry}
          code={codes.get(entry.id) ?? null}
          chips={entryChips(entry)}
          onopen={() => goto(`/totp/${entry.id}`)}
          oncopy={() => copy(entry)}
          onmenu={() => (menu = entry)}
        />
      {/each}
    </div>
  {/if}

  </div>
  <!-- The empty list has its own add, in the middle: the + would be a second one. -->
  {#if entries.length}
    <a class="m-fab" href="/totp/add" aria-label={$t('totp_btn_add')}><Icon name="plus" size={22} /></a>
  {/if}
</div>

<BottomSheet open={!!menu} title={menu ? label(menu) : ''} onclose={() => (menu = null)}>
  <div class="m-list">
    <button type="button" class="list-row dense" onclick={() => { if (menu) copy(menu); menu = null; }}>
      <Icon name="copy" size={20} /><span class="list-row-label">{$t('common_copy')}</span>
    </button>
    <button type="button" class="list-row dense" onclick={() => { const id = menu?.id; menu = null; if (id) goto(`/totp/${id}`); }}>
      <Icon name="pencil" size={20} /><span class="list-row-label">{$t('common_edit')}</span>
    </button>
    <button type="button" class="list-row dense" onclick={remove}>
      <Icon name="trash-2" size={20} /><span class="list-row-label danger">{$t('common_delete')}</span>
    </button>
  </div>
  <!-- The key, the computer's TotpSecretField as rows; «Сменить ключ» opens the entry at its new key. -->
  <!-- The id is taken before menu is emptied: {@const} of the block would read the null. -->
  {#if menu}
    <TotpKeyRows entryId={menu.id} onchange={() => { const id = menu?.id; menu = null; if (id) void goto(`/totp/${id}?key`); }} />
  {/if}
</BottomSheet>

<WorkspaceFilter
  open={filterOpen}
  title={$t('app_totp')}
  active={activeWorkspace}
  onpick={(ws) => { activeWorkspace = ws; filterOpen = false; }}
  onclose={() => (filterOpen = false)}
/>

<style>
  .err { margin-bottom: var(--sp-3); }
  .totp { display: flex; flex-direction: column; min-height: 100%; }
  /* Room under the last row for the floating «+» (.m-fab). */
  .m-cards { gap: var(--sp-0-5); --rows-gap: var(--sp-0-5); padding-bottom: var(--fab-room); }
  .danger { color: var(--danger-text); }
  .cta { margin-top: var(--sp-2); }
</style>

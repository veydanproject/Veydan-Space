<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { hasHome } from '$lib/core/mobile/nav';
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import type { TotpEntry, TotpCode } from '$lib/pass/types';
  import { api } from '$lib/pass/api';
  import { formatError } from '$lib/core/utils';
  import { onSyncChanged } from '$lib/core/mobile/api';
  import { directory } from '$lib/core/directory';
  import { userLabels } from '$lib/pass/totp-tags';
  import { t } from '$lib/core/mobile/i18n';
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import { longpress } from '$lib/core/mobile/longpress';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';

  let entries = $state<TotpEntry[]>([]);
  let codes = $state<Map<string, TotpCode>>(new Map());
  let profileNames = $state<Map<string, string>>(new Map());
  let workspaceNames = $state<Map<string, string>>(new Map());
  let tagColors = $state<Map<string, string>>(new Map());
  let error = $state('');
  let toast = $state('');
  let menu = $state<TotpEntry | null>(null);
  let search = $state('');

  /** The same search as the Passwords tab: name, issuer and the labels of the chips. */
  const filtered = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) =>
      [e.name, e.issuer ?? '', ...entryChips(e).map((c) => c.label)].some((s) => s.toLowerCase().includes(q)),
    );
  });
  let toastTimer: ReturnType<typeof setTimeout>;
  let clipTimer: ReturnType<typeof setTimeout>;
  let tick: ReturnType<typeof setInterval>;

  const RADIUS = 12;
  const CIRC = 2 * Math.PI * RADIUS;

  function offset(code: TotpCode | undefined, period: number): number {
    if (!code) return CIRC;
    return CIRC * (1 - Math.min(1, code.seconds_left / period));
  }

  function ringColor(code: TotpCode | undefined): string {
    if (!code || code.seconds_left <= 5) return 'var(--danger-text)';
    if (code.seconds_left <= 10) return 'var(--warn-text)';
    return 'var(--accent)';
  }

  function formatCode(code: string): string {
    const half = Math.ceil(code.length / 2);
    return `${code.slice(0, half)} ${code.slice(half)}`;
  }

  /** Stable colour per issuer from a string hash. */
  function avatarColor(s: string): string {
    let h = 0;
    for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return NAV_COLORS[h % NAV_COLORS.length];
  }

  function label(entry: TotpEntry): string {
    return entry.issuer || entry.name;
  }

  async function load() {
    try {
      entries = await api.totp.list();
      await refreshCodes();
    } catch (e) {
      error = formatError(e);
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

  function entryChips(entry: TotpEntry): { key: string; label: string; ws: boolean; color?: string }[] {
    const chips: { key: string; label: string; ws: boolean; color?: string }[] = [];
    for (const tag of entry.tags) {
      if (tag.startsWith('profile:')) {
        const id = tag.slice('profile:'.length);
        // Without the owner of the kind, the name the labels keep (10.3).
        const label = profileNames.get(id) ?? directory.linkName('profile', id);
        if (label) chips.push({ key: tag, label, ws: false });
      } else if (tag.startsWith('workspace:')) {
        const id = tag.slice('workspace:'.length);
        const label = workspaceNames.get(id) ?? directory.linkName('workspace', id);
        // In its color where it has one (the owner's, or its label's without Space); green otherwise.
        if (label) chips.push({ key: tag, label, ws: true, color: directory.color('workspace', id) });
      }
    }
    for (const label of userLabels(entry.tags)) {
      chips.push({ key: label, label, ws: false, color: tagColors.get(label) });
    }
    return chips;
  }

  async function refreshCodes() {
    if (entries.length === 0) return;
    try {
      const list = await api.totp.generateCodes(entries.map((e) => e.id));
      codes = new Map(list.map((c) => [c.id, c]));
    } catch (e) {
      error = formatError(e);
    }
  }

  function showToast(msg: string) {
    clearTimeout(toastTimer);
    toast = msg;
    toastTimer = setTimeout(() => (toast = ''), 1600);
  }

  async function copy(entry: TotpEntry) {
    const code = codes.get(entry.id);
    if (!code) return;
    await navigator.clipboard.writeText(code.code);
    showToast($t('totp_copied'));
    // Clear clipboard after 30s so the code does not linger.
    clearTimeout(clipTimer);
    clipTimer = setTimeout(() => navigator.clipboard.writeText('').catch(() => {}), 30_000);
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
    clearTimeout(toastTimer);
    clearTimeout(clipTimer);
  });
</script>

<div class="m-page totp">
  <div class="m-header">
    {#if hasHome()}
      <a class="m-ibtn" href="/" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    {/if}
    <h1 class="m-title">{$t('app_totp')}</h1>
    <a class="m-ibtn" href="/totp/add" aria-label={$t('totp_btn_add')}><Icon name="plus" size={24} /></a>
  </div>

  {#if entries.length > 0}
    <div class="m-search">
      <div class="field">
        <Icon name="search" size={16} />
        <input type="search" bind:value={search} placeholder={$t('totp_search_placeholder')} />
      </div>
    </div>
  {/if}

  <div class="m-body">
  {#if error}
    <div class="m-error">{error}</div>
  {/if}

  {#if entries.length === 0}
    <div class="m-empty">
      <Icon name="shield" size={40} />
      <p>{$t('totp_empty')}</p>
      <span>{$t('totp_empty_hint')}</span>
      <a class="m-btn-grad cta" href="/totp/add"><Icon name="plus" size={18} />{$t('totp_btn_add')}</a>
    </div>
  {:else if filtered.length === 0}
    <div class="m-empty">
      <Icon name="search" size={40} />
      <p>{$t('pass_nothing_found')}</p>
      <span>{$t('pass_nothing_found_hint', { q: search.trim() })}</span>
    </div>
  {:else}
    <div class="m-cards">
      {#each filtered as entry (entry.id)}
        {@const code = codes.get(entry.id)}
        <button class="m-card item" onclick={() => copy(entry)} {@attach longpress(() => (menu = entry))}>
          <span class="m-avatar" style:--c={avatarColor(label(entry))}>{label(entry).charAt(0).toUpperCase()}</span>
          <span class="meta">
            <span class="name">{label(entry)}</span>
            {#if entry.issuer}<span class="account">{entry.name}</span>{/if}
            {#if entryChips(entry).length}
              <span class="tagline">
                {#each entryChips(entry) as chip (chip.key)}
                  <span class="m-chip small tinted" style:--chip={chip.color || (chip.ws ? 'var(--success)' : undefined)}>{chip.label}</span>
                {/each}
              </span>
            {/if}
          </span>
          <span class="code">{code ? formatCode(code.code) : '••• •••'}</span>
          <svg class="ring" viewBox="0 0 28 28" width="24" height="24">
            <circle cx="14" cy="14" r={RADIUS} class="track" />
            <circle
              cx="14"
              cy="14"
              r={RADIUS}
              class="progress"
              style="stroke: {ringColor(code)}; stroke-dasharray: {CIRC}; stroke-dashoffset: {offset(code, entry.period)}"
            />
          </svg>
        </button>
      {/each}
    </div>
  {/if}

  {#if toast}
    <div class="toast">{toast}</div>
  {/if}
  </div>
</div>

<BottomSheet open={!!menu} title={menu ? label(menu) : ''} onclose={() => (menu = null)}>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => { if (menu) copy(menu); menu = null; }}>
      <Icon name="copy" size={20} /><span class="m-row-label">{$t('pwgen_btn_copy')}</span>
    </button>
    <button type="button" class="m-row" onclick={() => { const id = menu?.id; menu = null; if (id) goto(`/totp/${id}`); }}>
      <Icon name="pencil" size={20} /><span class="m-row-label">{$t('totp_edit')}</span>
    </button>
    <button type="button" class="m-row" onclick={remove}>
      <Icon name="trash-2" size={20} /><span class="m-row-label danger">{$t('totp_delete')}</span>
    </button>
  </div>
</BottomSheet>

<style>
  .totp { display: flex; flex-direction: column; min-height: 100%; }
  .item {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 64px;
    padding: var(--sp-3) var(--sp-4);
    color: var(--text);
    font: inherit;
    text-align: left;
    width: 100%;
  }
  .item:active { background: var(--surface-2); }
  .meta { flex: 1; display: flex; flex-direction: column; min-width: 0; gap: 1px; }
  .name { font-size: 15px; font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .account { font-size: 13px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tagline { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .code {
    font-family: var(--font-mono);
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--text);
    white-space: nowrap;
  }
  .ring { transform: rotate(-90deg); flex-shrink: 0; }
  .ring circle { fill: none; stroke-width: 3; }
  .track { stroke: var(--border); }
  .progress { transition: stroke-dashoffset 0.9s linear; }
  .m-cards { padding-bottom: var(--sp-6); }
  .danger { color: var(--danger-text); }
  .cta { width: auto; margin-top: var(--sp-2); padding: 0 var(--sp-5); }
</style>

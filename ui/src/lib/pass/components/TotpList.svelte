<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/pass/api';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { directory } from '$lib/core/directory';
  import type { TotpEntry, TotpCode } from '$lib/pass/types';
  import Icon from '$lib/core/Icon.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import TotpAddModal from './TotpAddModal.svelte';
  import { userLabels } from '$lib/pass/totp-tags';

  interface Props {
    entries: TotpEntry[];
    showProfileBadge?: boolean;
    onrequestAdd?: () => void;
    emptyText?: string;
    /** The search the entries are filtered by: an empty result then says nothing matched, not that there are none. */
    query?: string;
  }

  let { entries, showProfileBadge = false, onrequestAdd, emptyText, query = '' }: Props = $props();

  let codes = $state<Map<string, TotpCode>>(new Map());
  let copiedId = $state('');
  let copyTimer: ReturnType<typeof setTimeout>;
  let clipClearTimer: ReturnType<typeof setTimeout>;
  let editing = $state<TotpEntry | null>(null);

  // Countdown ring
  const RADIUS = 10;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

  function strokeOffset(secondsLeft: number, period: number): number {
    const frac = Math.max(0, Math.min(1, secondsLeft / period));
    return CIRCUMFERENCE * (1 - frac);
  }

  function ringColor(secondsLeft: number): string {
    if (secondsLeft <= 5) return 'var(--danger-text)';
    if (secondsLeft <= 10) return 'var(--warn-text)';
    return 'var(--accent)';
  }

  async function loadCodes() {
    if (entries.length === 0) return;
    const ids = entries.map((e) => e.id);
    try {
      const results = await api.totp.generateCodes(ids);
      const map = new Map<string, TotpCode>();
      for (const r of results) map.set(r.id, r);
      codes = map;
    } catch {}
  }

  let interval: ReturnType<typeof setInterval>;

  onMount(async () => {
    await loadCodes();
    interval = setInterval(loadCodes, 1000);
  });

  onDestroy(() => {
    clearInterval(interval);
    clearTimeout(copyTimer);
    clearTimeout(clipClearTimer);
  });

  $effect(() => {
    // Reload when entries change
    entries;
    loadCodes();
  });

  async function copy(entry: TotpEntry) {
    const code = codes.get(entry.id);
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code.code);
      copiedId = '';
      requestAnimationFrame(() => (copiedId = entry.id));
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copiedId = ''), 2000);

      // Auto-clear clipboard after 30s
      clearTimeout(clipClearTimer);
      clipClearTimer = setTimeout(() => {
        navigator.clipboard.writeText('').catch(() => {});
      }, 30_000);
    } catch {}
  }

  function profileName(tags: string[]): string | null {
    for (const tag of tags) {
      if (tag.startsWith('profile:')) {
        const pid = tag.slice('profile:'.length);
        return directory.linkName('profile', pid) ?? null;
      }
    }
    return null;
  }

  async function remove(entry: TotpEntry) {
    if (!(await ask({ title: $t('totp_delete_confirm', { name: entry.name }) }))) return;
    await api.totp.delete(entry.id);
    await totpStore.refresh();
  }

  function formatCode(code: string): string {
    // Split 6-digit code as "123 456", 8-digit as "1234 5678"
    if (code.length === 6) return `${code.slice(0, 3)} ${code.slice(3)}`;
    if (code.length === 8) return `${code.slice(0, 4)} ${code.slice(4)}`;
    return code;
  }
</script>

{#if entries.length === 0}
  <div class="empty-state">
    {#if query}
      <span class="empty-icon"><Icon name="search" size={22} /></span>
      <p>{$t('pass_nothing_found')}</p>
      <span>{$t('pass_nothing_found_hint', { q: query })}</span>
    {:else}
      <span class="empty-icon"><Icon name="shield-off" size={22} /></span>
      <p>{emptyText ?? $t('totp_empty')}</p>
      {#if onrequestAdd}
        <button class="btn btn-ghost btn-sm" onclick={onrequestAdd}>
          <Icon name="plus" size={13} /> {$t('totp_btn_add')}
        </button>
      {/if}
    {/if}
  </div>
{:else}
  <!-- A container: a row lays itself out by the width of its list (a 300px pane or a 440px drawer). -->
  <div class="list">
    {#each entries as entry (entry.id)}
      {@const code = codes.get(entry.id)}
      {@const pname = showProfileBadge ? profileName(entry.tags) : null}
      {@const labels = userLabels(entry.tags)}
      <div class="entry">
        <div class="entry-info">
          <div class="entry-name" title={entry.name}>{entry.name}</div>
          {#if entry.issuer}
            <div class="entry-issuer">{entry.issuer}</div>
          {/if}
          {#if pname || labels.length > 0}
            <div class="entry-labels">
              {#if pname}<span class="profile-badge">{pname}</span>{/if}
              {#each labels as label (label)}
                <span class="label-badge">{label}</span>
              {/each}
            </div>
          {/if}
        </div>

        <div class="entry-code">
          <svg class="ring" width="28" height="28" viewBox="-2 -2 28 28" aria-hidden="true">
            <circle cx="12" cy="12" r={RADIUS} fill="none" stroke="var(--border)" stroke-width="2.5" />
            {#if code}
              <circle
                cx="12" cy="12" r={RADIUS}
                fill="none"
                stroke={ringColor(code.seconds_left)}
                stroke-width="2.5"
                stroke-dasharray={CIRCUMFERENCE}
                stroke-dashoffset={strokeOffset(code.seconds_left, entry.period)}
                stroke-linecap="round"
                transform="rotate(-90 12 12)"
                style="transition: stroke-dashoffset 0.9s linear, stroke 0.3s"
              />
            {/if}
          </svg>
          <span
            class="code-value"
            class:copied={copiedId === entry.id}
            style:color={copiedId === entry.id ? 'var(--success-text)' : code ? ringColor(code.seconds_left) : undefined}
          >
            {code ? formatCode(code.code) : '••• •••'}
          </span>
        </div>

        <div class="entry-actions">
          <button
            class="icon-btn"
            class:success={copiedId === entry.id}
            onclick={() => copy(entry)}
            title={$t('totp_copy')}
            aria-label={$t('totp_copy')}
            disabled={!code}
          >
            <Icon name={copiedId === entry.id ? 'check' : 'copy'} size={13} />
          </button>
          <button class="icon-btn" onclick={() => (editing = entry)} title={$t('totp_edit')} aria-label={$t('totp_edit')}>
            <Icon name="pencil" size={13} />
          </button>
          <button
            class="icon-btn danger-soft"
            onclick={() => remove(entry)}
            title={$t('common_delete')}
            aria-label={$t('common_delete')}
          >
            <Icon name="trash" size={13} />
          </button>
        </div>
      </div>
    {/each}
  </div>
{/if}

{#if editing}
  <TotpAddModal entry={editing} onclose={() => (editing = null)} />
{/if}

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    container-type: inline-size;
  }

  .entry {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: 0.875rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    transition: border-color var(--dur-fast);
  }

  .entry:hover { border-color: var(--border-2); }

  .entry-info {
    flex: 1;
    min-width: 0;
  }

  /* One line: a long account name ends in an ellipsis (the whole name is the tooltip). */
  .entry-name {
    font-size: var(--fs-base);
    font-weight: var(--fw-semibold);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry-issuer {
    font-size: var(--fs-xs);
    color: var(--text-2);
    margin-top: 0.1rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry-labels {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin-top: 0.25rem;
  }

  .label-badge {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-2);
    font-size: var(--fs-2xs);
    padding: 0.05rem 0.35rem;
  }

  .profile-badge {
    background: var(--accent-tint);
    border: 1px solid var(--accent-tint-border);
    color: var(--accent-text-2);
    border-radius: var(--radius-sm);
    font-size: var(--fs-2xs);
    padding: 0.1rem 0.4rem;
    font-weight: var(--fw-medium);
  }

  .entry-code {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-shrink: 0;
  }

  .ring {
    flex-shrink: 0;
  }

  .code-value {
    font-family: var(--font-mono);
    font-size: var(--fs-xl);
    font-weight: var(--fw-bold);
    letter-spacing: 0.08em;
    min-width: 5.5ch;
    text-align: center;
    white-space: nowrap;
  }

  .code-value.copied { animation: code-pop 0.4s ease; }
  .icon-btn.success { animation: code-pop 0.35s ease; }

  @keyframes code-pop {
    0% { transform: scale(1); }
    40% { transform: scale(1.14); }
    100% { transform: scale(1); }
  }

  .entry-actions {
    display: flex;
    gap: var(--sp-1);
    flex-shrink: 0;
  }

  /* A narrow list (a pane of the Pass page): the name, the issuer and the
     labels take the first line in full; the code and the actions the second. */
  @container (max-width: 380px) {
    .entry {
      flex-wrap: wrap;
      row-gap: var(--sp-2);
    }
    .entry-info { flex: 1 0 100%; }
    .code-value { font-size: var(--fs-lg); min-width: 0; }
    .entry-actions { margin-left: auto; }
  }

  /* .icon-btn / .empty-state / .btn are global primitives (base.css) */
</style>

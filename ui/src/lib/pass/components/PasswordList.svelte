<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/pass/api';
  import { t } from '$lib/core/i18n';
  import { parseBinding } from '$lib/core/bindings';
  import { userLabels } from '$lib/core/entity-tags';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { directory } from '$lib/core/directory';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';
  import type { PasswordEntry } from '$lib/pass/types';

  interface Props {
    entries: PasswordEntry[];
    onopen: (id: string) => void;
    onedit?: (entry: PasswordEntry) => void;
  }

  let { entries, onopen, onedit }: Props = $props();

  let copied = $state('');
  let error = $state('');
  let unlockOpen = $state(false);
  let pending = $state<PasswordEntry | null>(null);
  let copyTimer: ReturnType<typeof setTimeout>;
  /** How many chips a row shows; the rest is one counter chip with their names as the tooltip. */
  const MAX_CHIPS = 2;

  onMount(() => {
    void directory.ensureLoaded();
    void totpStore.ensureLoaded();
    return () => clearTimeout(copyTimer);
  });

  function profilesOf(tags: string[]): string[] {
    return tags.flatMap((tag) => {
      const parsed = parseBinding(tag);
      if (parsed?.kind !== 'profile') return [];
      // The owner's name, or the label of a profile of Space where the browser is absent (10.3).
      const name = directory.linkName('profile', parsed.value);
      return name ? [name] : [];
    });
  }

  function notesOf(tags: string[]): string[] {
    return tags.flatMap((tag) => {
      const parsed = parseBinding(tag);
      if (parsed?.kind !== 'note') return [];
      const title = directory.linkName('note', parsed.value);
      return title ? [title] : [];
    });
  }
  function labelColor(name: string): string | undefined {
    return directory.summary('note_tag', name)?.color;
  }

  function markCopied(key: string) {
    copied = '';
    requestAnimationFrame(() => (copied = key));
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = ''), 2000);
  }

  async function copyUsername(entry: PasswordEntry) {
    if (!entry.username) return;
    error = '';
    try {
      await navigator.clipboard.writeText(entry.username);
      markCopied(`${entry.id}:user`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function askUnlock(entry: PasswordEntry) {
    pending = entry;
    unlockOpen = true;
  }

  async function copyPassword(entry: PasswordEntry) {
    if (appLock.locked) {
      askUnlock(entry);
      return;
    }
    error = '';
    try {
      await api.passwords.copy(entry.id);
      markCopied(`${entry.id}:pass`);
    } catch (e) {
      const code = e && typeof e === 'object' && 'code' in e ? String((e as { code: string }).code) : '';
      if (code === 'vault_locked' || code === 'vault_mismatch' || code === 'decrypt_failed') {
        askUnlock(entry);
        return;
      }
      try {
        const value = (await api.passwords.reveal(entry.id, 'password')).value;
        await navigator.clipboard.writeText(value);
        markCopied(`${entry.id}:pass`);
        const secret = value;
        setTimeout(() => {
          navigator.clipboard.readText().then((current) => {
            if (current === secret) void navigator.clipboard.writeText('');
          }).catch(() => {});
        }, 30_000);
      } catch (err) {
        error = err instanceof Error ? err.message : String(err);
      }
    }
  }
</script>

{#if error}<p class="err">{error}</p>{/if}
<div class="list">
  {#each entries as entry (entry.id)}
    {@const profiles = profilesOf(entry.tags)}
    {@const labels = [...userLabels(entry.tags), ...notesOf(entry.tags)]}
    {@const firstTotp = entry.totp_ids[0]}
    {@const totp = firstTotp ? totpStore.list.find((item) => item.id === firstTotp) : undefined}
    <div class="entry">
      <button type="button" class="entry-info" onclick={() => onopen(entry.id)}>
        <div class="entry-name">{entry.title}</div>
        {#if entry.username || entry.url}
          <div class="entry-sub">{entry.username || entry.url}</div>
        {/if}
        {#if profiles.length || labels.length}
          {@const chips = [...profiles.map((name) => ({ name, profile: true })), ...labels.map((name) => ({ name, profile: false }))]}
          {@const rest = chips.slice(MAX_CHIPS)}
          <!-- Whole chips that wrap, never one cut in the middle; past two, a counter. -->
          <div class="entry-labels">
            {#each chips.slice(0, MAX_CHIPS) as chip, i (i)}
              {#if chip.profile}
                <span class="profile-badge">{chip.name}</span>
              {:else}
                <span class="label-badge tinted" style:--chip={labelColor(chip.name)}>{chip.name}</span>
              {/if}
            {/each}
            {#if rest.length}
              <span class="label-badge tinted" title={rest.map((c) => c.name).join(', ')}>+{rest.length}</span>
            {/if}
          </div>
        {/if}
      </button>
      {#if firstTotp}
        <TotpLiveCode entryId={firstTotp} period={totp?.period ?? 30} copiedLabel={$t('totp_copy')} compact />
        {#if entry.totp_ids.length > 1}<span class="totp-more">+{entry.totp_ids.length - 1}</span>{/if}
      {/if}
      <div class="entry-actions">
        {#if entry.username}
          <button
            type="button"
            class="icon-btn"
            class:success={copied === `${entry.id}:user`}
            title={$t('ctx_action_copy_username')}
            onclick={() => copyUsername(entry)}
          >
            <Icon name={copied === `${entry.id}:user` ? 'check' : 'user'} size={13} />
          </button>
        {/if}
        <button
          type="button"
          class="icon-btn"
          class:success={copied === `${entry.id}:pass`}
          title={appLock.locked ? $t('lock_title') : $t('ctx_action_copy_password')}
          onclick={() => copyPassword(entry)}
        >
          <Icon name={copied === `${entry.id}:pass` ? 'check' : appLock.locked ? 'lock' : 'copy'} size={13} />
        </button>
        {#if onedit && !appLock.locked}
          <button type="button" class="icon-btn" title={$t('pw_btn_edit')} onclick={() => onedit(entry)}>
            <Icon name="pencil" size={13} />
          </button>
        {/if}
      </div>
    </div>
  {/each}
</div>

<VaultUnlock bind:open={unlockOpen} onunlocked={() => { if (pending) void copyPassword(pending); }} />

<style>
  .list { display: flex; flex-direction: column; gap: 0.4rem; }
  .entry {
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    gap: var(--sp-2);
    min-height: 64px;
    padding: 0.55rem 0.75rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  .entry:hover { border-color: var(--border-2); }
  .entry-info {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .entry-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-base);
    font-weight: var(--fw-semibold);
  }
  .entry-sub {
    margin-top: 0.1rem;
    font-size: var(--fs-xs);
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry-labels {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin-top: 0.25rem;
    min-width: 0;
  }
  .label-badge {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    border-width: 1px;
    border-style: solid;
    border-radius: var(--radius-sm);
    font-size: var(--fs-2xs);
    padding: 0.05rem 0.35rem;
    white-space: nowrap;
  }
  .profile-badge {
    flex-shrink: 0;
    background: var(--accent-tint);
    border: 1px solid var(--accent-tint-border);
    color: var(--accent-text-2);
    border-radius: var(--radius-sm);
    font-size: var(--fs-2xs);
    padding: 0.1rem 0.4rem;
    font-weight: var(--fw-medium);
    white-space: nowrap;
  }
  .entry-actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }
  .totp-more { font-size: var(--fs-2xs); color: var(--text-2); flex-shrink: 0; }
  .icon-btn.success { color: var(--success-text); }
  .err { margin: 0 0 var(--sp-2); color: var(--danger-text); font-size: var(--fs-xs); }
</style>

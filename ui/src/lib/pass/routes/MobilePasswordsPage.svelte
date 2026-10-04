<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's password list: one finger-size row per password (mobile/PasswordRow),
  a tap opens it, a long press offers copy, edit and delete in a sheet.
-->
<script lang="ts">
  import { hasHome } from '$lib/core/mobile/nav';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import PasswordRow from '$lib/pass/mobile/PasswordRow.svelte';
  import PasswordLockBadge from '$lib/pass/components/PasswordLockBadge.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';
  import { api } from '$lib/pass/api';
  import { formatError, hasErrorCode } from '$lib/core/utils';
  import { onSyncChanged } from '$lib/core/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { directory } from '$lib/core/directory';
  import { userLabels } from '$lib/core/entity-tags';
  import { parseBinding } from '$lib/core/bindings';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import type { PasswordEntry } from '$lib/pass/types';

  let entries = $state<PasswordEntry[]>([]);
  let loaded = $state(false);
  let search = $state('');
  let error = $state('');
  let toast = $state('');
  let menu = $state<PasswordEntry | null>(null);
  let unlockOpen = $state(false);
  let pendingCopy = $state<PasswordEntry | null>(null);
  let toastTimer: ReturnType<typeof setTimeout>;

  const canAdd = $derived(!appLock.locked && appLock.status.vault !== 'mismatch');

  async function load() {
    try {
      entries = await api.passwords.list();
    } catch (e) {
      error = formatError(e);
    } finally {
      loaded = true;
    }
  }

  /** The names of the entities an entry links to; a link nothing here can name is left out of the row. */
  function linkedNames(entry: PasswordEntry): string[] {
    return entry.tags.flatMap((tag) => {
      const parsed = parseBinding(tag);
      if (!parsed || (parsed.kind !== 'profile' && parsed.kind !== 'note')) return [];
      // The owner's name, or the label of an entity of Space where its module is absent (10.3).
      const name = directory.linkName(parsed.kind, parsed.value);
      return name ? [name] : [];
    });
  }

  const chipsOf = (entry: PasswordEntry) => [...userLabels(entry.tags), ...linkedNames(entry)];

  const filtered = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((entry) =>
      `${entry.title} ${entry.username ?? ''} ${entry.url ?? ''} ${chipsOf(entry).join(' ')}`.toLowerCase().includes(q),
    );
  });

  const period = (entry: PasswordEntry) => totpStore.list.find((item) => item.id === entry.totp_ids[0])?.period ?? 30;

  function showToast(text: string) {
    clearTimeout(toastTimer);
    toast = text;
    toastTimer = setTimeout(() => (toast = ''), 1600);
  }

  async function writeClipboard(value: string) {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      const el = document.createElement('textarea');
      el.value = value;
      el.setAttribute('readonly', '');
      el.style.position = 'fixed';
      el.style.left = '-9999px';
      document.body.appendChild(el);
      el.select();
      document.execCommand('copy');
      el.remove();
    }
  }

  async function copyUsername(entry: PasswordEntry) {
    menu = null;
    if (!entry.username) return;
    await writeClipboard(entry.username);
    showToast($t('pw_copied'));
  }

  async function copyPassword(entry: PasswordEntry) {
    menu = null;
    if (appLock.locked) {
      pendingCopy = entry;
      unlockOpen = true;
      return;
    }
    try {
      const value = (await api.passwords.reveal(entry.id, 'password')).value;
      await writeClipboard(value);
      showToast($t('pw_copied'));
      // The clipboard forgets the password after 30 s, unless something else was copied since.
      setTimeout(() => {
        navigator.clipboard.readText().then((current) => {
          if (current === value) void navigator.clipboard.writeText('');
        }).catch(() => {});
      }, 30_000);
    } catch (e) {
      if (hasErrorCode(e, 'vault_locked') || hasErrorCode(e, 'vault_mismatch') || hasErrorCode(e, 'decrypt_failed')) {
        pendingCopy = entry;
        unlockOpen = true;
        return;
      }
      error = formatError(e);
    }
  }

  async function remove(entry: PasswordEntry) {
    menu = null;
    if (!(await ask({ title: $t('pw_delete_confirm', { name: entry.title }) }))) return;
    try {
      await api.passwords.delete(entry.id);
      await passwordStore.refresh().catch(() => {});
      await load();
    } catch (e) {
      error = formatError(e);
    }
  }

  onMount(() => {
    void appLock.refresh();
    void appLock.listen();
    void totpStore.ensureLoaded();
    void directory.get('profile')?.ensureLoaded();
    void load();
    const unlisten = onSyncChanged(['password', 'password_vault'], () => void load());
    return () => {
      clearTimeout(toastTimer);
      void unlisten.then((fn) => fn());
    };
  });
</script>

<div class="m-page">
  <div class="m-header">
    {#if hasHome()}
      <a class="m-ibtn" href="/" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    {/if}
    <h1 class="m-title title-with-badge">{$t('pw_title')}<PasswordLockBadge size={11} /></h1>
    {#if canAdd}
      <a class="m-ibtn" href="/passwords/add" aria-label={$t('cmd_passwords_create')}><Icon name="plus" size={24} /></a>
    {/if}
  </div>
  <div class="m-search">
    <div class="field">
      <Icon name="search" size={16} />
      <input type="search" bind:value={search} placeholder={$t('pw_search')} />
    </div>
  </div>
  <div class="m-body">
    {#if error}<div class="m-error">{error}</div>{/if}
    {#if !loaded}
      <!-- Nothing yet: no empty-state flash before the list arrives. -->
    {:else if entries.length === 0}
      <div class="m-empty">
        <Icon name="lock" size={40} />
        <p>{$t('pw_empty')}</p>
        {#if canAdd}
          <a class="m-btn-grad cta" href="/passwords/add"><Icon name="plus" size={18} />{$t('cmd_passwords_create')}</a>
        {/if}
      </div>
    {:else if filtered.length === 0}
      <div class="m-empty">
        <Icon name="search" size={40} />
        <p>{$t('pass_nothing_found')}</p>
        <span>{$t('pass_nothing_found_hint', { q: search.trim() })}</span>
      </div>
    {:else}
      <div class="m-cards list">
        {#each filtered as entry (entry.id)}
          <PasswordRow
            {entry}
            chips={chipsOf(entry)}
            period={period(entry)}
            onopen={() => goto(`/passwords/${entry.id}`)}
            onmenu={() => (menu = entry)}
          />
        {/each}
      </div>
    {/if}
    {#if toast}<div class="toast">{toast}</div>{/if}
  </div>
</div>

<BottomSheet open={!!menu} title={menu?.title ?? ''} onclose={() => (menu = null)}>
  {#if menu}
    {@const entry = menu}
    <div class="m-list">
      {#if entry.username}
        <button type="button" class="m-row" onclick={() => copyUsername(entry)}>
          <Icon name="user" size={20} /><span class="m-row-label">{$t('ctx_action_copy_username')}</span>
        </button>
      {/if}
      <button type="button" class="m-row" onclick={() => copyPassword(entry)}>
        <Icon name={appLock.locked ? 'lock' : 'copy'} size={20} /><span class="m-row-label">{$t('ctx_action_copy_password')}</span>
      </button>
      {#if canAdd}
        <button type="button" class="m-row" onclick={() => { menu = null; void goto(`/passwords/${entry.id}?edit`); }}>
          <Icon name="pencil" size={20} /><span class="m-row-label">{$t('pw_btn_edit')}</span>
        </button>
        <button type="button" class="m-row" onclick={() => remove(entry)}>
          <Icon name="trash-2" size={20} /><span class="m-row-label danger">{$t('pw_btn_delete')}</span>
        </button>
      {/if}
    </div>
  {/if}
</BottomSheet>

<VaultUnlock
  bind:open={unlockOpen}
  onunlocked={() => {
    const entry = pendingCopy;
    pendingCopy = null;
    if (entry) void copyPassword(entry);
  }}
/>

<style>
  .title-with-badge { display: flex; align-items: center; gap: 2px; }
  .list { padding-bottom: var(--sp-6); }
  .cta { width: auto; margin-top: var(--sp-2); padding: 0 var(--sp-5); }
  .danger { color: var(--danger-text); }
</style>

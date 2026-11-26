<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's password list: one finger-size row per password (mobile/PasswordRow),
  a tap opens it, a long press offers copy, edit and delete in a sheet. In a
  wide window it is the column beside the open password (core/ui/split).
-->
<script lang="ts">
  import { toast } from '$lib/core/ui/toast.svelte';
  import { hasHome } from '$lib/core/mobile/nav';
  import { split } from '$lib/core/ui/split.svelte';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import PasswordRow from '$lib/pass/mobile/PasswordRow.svelte';
  import PasswordMenuHead from '$lib/pass/mobile/PasswordMenuHead.svelte';
  import HubButton from '$lib/core/ui/HubButton.svelte';
  import PasswordLockBadge from '$lib/pass/components/PasswordLockBadge.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';
  import { api } from '$lib/pass/api';
  import { copySecret } from '$lib/pass/clipboard';
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

  // The store's list: what the open password beside it saves or deletes shows here at once.
  const entries = $derived(passwordStore.list);
  let loaded = $state(passwordStore.loaded);
  let search = $state('');
  let error = $state('');
  let menu = $state<PasswordEntry | null>(null);

  // The id is read before the sheet closes: «menu = null» re-runs {@const entry = menu}.
  function edit(entry: PasswordEntry) {
    const id = entry.id;
    menu = null;
    void split.open(`/passwords/${id}?edit`);
  }
  let unlockOpen = $state(false);
  let pendingCopy = $state<PasswordEntry | null>(null);

  const canAdd = $derived(!appLock.locked && appLock.status.vault !== 'mismatch');

  /** A long press's sheet (DS2-31): the copies, «Изменить», a line, the red «Удалить». */
  const menuItems = $derived.by((): MenuEntry[] => {
    const entry = menu;
    if (!entry) return [];
    const out: MenuEntry[] = [];
    if (entry.username) out.push({ label: $t('ctx_action_copy_username'), icon: 'id', onselect: () => void copyUsername(entry) });
    out.push({ label: $t('ctx_action_copy_password'), icon: appLock.locked ? 'lock-square-rounded' : 'copy', onselect: () => void copyPassword(entry) });
    if (canAdd) {
      out.push(
        { label: $t('common_edit'), icon: 'pencil', onselect: () => edit(entry) },
        { type: 'separator' },
        { label: $t('common_delete'), icon: 'trash-2', danger: true, onselect: () => void remove(entry) },
      );
    }
    return out;
  });

  async function load() {
    try {
      await passwordStore.refresh();
    } catch (e) {
      error = formatError(e);
    } finally {
      loaded = true;
    }
  }

  /** The entities an entry links to, neutral; a link nothing here can name is left out of the row. */
  function linkedChips(entry: PasswordEntry): { label: string; kind: string }[] {
    return entry.tags.flatMap((tag) => {
      const parsed = parseBinding(tag);
      if (!parsed || (parsed.kind !== 'profile' && parsed.kind !== 'note')) return [];
      // The owner's name, or the label of an entity of Space where its module is absent (10.3).
      const name = directory.linkName(parsed.kind, parsed.value);
      return name ? [{ label: name, kind: parsed.kind }] : [];
    });
  }

  /** A row's chips: its labels in the colour the user gave them, then the linked entities. */
  const chipItemsOf = (entry: PasswordEntry) => [
    ...userLabels(entry.tags).map((label) => ({ label, kind: 'tag', color: directory.summary('note_tag', label)?.color || undefined })),
    ...linkedChips(entry),
  ];
  const chipsOf = (entry: PasswordEntry) => chipItemsOf(entry).map((c) => c.label);

  const filtered = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((entry) =>
      `${entry.title} ${entry.username ?? ''} ${entry.url ?? ''} ${chipsOf(entry).join(' ')}`.toLowerCase().includes(q),
    );
  });

  const period = (entry: PasswordEntry) => totpStore.list.find((item) => item.id === entry.totp_ids[0])?.period ?? 30;


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
    toast.success($t('toast_copied'));
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
      await copySecret(value, { toast: $t('toast_copied') });
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
      // The password open beside the list goes with it.
      if (split.shows(`/passwords/${entry.id}`)) void goto('/passwords', { replaceState: true });
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
    // The colours of the labels (only the tags, not the whole notes store).
    void directory.get('note_tag')?.ensureLoaded();
    void load();
    const unlisten = onSyncChanged(['password', 'password_vault'], () => void load());
    return () => {
      void unlisten.then((fn) => fn());
    };
  });
</script>

<div class="m-page">
  <div class="m-header">
    {#if hasHome()}
      <HubButton />
    {/if}
    <h1 class="m-title">{$t('pw_title')}</h1>
    <PasswordLockBadge size={20} />
  </div>
  <div class="m-search">
    <div class="field">
      <Icon name="search" size={16} />
      <input type="search" bind:value={search} placeholder={$t('pw_search')} />
    </div>
  </div>
  <div class="m-body">
    {#if error}<div class="err"><Notice tone="danger">{error}</Notice></div>{/if}
    {#if !loaded}
      <!-- The shape of the rows, no empty-state flash before the list arrives. -->
      <Skeleton height={56} avatar="square" />
    {:else if entries.length === 0}
      <div class="m-empty">
        <Icon name="lock-password" size={40} />
        <p>{$t('pw_empty')}</p>
        {#if canAdd}
          <a class="m-btn-grad cta" href="/passwords/add" onclick={split.follow}><Icon name="plus" size={18} />{$t('cmd_passwords_create')}</a>
        {/if}
      </div>
    {:else if filtered.length === 0}
      <div class="m-empty">
        <Icon name="search" size={40} />
        <p>{$t('common_nothing_found')}</p>
        <span>{$t('pass_nothing_found_hint', { q: search.trim() })}</span>
      </div>
    {:else}
      <div class="m-cards list divided">
        {#each filtered as entry (entry.id)}
          <PasswordRow
            {entry}
            chips={chipItemsOf(entry)}
            period={period(entry)}
            active={split.shows(`/passwords/${entry.id}`)}
            onopen={() => split.open(`/passwords/${entry.id}`)}
            onmenu={() => (menu = entry)}
          />
        {/each}
      </div>
    {/if}
  </div>
  {#if canAdd}
    <a class="m-fab" href="/passwords/add" onclick={split.follow} aria-label={$t('cmd_passwords_create')}><Icon name="plus" size={22} /></a>
  {/if}
</div>

{#snippet menuHead()}{#if menu}<PasswordMenuHead entry={menu} />{/if}{/snippet}
<ContextMenu open={!!menu} x={0} y={0} items={menuItems} header={menuHead} onclose={() => (menu = null)} />

<VaultUnlock
  bind:open={unlockOpen}
  onunlocked={() => {
    const entry = pendingCopy;
    pendingCopy = null;
    if (entry) void copyPassword(entry);
  }}
/>

<style>
  /* The lock is a touch target of 40 at the head's right edge, its icon 20. */
  .m-header :global(.badge) { width: 40px; height: 40px; padding: 0; margin-left: auto; flex-shrink: 0; }
  .err { margin-bottom: var(--sp-3); }
  /* Room under the last row for the floating «+» (.m-fab). */
  .list { gap: 2px; --rows-gap: 2px; padding-bottom: calc(52px + var(--fab-lift) + var(--sp-2)); }
  .cta { width: auto; margin-top: var(--sp-2); padding: 0 var(--sp-5); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Whom to invite: a contact, or anyone by their key. -->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import { groupError } from './errors';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { messengerStore } from '../store.svelte';
  import { groupStore } from './groupStore.svelte';
  import { contactLabel, type MessengerGroup } from '../api';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as "msg_you", params);

  interface Props { open: boolean; group: MessengerGroup }
  let { open = $bindable(), group }: Props = $props();

  let key = $state('');
  let busy = $state(false);
  let error = $state('');
  let sent = $state<string[]>([]);

  $effect(() => { if (!open) { key = ''; error = ''; sent = []; } });

  const q = $derived(key.trim().toLowerCase());
  const looksLikeKey = $derived(/^(npub1[0-9a-z]{20,}|[0-9a-f]{64})$/i.test(key.trim()));
  const inGroup = $derived(new Set(group.members.map((m) => m.pubkey)));
  const banned = $derived(new Set(group.banned));
  const contacts = $derived(
    messengerStore.contacts.filter((c) => !inGroup.has(c.pubkey) && !banned.has(c.pubkey) && (!q || contactLabel(c).toLowerCase().includes(q) || c.npub.includes(q))),
  );

  async function invite(who: string) {
    error = ''; busy = true;
    try {
      const i = await groupStore.invite(group.id, who);
      sent = [...sent, i.peer];
      if (who === key.trim()) key = '';
    } catch (e) {
      error = groupError(e, tr);
    } finally { busy = false; }
  }
</script>

<Dialog bind:open title={$t('msg_group_invite_title', { name: group.name })} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    <input type="text" bind:value={key} placeholder={$t('msg_newchat_placeholder')} spellcheck="false" disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter' && looksLikeKey) invite(key.trim()); }} />
    {#if looksLikeKey}
      <button class="btn btn-primary" disabled={busy} onclick={() => invite(key.trim())}>{$t('msg_group_invite')}</button>
    {/if}
    <p class="hint">{$t('msg_group_invite_hint')}</p>
    {#if error}<div class="error-msg">{error}</div>{/if}
    {#if contacts.length}
      <ul class="list">
        {#each contacts as c (c.pubkey)}
          <li class="row">
            <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={34} />
            <span class="name">{contactLabel(c)}</span>
            {#if sent.includes(c.pubkey)}
              <span class="done"><Icon name="check" size={13} />{$t('msg_group_invite_sent')}</span>
            {:else}
              <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => invite(c.pubkey)}>{$t('msg_group_invite')}</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  input {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  input:focus { outline: none; border-color: var(--accent-border); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  .list { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: var(--sp-3); padding: 6px 8px; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .done { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--success-text, var(--success)); }
  @media (pointer: coarse) { input { font-size: 16px; min-height: 44px; } }
</style>

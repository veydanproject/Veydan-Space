<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { messengerStore } from '../store.svelte';
  import { contactLabel, messengerError, type MessengerContact } from '../api';
  import { chatStore } from './chatStore.svelte';

  interface Props { open: boolean; onopened?: () => void }
  let { open = $bindable(), onopened }: Props = $props();

  let key = $state('');
  let busy = $state(false);
  let error = $state('');

  const q = $derived(key.trim().toLowerCase());
  const looksLikeKey = $derived(/^(npub1[0-9a-z]{20,}|[0-9a-f]{64})$/i.test(key.trim()));
  const contacts = $derived(
    messengerStore.contacts.filter((c) => !q || contactLabel(c).toLowerCase().includes(q) || c.npub.includes(q)),
  );

  async function start(peer: string) {
    error = ''; busy = true;
    try {
      await chatStore.openPeer(peer);
      open = false; key = '';
      onopened?.();
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  const pick = (c: MessengerContact) => start(c.pubkey);
</script>

<Dialog bind:open title={$t('msg_newchat_title')} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    <input type="text" bind:value={key} placeholder={$t('msg_newchat_placeholder')} spellcheck="false" disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter' && looksLikeKey) start(key.trim()); }} />
    {#if looksLikeKey}
      <button class="btn btn-primary" disabled={busy} onclick={() => start(key.trim())}>{$t('msg_newchat_start')}</button>
    {/if}
    {#if error}<div class="error-msg">{error}</div>{/if}

    {#if contacts.length}
      <div class="label">{$t('msg_contacts_title')}</div>
      <ul class="list">
        {#each contacts as c (c.pubkey)}
          <li>
            <button class="row" disabled={busy} onclick={() => pick(c)}>
              <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={34} />
              <span class="name">{contactLabel(c)}</span>
              <code>{c.npub.slice(0, 12)}…</code>
            </button>
          </li>
        {/each}
      </ul>
    {:else if !looksLikeKey}
      <p class="hint">{$t('msg_newchat_hint')}</p>
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
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .list { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: 6px 8px; text-align: left;
    border: none; border-radius: var(--radius-sm); background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  code { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
</style>

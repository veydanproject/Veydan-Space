<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Adds a contact by npub, hex key or NIP-05: a row of the contacts card on
  the desktop, the body of the phone's "New contact" sheet (stacked).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError } from '../api';

  interface Props {
    /** One field under the other and a full-width button (the phone's sheet). */
    stacked?: boolean;
    onadded?: () => void;
  }
  let { stacked = false, onadded }: Props = $props();

  let key = $state('');
  let nickname = $state('');
  let busy = $state(false);
  let error = $state('');

  const canAct = $derived(!!messengerStore.status?.runtime?.session_active);

  async function add() {
    if (!key.trim()) return;
    error = '';
    busy = true;
    try {
      await messengerStore.addContact(key, nickname);
      key = '';
      nickname = '';
      onadded?.();
    } catch (e) {
      error = messengerError(e);
    } finally {
      busy = false;
    }
  }
</script>

<form class="add" class:stacked onsubmit={(e) => { e.preventDefault(); add(); }}>
  <input type="text" class="key mono" bind:value={key} placeholder={$t('msg_contacts_key_placeholder')} spellcheck="false" disabled={busy || !canAct} />
  <input type="text" class="nick" bind:value={nickname} placeholder={$t('msg_contacts_nick_placeholder')} disabled={busy || !canAct} />
  <button class="btn btn-primary" type="submit" disabled={busy || !canAct || !key.trim()}>
    <Icon name="plus" size={14} />{$t('msg_contacts_add')}
  </button>
</form>
{#if !canAct}<div class="note">{$t('msg_debug_no_session')}</div>{/if}
{#if error}<div class="error-msg">{error}</div>{/if}

<style>
  .add { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .add input {
    flex: 1; min-width: 180px; font: inherit; font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  .add input.key { font-size: var(--fs-xs); }
  .add input.nick { flex: 0 1 140px; }
  .add input:focus { outline: none; border-color: var(--accent-border); }
  .note { color: var(--text-2); font-size: var(--fs-xs); margin: 0; }

  .add.stacked { flex-direction: column; flex-wrap: nowrap; }
  .add.stacked input, .add.stacked input.nick { flex: none; width: 100%; min-width: 0; border-radius: var(--radius); font-size: 16px; }
  .add.stacked .btn { justify-content: center; }
</style>

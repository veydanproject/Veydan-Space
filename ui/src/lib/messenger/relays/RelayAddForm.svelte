<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError } from '../api';

  let newUrl = $state('');
  let newKey = $state('');
  let busy = $state(false);
  let error = $state('');

  async function add() {
    const url = newUrl.trim();
    if (!url) return;
    error = '';
    busy = true;
    try {
      await messengerStore.addRelay(url, newKey);
      newUrl = '';
      newKey = '';
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }
</script>

<form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
  <input type="text" bind:value={newUrl} placeholder="wss://relay.example.com" spellcheck="false" disabled={busy} />
  <input type="password" class="key" bind:value={newKey} placeholder={$t('msg_relay_key_placeholder')} autocomplete="off" disabled={busy} />
  <button class="btn btn-ghost" type="submit" disabled={busy || !newUrl.trim()}>
    <Icon name="plus" size={18} />{$t('msg_relay_add')}
  </button>
</form>
{#if error}<div class="error-msg">{error}</div>{/if}

<style>
  .add { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  @media (max-width: 560px) {
    .add input:first-child { flex: 1 1 100%; }
    .add input.key { flex: 1 1 140px; }
  }
  .add input {
    flex: 1; min-width: 0; min-height: 44px; font: inherit; font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: 0 var(--sp-3);
  }
  .add input:focus { outline: none; border-color: var(--accent-border); }
  /* Room for the placeholder in either language ("ключ доступа (необязательно)"). */
  .add input.key { flex: 0 1 220px; font-family: inherit; }
</style>

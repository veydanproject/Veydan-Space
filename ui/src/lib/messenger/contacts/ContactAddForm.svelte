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

<!-- Stacked (in a window), «Добавить» is the window's foot: below the note and the error, pinned at the bottom. -->
<div class="add-wrap" class:stacked>
  <form class="add" class:stacked onsubmit={(e) => { e.preventDefault(); add(); }}>
    <input type="text" class="key mono" bind:value={key} placeholder={$t('msg_contacts_key_placeholder')} spellcheck="false" disabled={busy || !canAct} />
    <input type="text" class="nick" bind:value={nickname} placeholder={$t('msg_contacts_nick_placeholder')} disabled={busy || !canAct} />
    <div class="foot">
      <button class="btn btn-primary" type="submit" disabled={busy || !canAct || !key.trim()}>
        <Icon name="plus" size={18} />{$t('msg_contacts_add')}
      </button>
    </div>
  </form>
  {#if !canAct}<div class="note">{$t('msg_debug_no_session')}</div>{/if}
  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

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

  /* In a row of the contacts card the wrappers do not exist for the layout. */
  .add-wrap, .foot { display: contents; }

  /* Stacked: the fields 44 one under the other, the note and the error under them, «Добавить» last the whole width. */
  .add-wrap.stacked { display: flex; flex-direction: column; gap: var(--sp-3); }
  .add.stacked { display: contents; }
  .add.stacked input, .add.stacked input.nick {
    flex: none; width: 100%; min-width: 0; height: var(--control-h-lg); padding: 0 var(--sp-3);
    background: var(--surface-3); border-radius: var(--radius); font-size: var(--fs-base);
  }
  .add.stacked input.key { font-size: var(--fs-sm); }
  .add.stacked input::placeholder { font-family: var(--font-ui); }
  .add.stacked .foot { display: block; order: 1; }
  .add.stacked .btn { width: 100%; height: var(--control-h-lg); gap: var(--sp-2); justify-content: center; }
  /* On a computer's window the foot is the window's: the full width under a line, pinned to the bottom of the body. */
  :global(.dialog-body):has(> .add-wrap.stacked) { scrollbar-gutter: auto; }
  :global(.dialog-body) > .add-wrap.stacked .foot {
    position: sticky; bottom: calc(-1 * var(--sp-5)); margin: var(--sp-2) calc(-1 * var(--sp-6)) calc(-1 * var(--sp-5));
    padding: var(--sp-3) var(--sp-6) var(--sp-5); border-top: 1px solid var(--border); background: var(--glass-bg);
  }
  /* A finger: 48, and 16 so that a phone does not zoom into the field. */
  @media (pointer: coarse) {
    .add.stacked input, .add.stacked input.nick, .add.stacked .btn { height: 48px; }
    .add.stacked input, .add.stacked input.nick, .add.stacked input.key { font-size: var(--fs-md); }
  }
</style>

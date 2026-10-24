<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The Modules section of the settings (platform-spec 12): a switch for each
  module of the product that has one. A module that is off is hidden and its
  background work stops; its data stays and keeps syncing. The last module
  that is on cannot go off, and says so in its row. A row is the switch, the
  whole width: a card of rows on a computer, the list of the phone.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { registry } from '$lib/core/registry';
  import { modulesStore } from '$lib/core/store/modules.svelte';
  import { formatError } from '$lib/core/utils';

  let { mobile = false }: { mobile?: boolean } = $props();

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  const modules = $derived(registry.switchable());
  const onCount = $derived(modules.filter((m) => registry.enabled(m.id)).length);
  let busy = $state<string | null>(null);
  let error = $state('');

  async function flip(id: string) {
    if (busy) return;
    busy = id;
    error = '';
    try {
      await modulesStore.set(id, !registry.enabled(id));
    } catch (e) {
      const message = formatError(e);
      error = message === 'modules_last' ? $t('modules_last') : message;
    } finally {
      busy = null;
    }
  }
</script>

<!-- A row is the module's switch, the whole width; the one module left on says why it cannot go off, in its own row. -->
<div class={mobile ? 'list' : 'card list'} role="group" aria-label={$t('modules_section')}>
  {#each modules as m (m.id)}
    {@const on = registry.enabled(m.id)}
    {@const last = on && onCount === 1}
    {@const title = $t(m.title as TranslationKey)}
    <button
      type="button"
      class={mobile ? 'm-row' : 'row'}
      role="switch"
      aria-checked={on}
      aria-describedby={last && !mobile ? 'modules-last' : undefined}
      disabled={!isTauri || busy !== null || last}
      onclick={() => flip(m.id)}
    >
      <span class="module-icon"><Icon name={m.icon} size={20} /></span>
      <span class="words">
        <span class="title">{title}</span>
        {#if last && !mobile}<span class="sub" id="modules-last">{$t('modules_last')}</span>{/if}
      </span>
      <span class="toggle" class:on aria-hidden="true"></span>
    </button>
  {/each}
</div>
{#if error}
  <div class="note" class:m-error={mobile} role="alert">
    <Icon name="alert-triangle" size={18} />
    <span>{error}</span>
  </div>
{/if}

<style>
  .list { display: flex; flex-direction: column; }
  .card.list { padding: var(--sp-1); }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 60px; padding: var(--sp-2) var(--sp-4);
    border: 0; border-radius: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  /* The hover of a row: square between lines, the first and the last row take the card's corners (24 less its 4 of padding). */
  .row:first-child { border-top-left-radius: 20px; border-top-right-radius: 20px; }
  .row:last-child { border-bottom-left-radius: 20px; border-bottom-right-radius: 20px; }
  /* A straight line between rows: a shadow would follow the rounded corners. */
  .card.list .row { position: relative; }
  .card.list .row + .row::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .card.list .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .row:disabled, .m-row:disabled { cursor: default; opacity: 1; }
  button:disabled .toggle { opacity: 0.45; }
  .m-row { gap: var(--sp-3); }
  .module-icon {
    display: inline-flex; align-items: center; justify-content: center; width: 40px; height: 40px; flex-shrink: 0;
    border-radius: var(--radius-md); background: var(--accent-tint); color: var(--accent-text);
  }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: var(--fs-body); font-weight: var(--fw-semibold); }
  .m-row .title { font-size: var(--fs-md); }
  .sub { font-size: var(--fs-sm); line-height: 1.4; color: var(--text-2); }
  .toggle { flex-shrink: 0; pointer-events: none; }
  .note {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); background: var(--danger-bg); color: var(--text); font-size: var(--fs-base); line-height: 1.5;
  }
  .note :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--danger-text); }
  .m-error { margin: var(--sp-2) 0 0; }
</style>

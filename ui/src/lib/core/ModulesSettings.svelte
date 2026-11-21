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
  import Notice from '$lib/core/ui/Notice.svelte';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
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
    {#if mobile}
      <button type="button" class="list-row dense" role="switch" aria-checked={on} disabled={!isTauri || busy !== null || last} onclick={() => flip(m.id)}>
        <span class="module-icon"><Icon name={m.icon} size={20} /></span>
        <span class="words"><span class="title">{title}</span></span>
        <span class="toggle" class:on aria-hidden="true"></span>
      </button>
    {:else}
      <ToggleRow surface="row" {title} hint={last ? $t('modules_last') : undefined} {on} disabled={!isTauri || busy !== null || last} onclick={() => flip(m.id)}>
        {#snippet lead()}<span class="module-icon"><Icon name={m.icon} size={20} /></span>{/snippet}
      </ToggleRow>
    {/if}
  {/each}
</div>
{#if error}
  <div class="err" class:mobile><Notice tone="danger">{error}</Notice></div>
{/if}

<style>
  .list { display: flex; flex-direction: column; }
  /* The rows are ToggleRow `row`: they reach past this padding by --sp-3, their hover --sp-1 inside the card. */
  .card.list { padding: var(--sp-1) var(--sp-4); }
  .list-row:disabled { cursor: default; opacity: 1; }
  .list-row:disabled .toggle { opacity: 0.45; }
  .module-icon {
    display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; flex-shrink: 0;
    border-radius: var(--radius); background: var(--accent-tint); color: var(--accent-text);
  }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-0-5); }
  .list-row .title { font-size: var(--fs-body); font-weight: var(--fw-semibold); }
  .list-row .toggle { flex-shrink: 0; pointer-events: none; }
  .err.mobile { margin: var(--sp-2) 0 0; }
</style>

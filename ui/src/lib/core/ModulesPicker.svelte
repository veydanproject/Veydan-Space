<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The first start's question over a new data file (platform-spec 12): which
  modules this device is to show. Asked once, in a product of more than one
  module; the answer is the Modules section's to change later. Every module
  is ticked to begin with, and at least one stays ticked.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { registry } from '$lib/core/registry';
  import { modulesStore } from '$lib/core/store/modules.svelte';
  import { formatError } from '$lib/core/utils';

  const modules = $derived(registry.switchable());
  const open = $derived(modulesStore.firstRun && registry.only === undefined && modules.length > 1);
  let chosen = $state<string[] | null>(null);
  const picked = $derived(chosen ?? modules.map((m) => m.id));
  let busy = $state(false);
  let error = $state('');

  function flip(id: string) {
    chosen = picked.includes(id) ? picked.filter((x) => x !== id) : [...picked, id];
  }

  async function choose(ids: string[]) {
    if (busy || ids.length === 0) return;
    busy = true;
    error = '';
    try {
      await modulesStore.choose(ids);
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }
</script>

<!-- Closed without an answer, the device keeps everything. -->
<Dialog
  open={open}
  title={$t('modules_first_title')}
  closeOnBackdrop={false}
  onclose={() => choose(modules.map((m) => m.id))}
>
  <p class="muted hint">{$t('modules_first_hint')}</p>
  <div class="choices">
    {#each modules as m (m.id)}
      {@const on = picked.includes(m.id)}
      <button type="button" class="choice" class:on aria-pressed={on} disabled={busy} onclick={() => flip(m.id)}>
        <span class="choice-icon"><Icon name={m.icon} size={18} /></span>
        <span class="choice-title">{$t(m.title as TranslationKey)}</span>
        <span class="check" aria-hidden="true">{#if on}<Icon name="check" size={14} />{/if}</span>
      </button>
    {/each}
  </div>
  {#if error}<p class="error-msg">{error}</p>{/if}
  {#snippet footer()}
    <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => choose(modules.map((m) => m.id))}>
      {$t('modules_first_all')}
    </button>
    <button class="btn btn-primary btn-sm" disabled={busy || picked.length === 0} onclick={() => choose(picked)}>
      {$t('modules_first_continue')}
    </button>
  {/snippet}
</Dialog>

<style>
  .hint { margin: 0 0 var(--sp-4); }
  .choices { display: flex; flex-direction: column; gap: var(--sp-2); }
  .choice {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    width: 100%;
    min-height: var(--control-h-lg);
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-base);
    text-align: left;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }
  .choice:hover:not(:disabled) { border-color: var(--border-2); }
  .choice.on { border-color: var(--accent-border); background: var(--accent-bg); }
  .choice-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: var(--accent-tint);
    color: var(--accent-text);
  }
  .choice-title { flex: 1; min-width: 0; font-weight: var(--fw-semibold); }
  .check {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border: 1px solid var(--border-2);
    border-radius: var(--radius-xs);
    color: #fff;
  }
  .choice.on .check { background: var(--accent); border-color: var(--accent); }
</style>

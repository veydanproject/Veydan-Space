<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The first start's question (platform-spec 12): which modules this device
  is to show. Asked once, in a product of more than one module; the answer
  is the Modules section's to change later. Every module is ticked to begin
  with, and at least one stays ticked.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { registry } from '$lib/core/registry';
  import { modulesStore } from '$lib/core/store/modules.svelte';
  import { formatError } from '$lib/core/utils';
  import { onboarding } from './store.svelte';

  const modules = $derived(registry.switchable());
  // What is on now: all of it at the first start, the answer given when the person comes back to the step.
  let chosen = $state<string[] | null>(null);
  const picked = $derived(chosen ?? modules.filter((m) => modulesStore.isOn(m.id)).map((m) => m.id));
  let busy = $state(false);
  let error = $state('');

  function flip(id: string) {
    chosen = picked.includes(id) ? picked.filter((x) => x !== id) : [...picked, id];
  }

  async function choose() {
    if (busy || picked.length === 0) return;
    busy = true;
    error = '';
    try {
      await modulesStore.choose(picked);
      await onboarding.next();
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="ob-main">
  <h1 class="ob-title">{$t('modules_first_title')}</h1>
  <p class="ob-text">{$t('modules_first_hint')}</p>
  <div class="ob-opts">
    {#each modules as m (m.id)}
      {@const on = picked.includes(m.id)}
      <button type="button" class="ob-opt" class:on aria-pressed={on} disabled={busy} onclick={() => flip(m.id)}>
        <span class="ob-opt-ic"><Icon name={m.icon} size={20} /></span>
        <span class="ob-opt-b"><span class="ob-opt-t">{$t(m.title as TranslationKey)}</span></span>
        <span class="ob-check" aria-hidden="true">{#if on}<Icon name="check" size={14} />{/if}</span>
      </button>
    {/each}
  </div>
  {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
</div>
<div class="ob-foot">
  <button type="button" class="btn btn-primary" disabled={busy || picked.length === 0} onclick={choose}>{$t('modules_first_continue')}</button>
</div>

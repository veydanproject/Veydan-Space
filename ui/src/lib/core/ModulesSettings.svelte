<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The Modules section of the settings (platform-spec 12): a switch for each
  module of the product that has one. A module that is off is hidden and its
  background work stops; its data stays and keeps syncing. The last module
  that is on cannot go off. The desktop card's body and the phone's list rows.
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

{#each modules as m (m.id)}
  {@const on = registry.enabled(m.id)}
  {@const last = on && onCount === 1}
  {@const title = $t(m.title as TranslationKey)}
  {#if mobile}
    <button type="button" class="m-row" disabled={!isTauri || busy !== null || last} onclick={() => flip(m.id)}>
      <span class="module-icon"><Icon name={m.icon} size={16} /></span>
      <span class="m-row-label">{title}</span>
      <span class="toggle" class:on aria-hidden="true"></span>
    </button>
  {:else}
    <div class="dev-tools-row">
      <div class="module">
        <span class="module-icon"><Icon name={m.icon} size={16} /></span>
        <span>{title}</span>
      </div>
      <button
        class="toggle"
        class:on
        disabled={!isTauri || busy !== null || last}
        title={last ? $t('modules_last') : undefined}
        onclick={() => flip(m.id)}
        aria-pressed={on}
        aria-label={title}
      ></button>
    </div>
  {/if}
{/each}
{#if !mobile && onCount === 1}
  <p class="muted last">{$t('modules_last')}</p>
{/if}
{#if error}
  <p class="error-msg" class:m-error={mobile}>{error}</p>
{/if}

<style>
  .module {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    font-size: var(--fs-base);
    color: var(--text);
  }
  .module-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: var(--accent-tint);
    color: var(--accent-text);
  }
  .m-row:disabled { opacity: 1; }
  .m-row:disabled .toggle { opacity: 0.5; }
  .last { margin: var(--sp-3) 0 0; font-size: var(--fs-sm); }
  .m-error { margin: var(--sp-2) var(--sp-4); }
</style>

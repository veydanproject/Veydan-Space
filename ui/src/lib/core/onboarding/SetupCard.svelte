<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Finish setting up»: what the first start left undone, on Home of a phone
  and in a corner of the window of a computer. It is there after the wizard
  of this device (the mark the wizard leaves) and goes when everything is
  done or the person hides it; an install that never saw the wizard never
  sees the card.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { SETUP_CARD_KEY, onboarding } from './store.svelte';
  import { doneCount, setupLines, type SetupLine } from './setup';

  let { floating = false }: { floating?: boolean } = $props();

  function marked(): boolean {
    try {
      return localStorage.getItem(SETUP_CARD_KEY) === 'open';
    } catch {
      return false;
    }
  }

  let open = $state(marked());
  let lines = $state<SetupLine[]>([]);
  const done = $derived(doneCount(lines));
  const shown = $derived(open && !onboarding.active && lines.length > 0 && done < lines.length);

  function forget() {
    open = false;
    try {
      localStorage.removeItem(SETUP_CARD_KEY);
    } catch {
      // Nothing was kept.
    }
  }

  async function load() {
    if (!marked()) {
      open = false;
      return;
    }
    open = true;
    lines = await setupLines();
    if (lines.length > 0 && doneCount(lines) === lines.length) forget();
  }

  // The wizard ended, or the app came back to the front: what is done is asked again.
  $effect(() => {
    if (!onboarding.active) void load();
  });

  onMount(() => {
    const again = () => {
      if (document.visibilityState === 'visible' && !onboarding.active) void load();
    };
    document.addEventListener('visibilitychange', again);
    return () => document.removeEventListener('visibilitychange', again);
  });
</script>

{#if shown}
  <section class="setup" class:floating aria-labelledby="setup-title">
    <header>
      <strong id="setup-title">{$t('setup_title')}</strong>
      <span class="count">{$t('setup_progress', { done: String(done), total: String(lines.length) })}</span>
      <button type="button" class="hide" onclick={forget} aria-label={$t('setup_dismiss')} title={$t('setup_dismiss')}><Icon name="x" size={16} /></button>
    </header>
    <div class="bar" aria-hidden="true"><i style="width: {(done / lines.length) * 100}%"></i></div>
    <ul>
      {#each lines as line (line.item.id)}
        <li>
          {#if line.done}
            <span class="line done"><Icon name="check-circle" size={18} /><span>{$t(line.item.title as TranslationKey)}</span></span>
          {:else if line.item.step && onboarding.has(line.item.step)}
            <!-- Set up where it was passed by: the wizard opens again at its step. -->
            <button type="button" class="line" onclick={() => onboarding.resume(line.item.step ?? '')}>
              <Icon name="circle" size={18} /><span>{$t(line.item.title as TranslationKey)}</span><Icon name="chevron-right" size={16} />
            </button>
          {:else}
            <a class="line" href={line.item.href()}>
              <Icon name="circle" size={18} /><span>{$t(line.item.title as TranslationKey)}</span><Icon name="chevron-right" size={16} />
            </a>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .setup {
    background: var(--m-card, var(--surface)); border: 1px solid var(--m-card-border, var(--border)); border-radius: var(--m-radius, var(--radius-md));
    box-shadow: var(--m-card-shadow, none); padding: var(--sp-3) var(--sp-4) var(--sp-1); margin-bottom: var(--sp-5); color: var(--text);
  }
  .setup.floating {
    position: fixed; left: var(--sp-4); bottom: var(--sp-4); width: 300px; margin: 0; z-index: var(--z-panel);
    background: var(--surface-drawer); box-shadow: var(--shadow-lg);
  }
  header { display: flex; align-items: center; gap: var(--sp-2); margin-bottom: var(--sp-2); }
  strong { flex: 1; min-width: 0; font-size: var(--fs-body); }
  .count { color: var(--text-2); font-size: var(--fs-sm); }
  .hide { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; margin-right: calc(var(--sp-2) * -1); border: 0; border-radius: 50%; background: none; color: var(--text-3); cursor: pointer; }
  .bar { height: 6px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .bar i { display: block; height: 100%; border-radius: var(--radius-pill); background: var(--accent); transition: width 0.2s ease; }
  ul { list-style: none; margin: var(--sp-1) 0 0; padding: 0; }
  li + li { border-top: 1px solid var(--m-card-border, var(--border)); }
  .line { display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 44px; padding: 0; border: 0; background: none; color: var(--text); font: inherit; font-size: var(--fs-base); text-align: left; text-decoration: none; cursor: pointer; }
  .line span { flex: 1; min-width: 0; }
  .line :global(svg) { color: var(--text-3); flex-shrink: 0; }
  .line.done { color: var(--text-2); cursor: default; }
  .line.done :global(svg) { color: var(--success-text); }
</style>

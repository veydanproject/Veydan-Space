<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Emoji panel for any composer, and for a reaction in the menu of a message.
  A popover floats above the composer and closes on a tap outside; inline
  sits in the flow of whatever holds it, which also decides when it closes.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { EMOJI, loadRecent, pushRecent, type EmojiGroup } from './data';
  import { usageStore } from './usageStore.svelte';

  interface Props {
    onpick: (emoji: string) => void;
    onclose?: () => void;
    variant?: 'popover' | 'inline';
  }
  let { onpick, onclose, variant = 'popover' }: Props = $props();
  const inline = $derived(variant === 'inline');

  /** Recents of this device: shown only until the runtime names popular ones. */
  let local = $state<string[]>([]);
  const popular = $derived(usageStore.popular.length > 0);
  const recent = $derived(popular ? usageStore.popular : local);
  let active = $state<EmojiGroup['id'] | 'recent'>('smileys');
  let root = $state<HTMLDivElement | null>(null);
  let body = $state<HTMLDivElement | null>(null);

  onMount(() => {
    local = loadRecent();
    if (recent.length) active = 'recent';
    if (variant === 'inline') return;
    // A tap outside closes the panel; taps inside never steal the focus.
    const outside = (e: PointerEvent) => {
      const target = e.target as Node;
      if (root && !root.contains(target) && !(target as HTMLElement).closest?.('[data-emoji-toggle]')) onclose?.();
    };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') onclose?.(); };
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('keydown', key);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('keydown', key);
    };
  });

  function pick(e: string) {
    local = pushRecent(e);
    onpick(e);
  }

  function go(id: EmojiGroup['id'] | 'recent') {
    active = id;
    body?.querySelector(`[data-group="${id}"]`)?.scrollIntoView({ block: 'start' });
  }

  const keep = (e: Event) => e.preventDefault();
</script>

<div class="picker msg-font" class:inline bind:this={root} role="dialog" aria-label={$t('msg_emoji_title')}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tabs" onpointerdown={keep}>
    {#if recent.length}
      <button class:active={active === 'recent'} onclick={() => go('recent')} title={popular ? $t('msg_reactions_popular') : $t('msg_emoji_recent')}>🕘</button>
    {/if}
    {#each EMOJI as g (g.id)}
      <button class:active={active === g.id} onclick={() => go(g.id)} title={$t(`msg_emoji_${g.id}` as 'msg_emoji_smileys')}>{g.icon}</button>
    {/each}
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="body" bind:this={body} onpointerdown={keep}>
    {#if recent.length}
      <div class="label" data-group="recent">{popular ? $t('msg_reactions_popular') : $t('msg_emoji_recent')}</div>
      <div class="grid">{#each recent as e (e)}<button onclick={() => pick(e)}>{e}</button>{/each}</div>
    {/if}
    {#each EMOJI as g (g.id)}
      <div class="label" data-group={g.id}>{$t(`msg_emoji_${g.id}` as 'msg_emoji_smileys')}</div>
      <div class="grid">{#each g.list as e (e)}<button onclick={() => pick(e)}>{e}</button>{/each}</div>
    {/each}
  </div>
</div>

<style>
  .picker {
    position: absolute; left: var(--sp-3); bottom: calc(100% + 6px); z-index: var(--z-popover, 50);
    width: min(360px, calc(100% - 2 * var(--sp-3))); height: 300px; display: flex; flex-direction: column;
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-md); box-shadow: var(--shadow-lg);
    overflow: hidden;
  }
  .tabs { display: flex; gap: 2px; padding: 4px; border-bottom: 1px solid var(--border); overflow-x: auto; scrollbar-width: none; flex-shrink: 0; }
  .tabs button { border: none; background: none; font-size: 18px; line-height: 1; padding: 6px 7px; border-radius: var(--radius-sm); cursor: pointer; filter: grayscale(0.4); opacity: 0.75; }
  .tabs button.active { background: var(--accent-tint); filter: none; opacity: 1; }
  .body { flex: 1; min-height: 0; overflow-y: auto; padding: 4px 6px 8px; }
  .label { position: sticky; top: 0; background: var(--surface); font-size: var(--fs-2xs); font-weight: var(--fw-bold); text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-3); padding: 6px 4px 4px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(36px, 1fr)); }
  .grid button { border: none; background: none; font-size: 22px; line-height: 1; padding: 6px 0; border-radius: var(--radius-sm); cursor: pointer; font-family: inherit; }
  .grid button:hover { background: var(--surface-3); }
  /* In the flow of a menu or a sheet: as wide as it, no shadow of its own. */
  .picker.inline { position: static; width: 100%; height: 260px; box-shadow: none; }
  @media (pointer: coarse) {
    .picker:not(.inline) { left: 0; right: 0; width: 100%; border-radius: var(--radius-md) var(--radius-md) 0 0; border-inline: none; bottom: 100%; height: 280px; }
    .grid { grid-template-columns: repeat(auto-fill, minmax(42px, 1fr)); }
    .grid button { font-size: 26px; padding: 8px 0; }
  }
</style>

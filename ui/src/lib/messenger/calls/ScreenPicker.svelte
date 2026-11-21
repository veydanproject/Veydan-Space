<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My screen in a call on a computer: the button (drawn by the bar it
  stands in, `button`) and the choice of screens and windows when there
  are several. One thing to share is shared at once; nothing to share
  says so. The list is asked each time the choice opens: windows come
  and go. Escape closes it.

  Used by the capsule of a call of two (the menu opens under it, `below`),
  the video window of that call and the window of a group call.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import type { ScreenInfo } from '../api';
  import { callStore } from './callStore.svelte';

  interface Props {
    /** My screen goes now: the button stops it. */
    sharing: boolean;
    /** Shares the screen or window `id`. */
    share: (id: string) => Promise<void>;
    stop: () => Promise<void>;
    /** The menu opens under the button (a bar at the top of the window). */
    below?: boolean;
    /** The button: `onclick` opens the choice or stops the sharing, `expanded` while the choice is open. */
    button: Snippet<[{ onclick: () => void; expanded: boolean }]>;
  }
  let { sharing, share, stop, below = false, button }: Props = $props();

  let picking = $state<ScreenInfo[] | null>(null);
  let none = $state(false);
  let wrapEl = $state<HTMLSpanElement | null>(null);
  let at = $state({ x: 0, y: 0 });
  /** The press on the button that closed the menu (its mousedown) does not open it again. */
  let closedAt = 0;

  const items = $derived<MenuEntry[]>(picking ? [
    { type: 'heading', label: $t('msg_call_screen_pick') },
    ...picking.map((s) => ({
      label: s.title,
      icon: s.window ? 'browser' : 'screen-share',
      shortcut: $t(s.window ? 'msg_call_screen_kind_window' : 'msg_call_screen_kind_screen'),
      onselect: () => void pick(s),
    })),
  ] : []);

  async function toggle() {
    if (sharing) { picking = null; await stop(); return; }
    if (picking) { picking = null; return; }
    if (performance.now() - closedAt < 300) return;
    none = false;
    const list = await callStore.loadScreens();
    if (!list) return;
    if (list.length > 1) {
      const r = wrapEl?.getBoundingClientRect();
      // Over the button, centred; under it on a bar at the top (the menu keeps itself on the screen).
      if (r) at = below ? { x: r.left + r.width / 2 - 110, y: r.bottom + 10 } : { x: r.left + r.width / 2, y: r.top - 2 };
      picking = list;
    }
    else if (list.length === 1) await share(list[0].id);
    else none = true;
  }

  async function pick(s: ScreenInfo) {
    picking = null;
    await share(s.id);
  }
</script>

<span class="menu-wrap" bind:this={wrapEl}>
  {@render button({ onclick: () => void toggle(), expanded: !!picking })}
  <ContextMenu open={!!picking} x={at.x} y={at.y} {items} place={below ? 'point' : 'above'} onclose={() => { picking = null; closedAt = performance.now(); }} />
  {#if none}
    <div class="menu" class:below role="status">{$t('msg_call_screen_none')}</div>
  {/if}
</span>

<style>
  .menu-wrap { position: relative; display: inline-flex; }
  .menu {
    position: absolute; z-index: var(--z-popover); bottom: calc(100% + 10px); left: 50%; transform: translateX(-50%);
    width: max-content; max-width: min(360px, 80vw); padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md);
    background: var(--glass-bg); border: 1px solid var(--glass-border); box-shadow: var(--glass-highlight), var(--shadow-lg);
    -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
    font-size: var(--fs-xs); color: var(--text-2); cursor: default;
  }
  .menu.below { bottom: auto; top: calc(100% + 10px); }
</style>

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
  import Icon from '$lib/core/Icon.svelte';
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

  async function toggle() {
    if (sharing) { picking = null; await stop(); return; }
    if (picking) { picking = null; return; }
    none = false;
    const list = await callStore.loadScreens();
    if (!list) return;
    if (list.length > 1) picking = list;
    else if (list.length === 1) await share(list[0].id);
    else none = true;
  }

  async function pick(s: ScreenInfo) {
    picking = null;
    await share(s.id);
  }
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && picking) picking = null; }} />

<span class="menu-wrap">
  {@render button({ onclick: () => void toggle(), expanded: !!picking })}
  {#if picking}
    <div class="menu" class:below role="menu" aria-label={$t('msg_call_screen_pick')}>
      <div class="menu-title">{$t('msg_call_screen_pick')}</div>
      {#each picking as s (s.id)}
        <button class="item" role="menuitem" onclick={() => pick(s)}>
          <Icon name={s.window ? 'browser' : 'screen-share'} size={14} />
          <span class="item-title">{s.title}</span>
          <span class="item-kind">{$t(s.window ? 'msg_call_screen_kind_window' : 'msg_call_screen_kind_screen')}</span>
        </button>
      {/each}
    </div>
  {:else if none}
    <div class="menu note" class:below role="status">{$t('msg_call_screen_none')}</div>
  {/if}
</span>

<style>
  .menu-wrap { position: relative; display: inline-flex; }
  .menu {
    position: absolute; z-index: 10; bottom: calc(100% + 10px); left: 50%; transform: translateX(-50%);
    width: max-content; min-width: 220px; max-width: min(360px, 80vw); max-height: 260px; overflow-y: auto;
    padding: 6px; border-radius: var(--radius); background: var(--surface); color: var(--text);
    border: 1px solid var(--border-strong); box-shadow: var(--shadow-lg); cursor: default;
  }
  .menu.below { bottom: auto; top: calc(100% + 10px); }
  .menu.note { padding: 8px 12px; font-size: var(--fs-xs); color: var(--text-2); min-width: 0; }
  .menu-title { padding: 4px 8px 6px; font-size: var(--fs-2xs); color: var(--text-3); text-transform: uppercase; letter-spacing: 0.4px; }
  .item {
    width: 100%; display: flex; align-items: center; gap: 8px; padding: 7px 8px; border: none; border-radius: var(--radius-sm);
    background: none; color: inherit; font: inherit; font-size: var(--fs-sm); cursor: pointer; text-align: left;
  }
  .item:hover, .item:focus-visible { background: var(--surface-2); outline: none; }
  .item-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .item-kind { font-size: var(--fs-2xs); color: var(--text-3); }
</style>

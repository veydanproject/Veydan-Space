<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The parts of a page, one at a time: a row of tabs, the chosen one
  underlined. One look for «Аккаунт» and «Настройки чата» (ROO-259 item 8).
  On a phone the row is one line that scrolls, reaching the edges of the
  screen through the page's padding (`--tabs-bleed`, that padding at the
  sides; 16 by default), its right edge fading while there is more of it;
  the chosen tab is kept in view (`scrollTabs`). The arrows move along the
  tabs.
-->
<script lang="ts" generics="T extends string">
  import { tick } from 'svelte';
  import { scrollTabs } from './scrollTabs';

  interface Props {
    tabs: { id: T; label: string }[];
    selected: T;
    onpick: (id: T) => void;
    label?: string;
    /** The tab is `<prefix>-tab-<id>` and names its panel `<prefix>-panel-<id>`. */
    idPrefix?: string;
    phone?: boolean;
  }
  let { tabs, selected, onpick, label, idPrefix, phone = false }: Props = $props();

  let row = $state<HTMLElement | null>(null);

  function key(e: KeyboardEvent) {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const i = tabs.findIndex((x) => x.id === selected);
    onpick(tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length].id);
    void tick().then(() => row?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus());
  }
</script>

<div class="tabs" class:m={phone} role="tablist" aria-label={label} tabindex="-1" bind:this={row} onkeydown={key} use:scrollTabs={{ selected, watch: tabs }}>
  {#each tabs as x (x.id)}
    <button
      type="button"
      role="tab"
      class="ctab"
      id={idPrefix ? `${idPrefix}-tab-${x.id}` : undefined}
      aria-selected={selected === x.id}
      aria-controls={idPrefix ? `${idPrefix}-panel-${x.id}` : undefined}
      tabindex={selected === x.id ? 0 : -1}
      onclick={() => onpick(x.id)}
    >{x.label}</button>
  {/each}
</div>

<style>
  .tabs { display: flex; gap: var(--sp-1); border-bottom: 1px solid var(--divider); overflow-x: auto; scrollbar-width: none; }
  .tabs::-webkit-scrollbar { display: none; }
  .ctab {
    flex-shrink: 0; min-height: var(--control-h-lg); padding: 0 var(--sp-3); margin-bottom: -1px;
    border: 0; border-bottom: 2px solid transparent; border-radius: 0; background: none;
    color: var(--text-2); font: inherit; font-size: var(--fs-base); font-weight: var(--fw-semibold); white-space: nowrap; cursor: pointer;
    transition: color var(--dur-fast), border-color var(--dur-fast);
  }
  .ctab:hover { color: var(--text); }
  /* The row scrolls and clips: the keyboard's ring drawn inside the tab. */
  .ctab:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .ctab[aria-selected='true'] { color: var(--accent-text); border-bottom-color: var(--accent); }
  .m { margin: 0 calc(-1 * var(--tabs-bleed, var(--sp-4))); padding: 0 var(--sp-4); gap: 0; }
  .m:global(.fade-end) { -webkit-mask-image: linear-gradient(to right, #000 82%, transparent); mask-image: linear-gradient(to right, #000 82%, transparent); }
  .m .ctab { min-height: var(--sp-12); padding: 0 var(--sp-4); font-size: var(--fs-body); }
</style>

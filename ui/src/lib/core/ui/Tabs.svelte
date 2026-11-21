<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The parts of a page, one at a time: the row of tabs of the whole interface
  (TabBar, design 01) for a list of words. The arrows move along the tabs.
  On a phone (`phone`) a tab is 48 and the row reaches the edges of the
  screen through the page's padding (`--tabs-bleed`, 16 by default); it
  scrolls under the finger, a clipped edge fading, the chosen tab kept in view.
-->
<script lang="ts" generics="T extends string">
  import { tick } from 'svelte';
  import TabBar from './TabBar.svelte';

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

<TabBar {selected} watch={tabs} {label} {phone} tabindex={-1} onkeydown={key} bind:row>
  {#each tabs as x (x.id)}
    <button
      type="button"
      role="tab"
      class="tab"
      class:active={selected === x.id}
      id={idPrefix ? `${idPrefix}-tab-${x.id}` : undefined}
      aria-selected={selected === x.id}
      aria-controls={idPrefix ? `${idPrefix}-panel-${x.id}` : undefined}
      tabindex={selected === x.id ? 0 : -1}
      onclick={() => onpick(x.id)}
    >{x.label}</button>
  {/each}
</TabBar>

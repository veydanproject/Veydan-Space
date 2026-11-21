<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A row of tabs (design 01): the caller's `.tab` buttons on the line of the
  section, `.tab-bar` of base.css. One bar of the accent lies on the line under
  the chosen tab (`.tab.active` or `aria-selected`) and moves to the next one
  chosen (`--dur-base`, at once with reduced motion).
  A row that does not fit scrolls sideways (`scrollTabs`: the chosen tab is
  kept in view; `wheel`, a mouse's wheel moves it). On a computer a clipped
  edge has an arrow on glass that moves the row by what shows, its edge
  faded out under it; on a phone (`phone`, or a finger's pointer) there are
  no arrows and a clipped edge only fades.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { scrollTabs } from './scrollTabs';

  interface Props {
    /** The chosen tab: when it changes, it is brought into view. */
    selected?: unknown;
    /** What may widen the tabs (their list, their counts). */
    watch?: unknown;
    wheel?: boolean;
    label?: string;
    phone?: boolean;
    /** The row itself (the tablist): Tabs focuses its chosen tab through it. */
    row?: HTMLElement | null;
    tabindex?: number;
    onkeydown?: (e: KeyboardEvent) => void;
    children: Snippet;
  }
  let { selected, watch, wheel = false, label, phone = false, row = $bindable(null), tabindex, onkeydown, children }: Props = $props();

  let before = $state(false);
  let after = $state(false);
  let ink = $state({ left: 0, width: 0 });
  /** The bar moves only once it stands somewhere: the first time it is put in place, not flown in from the left. */
  let ready = $state(false);
  /** Bumped when a tab changes its size or the tabs change: scrollTabs measures the edges again (the arrows). */
  let sized = $state(0);
  /** With arrows the chosen tab stands clear of the faded edge under them (whole from 72, as the mask). */
  const fine = typeof matchMedia === 'function' && matchMedia('(pointer: fine)').matches;
  const inset = $derived(!phone && fine ? 72 : 0);

  function place() {
    const on = row?.querySelector<HTMLElement>('.tab.active, [role="tab"][aria-selected="true"]');
    if (!on || !on.offsetWidth) {
      ink = { left: 0, width: 0 };
      return;
    }
    const pad = parseFloat(getComputedStyle(on).paddingLeft) || 0;
    ink = { left: on.offsetLeft + pad, width: Math.max(on.offsetWidth - 2 * pad, 0) };
    if (!ready) requestAnimationFrame(() => (ready = true));
  }

  onMount(() => {
    if (!row) return;
    const node = row;
    // The chosen tab, its width (a bold word, a count) and the row's room move the bar and the arrows.
    const measure = () => {
      place();
      sized++;
    };
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    ro?.observe(node);
    for (const tab of node.querySelectorAll('.tab')) ro?.observe(tab);
    const mo = new MutationObserver((records) => {
      for (const r of records) for (const n of r.addedNodes) if (n instanceof Element && n.classList.contains('tab')) ro?.observe(n);
      measure();
    });
    mo.observe(node, { subtree: true, childList: true, attributes: true, attributeFilter: ['class', 'aria-selected'] });
    measure();
    return () => {
      ro?.disconnect();
      mo.disconnect();
    };
  });

  function page(dir: 1 | -1) {
    if (!row) return;
    const still = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    // What shows between the two faded edges, so the tab half under an arrow comes out whole.
    row.scrollBy({ left: dir * Math.max(row.clientWidth - 80, 40), behavior: still ? 'auto' : 'smooth' });
  }
</script>

<div class="tab-row" class:m={phone}>
  {#if !phone && before}
    <button type="button" class="icon-btn tab-arrow back" aria-label={$t('tabs_scroll_back')} title={$t('tabs_scroll_back')} onclick={() => page(-1)}>
      <Icon name="chevron-left" size={16} />
    </button>
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="tab-bar inked"
    class:ready
    class:arrows={!phone}
    role="tablist"
    aria-label={label}
    {tabindex}
    {onkeydown}
    bind:this={row}
    use:scrollTabs={{ selected, watch: [watch, sized], wheel, inset, fadeStart: true, onedges: (b, a) => ((before = b), (after = a)) }}
  >
    {@render children()}
    <span class="tab-ink" aria-hidden="true" hidden={!ink.width} style:left="{ink.left}px" style:width="{ink.width}px"></span>
  </div>
  {#if !phone && after}
    <button type="button" class="icon-btn tab-arrow more" aria-label={$t('tabs_scroll_more')} title={$t('tabs_scroll_more')} onclick={() => page(1)}>
      <Icon name="chevron-right" size={16} />
    </button>
  {/if}
</div>

<style>
  .tab-row { position: relative; display: flex; min-width: 0; }
  .tab-bar { flex: 1; }

  /* A clipped edge without an arrow (a phone) fades over the last 15% of the row. */
  .tab-bar:global(.fade-end) { -webkit-mask-image: linear-gradient(to right, black 85%, transparent); mask-image: linear-gradient(to right, black 85%, transparent); }
  .tab-bar:global(.fade-start) { -webkit-mask-image: linear-gradient(to left, black 85%, transparent); mask-image: linear-gradient(to left, black 85%, transparent); }
  .tab-bar:global(.fade-start.fade-end) {
    -webkit-mask-image: linear-gradient(to right, transparent, black 15%, black 85%, transparent);
    mask-image: linear-gradient(to right, transparent, black 15%, black 85%, transparent);
  }
  /* Under an arrow the edge is gone: clear over the arrow and --sp-2 (40), whole from 72 — no word shows through the glass. */
  @media (pointer: fine) {
    .tab-bar.arrows:global(.fade-end) { -webkit-mask-image: linear-gradient(to left, transparent 40px, black 72px); mask-image: linear-gradient(to left, transparent 40px, black 72px); }
    .tab-bar.arrows:global(.fade-start) { -webkit-mask-image: linear-gradient(to right, transparent 40px, black 72px); mask-image: linear-gradient(to right, transparent 40px, black 72px); }
    .tab-bar.arrows:global(.fade-start.fade-end) {
      -webkit-mask-image: linear-gradient(to right, transparent 40px, black 72px, black calc(100% - 72px), transparent calc(100% - 40px));
      mask-image: linear-gradient(to right, transparent 40px, black 72px, black calc(100% - 72px), transparent calc(100% - 40px));
    }
  }
  @media not (pointer: fine) {
    .tab-arrow { display: none; }
  }

  /* The arrow: the plain .icon-btn of 32 on glass, in the middle of the row's height, its icon 16. */
  .tab-arrow {
    position: absolute; top: calc(50% - 16px); z-index: 1;
    background: var(--glass-bg); border-color: var(--glass-border);
    -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
  }
  .tab-arrow:hover:not(:disabled) { background: linear-gradient(var(--surface-hover), var(--surface-hover)), var(--glass-bg); color: var(--text); }
  .tab-arrow > :global(.icon) { width: 16px; height: 16px; }
  .back { left: 0; }
  .more { right: 0; }

  /* A phone: 48 high, the words of the body, the line from edge to edge of the screen through the page's padding
     (`--tabs-bleed`, 16 by default); the first word stands where the page's text starts. */
  .m { margin: 0 calc(-1 * var(--tabs-bleed, var(--sp-4))); }
  .m .tab-bar { padding-inline: calc(var(--tabs-bleed, var(--sp-4)) - var(--sp-2)); }
  .m .tab-bar > :global(.tab) { min-height: var(--sp-12); font-size: var(--fs-body); }
</style>

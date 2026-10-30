<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { navItems } from '$lib/core/mobile/nav';
  import type { NavItem } from '$lib/core/module';
  import { product } from '$lib/core/product';

  interface Props {
    /** With the hub: items around the centre button, first half left, second half right. */
    items: NavItem[];
    activeId: string;
    /**
     * The centre button that opens the sheet of the product's apps. A product
     * of one module has no apps to choose between: its bar is plain equal
     * columns (platform-spec 11.7).
     */
    hub?: boolean;
    hubActive?: boolean;
    onhub?: () => void;
  }

  let { items, activeId, hub = true, hubActive = false, onhub }: Props = $props();

  const half = $derived(hub ? Math.ceil(items.length / 2) : items.length);
  const left = $derived(items.slice(0, half));
  const right = $derived(items.slice(half));

  /**
   * A tab of the bar replaces the current screen in the history, so the
   * system's Back leaves the bar's screens instead of replaying the taps; an
   * item that opens a screen without a bar (a new note) is a step forward.
   */
  function isTab(href: string): boolean {
    try {
      return navItems(new URL(href, location.origin)) !== null;
    } catch {
      return false;
    }
  }
</script>

{#snippet count(it: NavItem)}
  {@const n = it.badge?.() ?? 0}
  {#if n > 0}<span class="count-pill sm danger nav-count">{n > 99 ? "99+" : n}</span>{/if}
{/snippet}

{#snippet item(it: NavItem)}
  {#if it.href}
    <a
      href={it.href}
      class="m-bar-item"
      class:active={activeId === it.id}
      aria-current={activeId === it.id ? 'page' : undefined}
      data-sveltekit-replacestate={isTab(it.href) ? '' : undefined}
    >
      <span class="m-bar-ico"><Icon name={it.icon} size={22} />{@render count(it)}</span>
      <span class="m-bar-label">{$t(it.title)}</span>
    </a>
  {:else}
    <button type="button" class="m-bar-item" class:active={activeId === it.id} onclick={it.onclick}>
      <span class="m-bar-ico"><Icon name={it.icon} size={22} />{@render count(it)}</span>
      <span class="m-bar-label">{$t(it.title)}</span>
    </button>
  {/if}
{/snippet}

<!-- The bar floats over the glow; its slot keeps the room, so no screen scrolls under it. -->
<nav class="nav">
  <div class="bar m-bar" style:--cols={items.length + (hub ? 1 : 0)}>
    {#each left as it (it.id)}{@render item(it)}{/each}
    {#if hub}
      <button type="button" class="m-bar-item item--hub" class:active={hubActive} onclick={onhub} aria-label={product.name}>
        <img class="hub-logo" src="/logo.png" alt="" />
      </button>
    {/if}
    {#each right as it (it.id)}{@render item(it)}{/each}
  </div>
</nav>

<style>
  .nav {
    flex-shrink: 0;
    height: calc(var(--nav-h) + var(--nav-gap) + var(--sab));
    padding: 0 var(--sp-2) calc(var(--nav-gap) + var(--sab));
  }
  /* «In the air»: the shared plate (mobile.css .m-bar), 8 from the sides. */
  .bar {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    grid-template-rows: 100%;
    height: 100%;
    padding: var(--sp-1) var(--sp-2);
  }
  /* The count of a tab is the .count-pill of 18 (base.css), .danger, on the icon corner in a ring of the pane. */
  .nav-count { position: absolute; top: -5px; left: calc(100% - 8px); box-shadow: 0 0 0 2px var(--pane-strong); }
  /* The product's apps: its logo alone, no word under it. */
  .hub-logo { width: 34px; height: 34px; object-fit: contain; }
</style>

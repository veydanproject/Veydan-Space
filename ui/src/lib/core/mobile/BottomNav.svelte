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

{#snippet item(it: NavItem)}
  {#if it.href}
    <a
      href={it.href}
      class="item"
      class:active={activeId === it.id}
      aria-current={activeId === it.id ? 'page' : undefined}
      data-sveltekit-replacestate={isTab(it.href) ? '' : undefined}
    >
      <Icon name={it.icon} size={22} />
      <span class="label">{$t(it.title)}</span>
    </a>
  {:else}
    <button type="button" class="item" class:active={activeId === it.id} onclick={it.onclick}>
      <Icon name={it.icon} size={22} />
      <span class="label">{$t(it.title)}</span>
    </button>
  {/if}
{/snippet}

<nav class="nav" style:--cols={items.length + (hub ? 1 : 0)}>
  {#each left as it (it.id)}{@render item(it)}{/each}
  {#if hub}
    <button type="button" class="item item--hub" class:active={hubActive} onclick={onhub} aria-label={product.name}>
      <span class="hub"><img src="/logo.png" alt="" /></span>
      <span class="label">{product.name}</span>
    </button>
  {/if}
  {#each right as it (it.id)}{@render item(it)}{/each}
</nav>

<style>
  .nav {
    flex-shrink: 0;
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    /* One row exactly as tall as the bar: an item taller than it (the hub's
       circle) sticks out upward, never below the bar and the screen. */
    grid-template-rows: 100%;
    height: calc(var(--nav-h) + var(--sab));
    padding-bottom: var(--sab);
    background: var(--m-nav);
    --hub-ring: var(--m-nav);
    border-top: 1px solid var(--border);
  }
  .item {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text-2);
    text-decoration: none;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    transition: color var(--dur-fast);
  }
  .label { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .item.active { color: var(--accent); }
  .item:active { color: var(--text); }
  .item--hub { justify-content: flex-end; padding-bottom: 7px; }
  .hub {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 50px;
    height: 50px;
    border-radius: 50%;
    background: var(--m-hub);
    box-shadow: 0 0 0 4px var(--hub-ring), var(--m-hub-shadow);
    transform: translateY(-10px);
    transition: transform var(--dur-fast), filter var(--dur-fast), background var(--dur-fast), box-shadow var(--dur-fast);
  }
  .hub img { width: 32px; height: 32px; object-fit: contain; }
  .item--hub.active .hub { filter: brightness(1.15); }
  .item--hub:active .hub { transform: translateY(-10px) scale(0.94); }
  .item--hub .label { margin-top: -8px; flex-shrink: 0; }
</style>

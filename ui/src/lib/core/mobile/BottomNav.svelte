<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { navItems } from '$lib/core/mobile/nav';
  import type { NavItem } from '$lib/core/module';

  interface Props {
    items: NavItem[];
    activeId: string;
    /** A wide window (core/ui/split): the same items as a column of sections at the left. */
    rail?: boolean;
  }

  let { items, activeId, rail = false }: Props = $props();

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
  {#if n > 0}<span class="count-pill sm nav-count" class:danger={it.danger}>{n > 99 ? "99+" : n}</span>{/if}
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
      <span class="m-bar-ico"><Icon name={it.icon} size={20} />{@render count(it)}</span>
      <span class="m-bar-label">{$t(it.title)}</span>
    </a>
  {:else}
    <button type="button" class="m-bar-item" class:active={activeId === it.id} onclick={it.onclick}>
      <span class="m-bar-ico"><Icon name={it.icon} size={20} />{@render count(it)}</span>
      <span class="m-bar-label">{$t(it.title)}</span>
    </button>
  {/if}
{/snippet}

{#if rail}
  <nav class="rail">
    {#each items as it (it.id)}{@render item(it)}{/each}
  </nav>
{:else}
  <!-- The bar floats over the glow; its slot keeps the room, so no screen scrolls under it. -->
  <nav class="nav">
    <div class="bar m-bar m-bar--nav">
      {#each items as it (it.id)}{@render item(it)}{/each}
    </div>
  </nav>
{/if}

<style>
  .nav {
    display: flex;
    justify-content: center;
    align-items: flex-end;
    flex-shrink: 0;
    height: calc(var(--nav-h) + var(--nav-gap) + var(--sab));
    /* On the field's column: its gutter, or the system's insets where they are wider. */
    padding: 0 max(var(--col-gutter, var(--sp-2)), var(--sar, 0px)) calc(var(--nav-gap) + var(--sab)) max(var(--col-gutter, var(--sp-2)), var(--sal, 0px));
    box-sizing: border-box;
    width: 100%;
    /* A keyboard covers the bar: its slot gives way whole, so a screen that lifts itself by --kb ends at the keyboard
       and not above an empty strip. A small inset (a floating keyboard's) covers nothing, the bar stays where it is. */
    margin-bottom: calc(-1 * clamp(0px, (var(--kb, 0px) - 120px) * 1000, var(--nav-h) + var(--nav-gap) + var(--sab)));
  }
  /* The plate and its items are the shared tab bar (mobile.css .m-bar--nav), 8 from the sides. */
  /* The outer tabs reach the sides of the screen and every tab its bottom edge (nav 8 + bar border 1 + padding 4; padding 4 + border 1 + gap). */
  .bar :global(.m-bar-item) { --hit-b: calc(var(--sp-1) + 1px + var(--nav-gap)); }
  .bar > :global(.m-bar-item:first-child) { --hit-l: calc(var(--sp-2) + 1px + var(--sp-1)); }
  .bar > :global(.m-bar-item:last-child) { --hit-r: calc(var(--sp-2) + 1px + var(--sp-1)); }
  /* The column: 72 wide past the system's insets, an item a tile of 56 under the icon and its caption,
     the current one on the accent's tile, a count at the corner of the icon. */
  .rail {
    flex: 0 0 auto;
    box-sizing: content-box;
    width: 72px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding: calc(var(--sat) + var(--sp-3)) 0 calc(var(--sab) + var(--sp-3)) var(--sal);
    overflow-y: auto;
    border-right: 1px solid var(--border);
    background: var(--pane-top);
  }
  .rail :global(.m-bar-item) {
    width: 64px;
    min-height: 56px;
    padding: var(--sp-1) 2px;
    gap: 2px;
    line-height: var(--lh-tight);
  }
  /* The chosen: its icon and label in the accent, on a step lighter than the column (lilac in the light). */
  .rail :global(.m-bar-item.active) { background: color-mix(in srgb, var(--text) 10%, transparent); color: var(--accent-text); font-weight: var(--fw-bold); }
  :global([data-theme='light']) .rail :global(.m-bar-item.active) { background: var(--accent-bg); }
  .rail :global(.m-bar-item.active .m-bar-ico) { color: var(--accent-text); }
  .rail :global(.nav-count) { position: absolute; top: -6px; left: calc(100% - 8px); box-shadow: 0 0 0 2px var(--pane-top); }
</style>

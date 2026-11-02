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
  }

  let { items, activeId }: Props = $props();

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
      <span class="m-bar-ico"><Icon name={it.icon} size={21} />{@render count(it)}</span>
      <span class="m-bar-label">{$t(it.title)}</span>
    </a>
  {:else}
    <button type="button" class="m-bar-item" class:active={activeId === it.id} onclick={it.onclick}>
      <span class="m-bar-ico"><Icon name={it.icon} size={21} />{@render count(it)}</span>
      <span class="m-bar-label">{$t(it.title)}</span>
    </button>
  {/if}
{/snippet}

<!-- The bar floats over the glow; its slot keeps the room, so no screen scrolls under it. -->
<nav class="nav">
  <div class="bar m-bar m-bar--nav">
    {#each items as it (it.id)}{@render item(it)}{/each}
  </div>
</nav>

<style>
  .nav {
    display: flex;
    justify-content: center;
    align-items: flex-end;
    flex-shrink: 0;
    height: calc(var(--nav-h) + var(--nav-gap) + var(--sab));
    padding: 0 var(--sp-2) calc(var(--nav-gap) + var(--sab));
    /* A keyboard covers the bar: its slot gives way whole, so a screen that lifts itself by --kb ends at the keyboard
       and not above an empty strip. A small inset (a floating keyboard's) covers nothing, the bar stays where it is. */
    margin-bottom: calc(-1 * clamp(0px, (var(--kb, 0px) - 120px) * 1000, var(--nav-h) + var(--nav-gap) + var(--sab)));
  }
  /* The plate and its items are the shared tab bar (mobile.css .m-bar--nav), 8 from the sides. */
  /* The outer tabs reach the sides of the screen and every tab its bottom edge (nav 8 + bar border 1 + padding 4; padding 4 + border 1 + gap). */
  .bar :global(.m-bar-item) { --hit-b: calc(var(--sp-1) + 1px + var(--nav-gap)); }
  .bar > :global(.m-bar-item:first-child) { --hit-l: calc(var(--sp-2) + 1px + var(--sp-1)); }
  .bar > :global(.m-bar-item:last-child) { --hit-r: calc(var(--sp-2) + 1px + var(--sp-1)); }
</style>

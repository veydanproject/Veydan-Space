<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  export type SettingsNavGroup = {
    label: string;
    items: { id: string; label: string }[];
  };

  let {
    groups,
    active,
    onselect,
  }: {
    groups: SettingsNavGroup[];
    active: string;
    onselect: (id: string) => void;
  } = $props();

  let navEl = $state<HTMLElement | null>(null);

  // The nav scrolls inside itself (it is taller than a small window): the
  // active entry follows the page into view. Only the nav scrolls, never the page.
  $effect(() => {
    void active;
    const nav = navEl;
    const item = nav?.querySelector<HTMLElement>('.nav-item.active');
    if (!nav || !item) return;
    const n = nav.getBoundingClientRect();
    const r = item.getBoundingClientRect();
    if (r.top < n.top) nav.scrollTop -= n.top - r.top + 8;
    else if (r.bottom > n.bottom) nav.scrollTop += r.bottom - n.bottom + 8;
  });
</script>

<!-- Sticky section index for the settings page -->
<nav class="settings-nav" bind:this={navEl}>
  {#each groups as group (group.label)}
    <div class="nav-group">
      <div class="section-label">{group.label}</div>
      {#each group.items as item (item.id)}
        <button
          type="button"
          class="nav-item"
          class:active={active === item.id}
          title={item.label}
          onclick={() => onselect(item.id)}
        >
          {item.label}
        </button>
      {/each}
    </div>
  {/each}
</nav>

<style>
  .settings-nav {
    position: sticky;
    top: 0;
    align-self: start;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    width: 200px;
    padding-top: var(--sp-2);
    /* The height of the page's scroller (set by the settings page), less its
       padding: the window's title bar, frame and banners are not the nav's to count. */
    max-height: calc(var(--settings-view-h, 100vh) - var(--sp-6) * 2);
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .nav-group { display: flex; flex-direction: column; gap: 2px; }
  .section-label { padding: 0 var(--sp-3); margin-bottom: var(--sp-1); }
  .nav-item {
    display: block; width: 100%; text-align: left;
    padding: 0.3rem var(--sp-3);
    /* One line in either weight: the bold active entry does not wrap and push the rest down. */
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    background: none; border: none; border-radius: var(--radius-sm);
    color: var(--text-2); font-size: var(--fs-sm); cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .nav-item:hover { background: var(--surface-2); color: var(--text); }
  .nav-item.active { background: var(--accent-bg); color: var(--accent-text); font-weight: var(--fw-semibold); }
</style>

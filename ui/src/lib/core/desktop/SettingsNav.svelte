<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';

  export type SettingsNavGroup = {
    label: string;
    items: { id: string; label: string; icon: string }[];
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

  // The pane of the nav scrolls (it is taller than a small window): the
  // active entry stays in view.
  $effect(() => {
    void active;
    navEl?.querySelector<HTMLElement>('.nav-item.active')?.scrollIntoView({ block: 'nearest' });
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
          <Icon name={item.icon} size={20} />
          <span class="nav-label">{item.label}</span>
        </button>
      {/each}
    </div>
  {/each}
</nav>

<style>
  /* The sections of Settings, in their own pane (the page scrolls it). */
  .settings-nav {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }
  .nav-group { display: flex; flex-direction: column; gap: 2px; }
  /* The kit's group label: plain case, 12/600. */
  .section-label { padding: 0 var(--sp-3); margin-bottom: var(--sp-1); font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); text-transform: none; letter-spacing: 0; }
  .nav-item {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 40px; text-align: left;
    padding: 0 var(--sp-3);
    background: none; border: none; border-radius: var(--radius);
    color: var(--text-2); font-size: var(--fs-base); font-weight: var(--fw-semibold); cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .nav-item:hover { background: var(--surface-hover); color: var(--text); }
  .nav-item.active { background: var(--accent-bg); color: var(--accent-text); }
  .nav-item :global(.icon) { flex-shrink: 0; }
  .nav-label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>

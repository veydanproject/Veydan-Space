<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The footer of the notes sidebar, one short row on the Notes page and in the
  workspace's Notes drawer: hide the sidebar, sync, lock, and a More menu with
  the rarely used actions (open the folder, import, export, the notes window).
-->
<script lang="ts">
  import { api } from '$lib/notes/api';
  import { appLock } from '$lib/core/lock/store.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import NoteSyncButton from './NoteSyncButton.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    ontoggle: () => void;
    onimport: () => void;
    onexport: () => void;
    /** Offer the separate notes window (not inside that window itself). */
    windowItem?: boolean;
  }

  let { ontoggle, onimport, onexport, windowItem = false }: Props = $props();

  let menu = $state<{ x: number; y: number } | null>(null);
  const items = $derived<MenuEntry[]>([
    { label: $t('notes_btn_open_folder'), icon: 'folder-open', onselect: () => void api.notes.openFolder() },
    { label: $t('notes_btn_import'), icon: 'upload', onselect: onimport },
    { label: $t('notes_btn_export'), icon: 'download', onselect: onexport },
    ...(windowItem
      ? [
          { type: 'separator' } as const,
          { label: $t('notes_btn_open_window'), icon: 'file-text', onselect: () => void api.notes.openWindow($t('nav_notes')) },
        ]
      : []),
  ]);

  function openMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Beside the button, its bottom edge level with the button's (a row is 34px, a separator 9px)
    const height = items.reduce((h, i) => h + (i.type === 'separator' ? 9 : 34), 10);
    menu = { x: r.right + 6, y: r.bottom - height };
  }
</script>

{#if menu}
  <ContextMenu open x={menu.x} y={menu.y} {items} onclose={() => (menu = null)} />
{/if}

<div class="sidebar-footer">
  <button class="icon-btn" onclick={ontoggle} title={$t('notes_btn_toggle_sidebar')} aria-label={$t('notes_btn_toggle_sidebar')}>
    <Icon name="sidebar" size={14} />
  </button>
  <NoteSyncButton />
  {#if appLock.status.enabled}
    <button class="icon-btn" title={$t('lock_now')} aria-label={$t('lock_now')} onclick={() => appLock.lock()}>
      <Icon name="lock" size={14} />
    </button>
  {/if}
  <button
    class="icon-btn"
    class:active={menu !== null}
    title={$t('notes_btn_more')}
    aria-label={$t('notes_btn_more')}
    aria-haspopup="menu"
    onclick={openMenu}
  >
    <Icon name="more-horizontal" size={14} />
  </button>
</div>

<style>
  .sidebar-footer {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: var(--sp-2) var(--sp-3) var(--sp-3);
    margin-top: auto;
    flex-shrink: 0;
  }
  .sidebar-footer .icon-btn { width: 32px; height: 32px; }
</style>

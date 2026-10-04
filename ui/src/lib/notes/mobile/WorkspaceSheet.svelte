<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import type { NavChild } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    workspaces: NavChild[];
    /** Workspace ids the note is in now. */
    current: string[];
    busy?: boolean;
    onclose: () => void;
    ontoggle: (workspaceId: string, on: boolean) => void;
  }

  let { open, workspaces, current, busy = false, onclose, ontoggle }: Props = $props();

  const selected = $derived(new Set(current));
</script>

<BottomSheet {open} title={$t('notes_workspace_add')} {onclose}>
  {#if workspaces.length === 0}
    <p class="empty">{$t('notes_workspace_empty')}</p>
  {:else}
    <div class="list">
      {#each workspaces as w (w.id)}
        <button type="button" class="pick" class:on={selected.has(w.id)} disabled={busy} onclick={() => ontoggle(w.id, !selected.has(w.id))}>
          <span class="m-doc" style:--c={w.color}><Icon name="layers" size={16} /></span>
          <span class="name">{w.name}</span>
          {#if selected.has(w.id)}<Icon name="check" size={18} />{/if}
        </button>
      {/each}
    </div>
  {/if}
</BottomSheet>

<style>
  .empty { margin: var(--sp-4); color: var(--text-3); text-align: center; }
  .list { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; }
  .pick {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 48px;
    padding: 0 var(--sp-2);
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 15px;
    text-align: left;
  }
  .pick.on { background: var(--accent-bg); }
  .pick:disabled { opacity: 0.5; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>

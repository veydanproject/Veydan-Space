<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Autocomplete for `{{placeholder}}` while editing a template note. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { searchPlaceholders, placeholderMarkup } from '$lib/notes/templates';

  interface Props {
    query: string;
    /** Highlighted row; moved by arrow keys via `handleKeydown` */
    index?: number;
    /** Current value shown next to the name; omitted inside template notes */
    preview?: (name: string) => string;
    onpick: (name: string) => void;
  }

  let { query, index = $bindable(0), preview, onpick }: Props = $props();

  // A short editor pane caps the list (it scrolls): the row picked by the keys stays in view.
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void index;
    listEl?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest' });
  });

  const rows = $derived(searchPlaceholders(query).slice(0, 10));

  /** Arrow navigation and Enter/Tab pick; returns true when the key was consumed. */
  export function handleKeydown(e: KeyboardEvent): boolean {
    if (!rows.length) return false;
    if (e.key === 'ArrowDown') { e.preventDefault(); index = (index + 1) % rows.length; return true; }
    if (e.key === 'ArrowUp') { e.preventDefault(); index = (index - 1 + rows.length) % rows.length; return true; }
    if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault();
      onpick(rows[index]);
      return true;
    }
    return false;
  }
</script>

<div class="picker" role="listbox" bind:this={listEl}>
  {#each rows as name, i (name)}
    <button class="row" class:active={i === index} role="option" aria-selected={i === index} onmousedown={(e) => { e.preventDefault(); onpick(name); }}>
      <Icon name="code" size={12} />
      <span class="title">{placeholderMarkup(name)}</span>
      {#if preview}
        {@const value = preview(name)}
        <span class="value" class:missing={!value}>{value || '—'}</span>
      {/if}
    </button>
  {/each}
  {#if rows.length === 0}
    <div class="row empty">{$t('note_placeholder_none')}</div>
  {/if}
</div>

<style>
  .picker {
    position: absolute;
    left: var(--sp-4);
    bottom: var(--sp-3);
    z-index: 5;
    /* Inside the editor pane, whatever its height: the pane clips what sticks out. */
    max-height: calc(100% - 2 * var(--sp-3));
    overflow-y: auto;
    min-width: 260px;
    max-width: 420px;
    padding: 0.25rem;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
  }
  .row {
    display: flex; align-items: center; gap: 0.4rem; width: 100%;
    padding: 0.35rem 0.5rem; border: 0; border-radius: 4px;
    background: transparent; color: var(--text); font-size: var(--fs-sm);
    font-family: var(--font-mono); text-align: left; cursor: pointer;
  }
  .row.active, .row:hover { background: var(--surface-2); }
  .row.empty { color: var(--text-3); cursor: default; font-family: inherit; }
  .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .value {
    margin-left: auto; padding-left: 0.6rem; max-width: 180px;
    font-family: inherit; font-size: var(--fs-2xs); color: var(--text-2);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .value.missing { color: var(--text-3); }
</style>

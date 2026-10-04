<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import type { EditAction } from '$lib/notes/markdown-edit';
  import { t, type TranslationKey } from '$lib/core/i18n';

  /** `rich`: WYSIWYG editing (default); `source`: raw Markdown textarea */
  export type EditorMode = 'rich' | 'source';

  interface Props {
    mode: EditorMode;
    findOpen: boolean;
    disabled?: boolean;
    onaction: (a: EditAction) => void;
    onmode: (m: EditorMode) => void;
    ontogglefind: () => void;
  }

  let { mode, findOpen, disabled = false, onaction, onmode, ontogglefind }: Props = $props();

  interface TbItem { action: EditAction; icon: string; label: TranslationKey | 'H1' | 'H2' | 'H3'; key?: string }

  const groups: TbItem[][] = [
    [
      { action: 'h1', icon: 'heading', label: 'H1' },
      { action: 'h2', icon: 'heading', label: 'H2' },
      { action: 'h3', icon: 'heading', label: 'H3' },
    ],
    [
      { action: 'bold', icon: 'bold', label: 'note_tb_bold', key: 'Ctrl+B' },
      { action: 'italic', icon: 'italic', label: 'note_tb_italic', key: 'Ctrl+I' },
      { action: 'strike', icon: 'strikethrough', label: 'note_tb_strike' },
      { action: 'code', icon: 'code', label: 'note_tb_code' },
    ],
    [
      { action: 'ul', icon: 'list', label: 'note_tb_ul' },
      { action: 'ol', icon: 'list-ordered', label: 'note_tb_ol' },
      { action: 'task', icon: 'list-checks', label: 'note_tb_task' },
      { action: 'quote', icon: 'quote', label: 'note_tb_quote' },
    ],
    [
      { action: 'link', icon: 'link', label: 'note_tb_link', key: 'Ctrl+K' },
      { action: 'codeblock', icon: 'file-code', label: 'note_tb_codeblock' },
      { action: 'hr', icon: 'minus', label: 'note_tb_hr' },
    ],
  ];

  const modes: { id: EditorMode; icon?: string; text?: string; label: TranslationKey }[] = [
    { id: 'rich', icon: 'edit', label: 'note_mode_rich' },
    { id: 'source', text: 'MD', label: 'note_mode_source' },
  ];

  const title = (item: TbItem) => {
    const base = item.label.startsWith('note_') ? $t(item.label as TranslationKey) : item.label;
    return item.key ? `${base} (${item.key})` : base;
  };
</script>

<div class="toolbar">
  {#each groups as group, gi (gi)}
    {#if gi > 0}<span class="tb-sep"></span>{/if}
    {#each group as item (item.action)}
      <button
        class="tb-btn"
        class:tb-h={item.action.startsWith('h')}
        title={title(item)}
        {disabled}
        onmousedown={(e) => e.preventDefault()}
        onclick={() => onaction(item.action)}
      >
        {#if item.action === 'h1' || item.action === 'h2' || item.action === 'h3'}
          <span class="tb-h-label">{item.label}</span>
        {:else}
          <Icon name={item.icon} size={14} />
        {/if}
      </button>
    {/each}
  {/each}
  <span class="tb-sep"></span>
  <button
    class="tb-btn"
    class:active={findOpen}
    title="{$t('note_find_title')} (Ctrl+F)"
    onmousedown={(e) => e.preventDefault()}
    onclick={ontogglefind}
  >
    <Icon name="search" size={14} />
  </button>
  <div class="tb-modes">
    {#each modes as m (m.id)}
      <button
        class="tb-mode-toggle"
        class:active={mode === m.id}
        title={$t(m.label)}
        onclick={() => onmode(m.id)}
      >
        {#if m.icon}
          <Icon name={m.icon} size={13} />
        {:else}
          <span class="tb-h-label">{m.text}</span>
        {/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.1rem;
    padding: 0.3rem var(--sp-2);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    flex-wrap: wrap;
  }
  .tb-btn {
    background: none;
    border: none;
    border-radius: 4px;
    padding: 0.2rem 0.4rem;
    cursor: pointer;
    color: var(--text-2);
    font-size: var(--fs-sm);
    line-height: 1;
    transition: background 0.12s, color 0.12s;
    min-width: 1.8rem;
    height: 1.7rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .tb-btn:hover:not(:disabled) { background: var(--surface-2); color: var(--text); }
  .tb-btn:disabled { opacity: 0.4; cursor: default; }
  .tb-btn.active { color: var(--accent); background: var(--accent-bg); }
  .tb-h-label {
    font-family: var(--font-mono);
    font-size: var(--fs-2xs);
    font-weight: var(--fw-bold);
    letter-spacing: 0.02em;
  }
  .tb-sep {
    width: 1px;
    height: 1.1rem;
    background: var(--border);
    margin: 0 0.2rem;
    flex-shrink: 0;
  }

  /* Pushed right; a wrapped toggle stays at the right too */
  .tb-modes {
    margin-left: auto;
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .tb-mode-toggle {
    background: none;
    border: none;
    padding: 0.2rem 0.5rem;
    color: var(--text-2);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    transition: background 0.12s, color 0.12s;
  }
  .tb-mode-toggle + .tb-mode-toggle { border-left: 1px solid var(--border); }
  .tb-mode-toggle:hover { background: var(--surface-2); color: var(--text); }
  .tb-mode-toggle.active { color: var(--accent); background: var(--accent-bg); }
</style>

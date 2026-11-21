<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Horizontally scrollable Markdown formatting bar for the mobile note editor. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { scrollTabs } from '$lib/core/ui/scrollTabs';
  import type { EditAction } from '$lib/notes/markdown-edit';
  import type { EditorMode } from '$lib/notes/mobile/notes-editor';
  import { t, type Key } from '$lib/core/mobile/i18n';

  interface Props {
    mode: EditorMode;
    disabled?: boolean;
    onaction: (a: EditAction) => void;
    onmode: (m: EditorMode) => void;
  }

  let { mode, disabled = false, onaction, onmode }: Props = $props();

  interface Item { action: EditAction; icon: string; label: Key | 'H1' | 'H2' | 'H3' }

  const groups: Item[][] = [
    [
      { action: 'h1', icon: 'h-1', label: 'H1' },
      { action: 'h2', icon: 'h-2', label: 'H2' },
      { action: 'h3', icon: 'h-3', label: 'H3' },
    ],
    [
      { action: 'bold', icon: 'bold', label: 'note_tb_bold' },
      { action: 'italic', icon: 'italic', label: 'note_tb_italic' },
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
      { action: 'link', icon: 'link', label: 'note_tb_link' },
      { action: 'codeblock', icon: 'file-code', label: 'note_tb_codeblock' },
      { action: 'hr', icon: 'separator-horizontal', label: 'note_tb_hr' },
    ],
  ];

  const isHeading = (a: EditAction) => a === 'h1' || a === 'h2' || a === 'h3';
  const label = (item: Item) => (isHeading(item.action) ? item.label : $t(item.label as Key));
</script>

<div class="bar hit-row" use:scrollTabs={{ fadeStart: true }}>
  {#each groups as group, gi (gi)}
    {#if gi > 0}<span class="sep"></span>{/if}
    {#each group as item (item.action)}
      <button
        type="button"
        class="icon-btn"
        {disabled}
        aria-label={label(item)}
        onpointerdown={(e) => e.preventDefault()}
        onclick={() => onaction(item.action)}
      >
        {#if isHeading(item.action)}
          <span class="h-label">{item.label}</span>
        {:else}
          <Icon name={item.icon} size={22} />
        {/if}
      </button>
    {/each}
  {/each}
  <span class="spacer"></span>
  <div class="modes" role="group" aria-label={$t('notes_mode')}>
    <button type="button" class="icon-btn" class:on={mode === 'rich'} onclick={() => onmode('rich')} aria-label={$t('notes_mode_rich')}>
      <Icon name="pencil" size={22} />
    </button>
    <button type="button" class="icon-btn" class:on={mode === 'md'} onclick={() => onmode('md')} aria-label={$t('notes_mode_md')}>
      <span class="h-label">MD</span>
    </button>
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    overflow-x: auto;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
    margin-inline: calc(-1 * var(--sp-4));
    padding-inline: var(--sp-4);
    --hit-row: 2px;
  }
  .bar::-webkit-scrollbar { display: none; }
  /* A clipped edge fades, only while there is more of the row on that side (scrollTabs marks it). */
  .bar:global(.fade-end) { -webkit-mask-image: linear-gradient(to right, black 85%, transparent); mask-image: linear-gradient(to right, black 85%, transparent); }
  .bar:global(.fade-start) { -webkit-mask-image: linear-gradient(to left, black 85%, transparent); mask-image: linear-gradient(to left, black 85%, transparent); }
  .bar:global(.fade-start.fade-end) {
    -webkit-mask-image: linear-gradient(to right, transparent, black 15%, black 85%, transparent);
    mask-image: linear-gradient(to right, transparent, black 15%, black 85%, transparent);
  }
  /* The buttons are the common .icon-btn: 40, its zone 44 (mobile.css); 4 between two, their zones meet and do not cross. */
  .h-label {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: var(--fw-bold);
  }
  .sep {
    flex-shrink: 0;
    width: 1px;
    height: 18px;
    margin: 0 4px;
    background: var(--border);
  }
  .spacer { flex: 1; min-width: var(--sp-2); }
  /* The editor's mode: two icon buttons, the chosen one .on. */
  .modes { flex-shrink: 0; display: inline-flex; gap: var(--sp-1); }
</style>

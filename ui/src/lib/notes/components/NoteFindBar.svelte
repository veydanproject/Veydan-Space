<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    textarea: HTMLTextAreaElement | null;
    content: string;
    readonly?: boolean;
    onreplace: (content: string) => void;
    onclose: () => void;
  }

  let { textarea, content, readonly = false, onreplace, onclose }: Props = $props();

  let query = $state('');
  let replacement = $state('');
  let caseSensitive = $state(false);
  let current = $state(-1);
  let inputEl: HTMLInputElement | null = $state(null);

  $effect(() => { inputEl?.focus(); });

  /** Return focus to the query field (Ctrl+F while the bar is already open). */
  export function focus() {
    inputEl?.focus();
    inputEl?.select();
  }

  function escapeRe(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  const matches = $derived.by((): { start: number; end: number }[] => {
    if (!query) return [];
    const re = new RegExp(escapeRe(query), caseSensitive ? 'g' : 'gi');
    const out: { start: number; end: number }[] = [];
    for (const m of content.matchAll(re)) out.push({ start: m.index!, end: m.index! + m[0].length });
    return out;
  });

  // Reset position when query or text changes
  $effect(() => { matches; current = -1; });

  function select(idx: number) {
    if (!textarea || matches.length === 0) return;
    current = ((idx % matches.length) + matches.length) % matches.length;
    const { start, end } = matches[current];
    // WebKit paints the selection only in a focused textarea
    textarea.focus();
    textarea.setSelectionRange(start, end);
    const linesBefore = content.slice(0, start).split('\n').length - 1;
    const lineHeight = parseFloat(getComputedStyle(textarea).lineHeight) || 22;
    textarea.scrollTop = Math.max(0, linesBefore * lineHeight - textarea.clientHeight / 3);
  }

  const next = () => select(current + 1);
  const prev = () => select(current - 1);

  function replaceOne() {
    if (readonly || matches.length === 0) return;
    if (current < 0) { select(0); return; }
    const { start, end } = matches[current];
    onreplace(content.slice(0, start) + replacement + content.slice(end));
    const target = current;
    queueMicrotask(() => select(target));
  }

  function replaceAll() {
    if (readonly || matches.length === 0) return;
    const re = new RegExp(escapeRe(query), caseSensitive ? 'g' : 'gi');
    onreplace(content.replace(re, replacement));
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); onclose(); }
    else if (e.key === 'Enter') { e.preventDefault(); e.shiftKey ? prev() : next(); }
  }
</script>

<div class="find-bar" role="search">
  <div class="find-field">
    <Icon name="search" size={13} />
    <input
      bind:this={inputEl}
      bind:value={query}
      type="text"
      placeholder={$t('note_find_placeholder')}
      onkeydown={onKey}
    />
    <span class="find-count">{matches.length ? `${current + 1 || 0}/${matches.length}` : '0'}</span>
    <button class="fb-btn" class:active={caseSensitive} title={$t('note_find_case')} onclick={() => (caseSensitive = !caseSensitive)}>
      <Icon name="case-sensitive" size={13} />
    </button>
    <button class="fb-btn" title={$t('note_find_prev')} onclick={prev} disabled={!matches.length}><Icon name="chevron-up" size={13} /></button>
    <button class="fb-btn" title={$t('note_find_next')} onclick={next} disabled={!matches.length}><Icon name="chevron-down" size={13} /></button>
  </div>
  {#if !readonly}
    <div class="find-field">
      <Icon name="replace" size={13} />
      <input bind:value={replacement} type="text" placeholder={$t('note_replace_placeholder')} onkeydown={onKey} />
      <button class="fb-text" onclick={replaceOne} disabled={!matches.length}>{$t('note_replace_one')}</button>
      <button class="fb-text" onclick={replaceAll} disabled={!matches.length}>{$t('note_replace_all')}</button>
    </div>
  {/if}
  <button class="fb-btn" title={$t('notes_btn_cancel')} onclick={onclose}><Icon name="x" size={13} /></button>
</div>

<style>
  .find-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0.3rem var(--sp-3);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    flex-wrap: wrap;
  }
  .find-field {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0 0.4rem;
    height: 28px;
    color: var(--text-3);
    flex: 1;
    min-width: 200px;
  }
  .find-field:focus-within { border-color: var(--accent-border); }
  /* Reset the global input chrome from base.css: the field draws the border */
  .find-field input {
    flex: 1 1 0;
    width: 0;
    min-width: 0;
    height: 100%;
    padding: 0;
    background: none;
    border: none;
    border-radius: 0;
    outline: none;
    font-size: var(--fs-sm);
    color: var(--text);
  }
  .find-field input:focus { box-shadow: none; }
  .find-count {
    font-size: var(--fs-2xs);
    font-family: var(--font-mono);
    color: var(--text-3);
    white-space: nowrap;
  }
  .fb-btn, .fb-text {
    background: none;
    border: none;
    color: var(--text-2);
    cursor: pointer;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    padding: 0.15rem 0.3rem;
  }
  .fb-text { font-size: var(--fs-2xs); white-space: nowrap; }
  .fb-btn:hover:not(:disabled), .fb-text:hover:not(:disabled) { background: var(--surface-2); color: var(--text); }
  .fb-btn:disabled, .fb-text:disabled { opacity: 0.4; cursor: default; }
  .fb-btn.active { color: var(--accent); }
</style>

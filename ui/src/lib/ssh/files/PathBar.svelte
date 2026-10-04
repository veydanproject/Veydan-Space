<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    path: string;
    showHidden: boolean;
    onnavigate: (path: string) => void;
    onup: () => void;
    onrefresh: () => void;
    ontogglehidden: () => void;
  }

  let { path, showHidden, onnavigate, onup, onrefresh, ontogglehidden }: Props = $props();

  let editing = $state(false);
  let editValue = $state('');
  let inputEl = $state<HTMLInputElement | null>(null);

  // "/var/www/html" → clickable crumbs with cumulative paths
  let crumbs = $derived.by(() => {
    const parts = path.split('/').filter(Boolean);
    let acc = '';
    return parts.map((name) => {
      acc += '/' + name;
      return { name, path: acc };
    });
  });

  async function startEdit() {
    editValue = path;
    editing = true;
    await tick();
    inputEl?.focus();
    inputEl?.select();
  }

  function submitEdit() {
    editing = false;
    const value = editValue.trim();
    if (value && value !== path) onnavigate(value);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') submitEdit();
    if (e.key === 'Escape') editing = false;
  }
</script>

<div class="path-bar">
  <button class="icon-btn" title={$t('files_up')} onclick={onup}>
    <Icon name="arrow-up" size={13} />
  </button>

  {#if editing}
    <input
      bind:this={inputEl}
      class="path-input"
      type="text"
      bind:value={editValue}
      onkeydown={onKeydown}
      onblur={submitEdit}
      spellcheck="false"
    />
  {:else}
    <div
      class="crumbs"
      title={path}
      role="button"
      tabindex="0"
      onclick={startEdit}
      onkeydown={(e) => {
        if (e.target !== e.currentTarget) return; // ignore keys bubbling from crumb buttons
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); startEdit(); }
      }}
    >
      <button
        class="crumb crumb-root"
        onclick={(e) => { e.stopPropagation(); onnavigate('/'); }}
      >/</button>
      {#each crumbs as c, i (c.path)}
        {#if i > 0}<span class="crumb-sep">/</span>{/if}
        <button
          class="crumb"
          class:current={i === crumbs.length - 1}
          onclick={(e) => { e.stopPropagation(); onnavigate(c.path); }}
        >{c.name}</button>
      {/each}
    </div>
  {/if}

  <button
    class="icon-btn"
    class:active={showHidden}
    title={showHidden ? $t('files_hide_hidden') : $t('files_show_hidden')}
    onclick={ontogglehidden}
  >
    <Icon name={showHidden ? 'eye' : 'eye-off'} size={13} />
  </button>
  <button class="icon-btn" title={$t('files_refresh')} onclick={onrefresh}>
    <Icon name="refresh-cw" size={13} />
  </button>
</div>

<style>
  .path-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    min-width: 0;
  }

  .crumbs {
    flex: 1;
    display: flex;
    align-items: center;
    min-width: 0;
    overflow: hidden;
    height: var(--control-h);
    padding: 0 var(--sp-2);
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    cursor: text;
    text-align: left;
    /* Long paths: keep the tail (current dir) visible */
    justify-content: flex-start;
    flex-wrap: nowrap;
  }
  .crumbs:hover { border-color: var(--border-2); }

  .crumb {
    background: none;
    border: none;
    padding: 2px 1px;
    color: var(--text-2);
    font-size: var(--fs-sm);
    font-family: var(--font-mono);
    cursor: pointer;
    white-space: nowrap;
    border-radius: var(--radius-sm);
  }
  .crumb:hover { color: var(--accent-text); }
  .crumb.current { color: var(--text); font-weight: var(--fw-semibold); }
  .crumb-root { padding-right: 0; }
  .crumb-sep {
    color: var(--text-3);
    font-size: var(--fs-sm);
    font-family: var(--font-mono);
    flex-shrink: 0;
  }

  .path-input {
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--sp-2);
    background: var(--surface-3);
    border: 1px solid var(--accent-border);
    border-radius: var(--radius-field);
    color: var(--text);
    font-size: var(--fs-sm);
    font-family: var(--font-mono);
    outline: none;
  }

  .icon-btn.active { color: var(--accent-text); background: var(--accent-bg); }
</style>

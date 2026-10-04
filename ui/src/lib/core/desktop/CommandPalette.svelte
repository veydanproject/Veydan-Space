<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Global command palette (Ctrl+Shift+P / Ctrl+P) over the command registry. -->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { commands, type Command } from '$lib/core/commands';
  import { directory } from '$lib/core/directory';
  import { paletteStore } from '$lib/core/store/palette.svelte';
  import { matches } from '$lib/core/keybindings';

  const open = $derived(paletteStore.open);
  let query = $state('');
  let index = $state(0);
  let error = $state<string | null>(null);
  let inputEl: HTMLInputElement | null = $state(null);
  /** Bumped on open so providers re-read their data */
  let version = $state(0);

  const rows = $derived.by((): Command[] => {
    void version;
    return commands.search(query);
  });

  $effect(() => { void rows; index = 0; });

  // Reset and focus whenever the palette is opened, from the hotkey or any button.
  // untrack: store loading must not re-run this effect
  $effect(() => {
    if (!open) return;
    untrack(() => {
      query = '';
      error = null;
      version++;
      void directory.ensureLoaded().then(() => version++);
      void tick().then(() => inputEl?.focus());
    });
  });

  const hide = () => paletteStore.hide();

  async function run(cmd: Command) {
    hide();
    try {
      await cmd.run();
    } catch (e) {
      error = formatError(e);
      paletteStore.show();
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (matches(e, 'app.palette')) {
      e.preventDefault();
      paletteStore.toggle();
    }
  }

  function onInputKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); hide(); return; }
    if (!rows.length) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); index = (index + 1) % rows.length; }
    else if (e.key === 'ArrowUp') { e.preventDefault(); index = (index - 1 + rows.length) % rows.length; }
    else if (e.key === 'Enter') { e.preventDefault(); void run(rows[index]); }
  }

  /** Group label shown before the first row of each group. */
  function groupStart(i: number): string | null {
    const g = rows[i].group;
    return i === 0 || rows[i - 1].group !== g ? g : null;
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" onmousedown={hide}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="palette" role="dialog" aria-label={$t('cmd_title')} onmousedown={(e) => e.stopPropagation()}>
      <div class="input-row">
        <Icon name="search" size={14} />
        <input
          bind:this={inputEl}
          bind:value={query}
          type="text"
          placeholder={$t('cmd_placeholder')}
          onkeydown={onInputKeydown}
          spellcheck="false"
        />
        <kbd>Esc</kbd>
      </div>
      <div class="list" role="listbox">
        {#each rows as cmd, i (cmd.id)}
          {@const group = groupStart(i)}
          {#if group}<div class="group">{group}</div>{/if}
          <button
            class="row"
            class:active={i === index}
            role="option"
            aria-selected={i === index}
            onmouseenter={() => (index = i)}
            onclick={() => run(cmd)}
          >
            <Icon name={cmd.icon ?? 'zap'} size={13} />
            <span class="title">{cmd.title}</span>
          </button>
        {/each}
        {#if rows.length === 0}
          <div class="empty">{$t('cmd_empty')}</div>
        {/if}
      </div>
      {#if error}<div class="error">{error}</div>{/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0); z-index: 1000;
    background: rgba(0, 0, 0, 0.35);
    display: flex; justify-content: center; align-items: flex-start;
    padding-top: 12vh;
  }
  .palette {
    width: min(560px, 92vw);
    max-height: 70vh;
    display: flex; flex-direction: column;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 10px);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }
  .input-row {
    display: flex; align-items: center; gap: 0.5rem;
    padding: 0.5rem 0.8rem;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
  }
  /* Reset base.css input styles: no box, same size as the notes search field */
  .input-row input {
    flex: 1; width: auto; min-width: 0; height: auto;
    border: 0; border-radius: 0; background: transparent; box-shadow: none; outline: none;
    color: var(--text); font-size: var(--fs-sm); line-height: 1.4; padding: 0.2rem 0;
  }
  .input-row input:focus { border: 0; box-shadow: none; outline: none; }
  kbd {
    font-size: var(--fs-2xs); color: var(--text-3);
    border: 1px solid var(--border); border-radius: 4px; padding: 0 0.3rem;
  }
  .list { overflow-y: auto; padding: 0.3rem; }
  .group {
    padding: 0.4rem 0.6rem 0.15rem;
    font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.06em;
    color: var(--text-3); font-weight: 600;
  }
  .row {
    display: flex; align-items: center; gap: 0.5rem; width: 100%;
    padding: 0.4rem 0.6rem; border: 0; border-radius: 6px;
    background: transparent; color: var(--text); text-align: left; cursor: pointer;
    font-size: var(--fs-sm);
  }
  .row.active { background: color-mix(in srgb, var(--accent) 15%, transparent); }
  .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { padding: var(--sp-4); text-align: center; color: var(--text-3); font-size: var(--fs-sm); }
  .error { padding: 0.5rem 0.8rem; border-top: 1px solid var(--border); color: var(--danger-text); font-size: var(--fs-xs); }
</style>

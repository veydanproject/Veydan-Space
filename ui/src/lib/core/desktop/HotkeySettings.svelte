<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { registry } from '$lib/core/registry';
  import {
    HOTKEY_GROUPS,
    assignChord,
    captureEvent,
    clearCommand,
    formatCommand,
    isOverridden,
    keybindingOverrides,
    resetCommand,
    shortcutCapture,
    type CommandId,
  } from '$lib/core/keybindings';

  const GROUP_LABEL: Record<(typeof HOTKEY_GROUPS)[number]['id'], TranslationKey> = {
    edit: 'hotkey_group_edit',
    editor: 'hotkey_group_editor',
    app: 'hotkey_group_app',
  };

  const COMMAND_LABEL: Record<CommandId, TranslationKey> = {
    'edit.undo': 'hotkey_edit_undo',
    'edit.redo': 'hotkey_edit_redo',
    'editor.bold': 'hotkey_editor_bold',
    'editor.italic': 'hotkey_editor_italic',
    'editor.link': 'hotkey_editor_link',
    'editor.find': 'hotkey_editor_find',
    'editor.save': 'hotkey_editor_save',
    'app.palette': 'hotkey_app_palette',
    'app.inspector': 'hotkey_app_inspector',
  };

  // The core's groups, and those of the modules that are switched on.
  const groups = $derived(
    HOTKEY_GROUPS.filter((group) => !group.module || registry.active.some((m) => m.hotkeys?.includes(group.id))),
  );

  let recording = $state<CommandId | null>(null);
  let rowError = $state<{ id: CommandId; text: string } | null>(null);


  function keysOf(id: CommandId): string {
    return formatCommand(id, $keybindingOverrides) || $t('hotkey_unbound');
  }

  function start(id: CommandId) {
    recording = id;
    rowError = null;
  }

  $effect(() => {
    const id = recording;
    shortcutCapture.set(id !== null);
    if (!id) return;
    const translate = $t;
    const onKey = (e: KeyboardEvent) => {
      const result = captureEvent(e);
      if (result.kind === 'ignore') return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (result.kind === 'cancel') { recording = null; return; }
      if (result.kind === 'clear') { clearCommand(id); recording = null; return; }
      if (result.kind === 'reject') {
        rowError = { id, text: translate('hotkey_need_modifier') };
        return;
      }
      const other = assignChord(id, result.chord);
      if (other) {
        rowError = { id, text: translate('hotkey_conflict', { name: translate(COMMAND_LABEL[other]) }) };
        return;
      }
      rowError = null;
      recording = null;
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      shortcutCapture.set(false);
    };
  });
</script>

<!-- The hint over the groups; each group its own card under a caption. A shortcut shows as key chips,
     one per chord; the row being recorded is tinted, its error under the command's name. -->
<div class="hotkeys">
  <p class="muted hint">{$t('hotkey_hint')}</p>

  {#each groups as group (group.id)}
    <section class="group" aria-labelledby="hotkey-group-{group.id}">
      <div class="caption" id="hotkey-group-{group.id}">{$t(GROUP_LABEL[group.id])}</div>
      <div class="card list">
        {#each group.commands as id (id)}
          {@const keys = formatCommand(id, $keybindingOverrides)}
          <div class="row" class:recording={recording === id}>
            <span class="words">
              <span class="name">{$t(COMMAND_LABEL[id])}</span>
              {#if rowError?.id === id}
                <span class="row-error" role="alert"><Icon name="alert-triangle" size={16} />{rowError.text}</span>
              {/if}
            </span>
            <span class="keys">
              {#if recording === id}
                <span class="record">{$t('hotkey_record')}</span>
              {:else if keys}
                {#each keys.split(', ') as chord}<kbd class="keycap">{chord}</kbd>{/each}
              {:else}
                <span class="unbound">{keysOf(id)}</span>
              {/if}
            </span>
            <span class="actions">
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => start(id)}>
                {$t('hotkey_change')}
              </button>
              <button
                type="button"
                class="btn btn-ghost btn-sm"
                disabled={!isOverridden(id, $keybindingOverrides)}
                onclick={() => { resetCommand(id); if (recording === id) recording = null; rowError = null; }}
              >
                {$t('hotkey_reset')}
              </button>
            </span>
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>

<style>
  .hotkeys {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-width: 0;
    container-type: inline-size;
  }
  .hint { margin: 0; font-size: var(--fs-base); }
  .group { display: flex; flex-direction: column; gap: var(--sp-2); }
  .caption { padding: 0 var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }
  .card.list { padding: var(--sp-1); display: flex; flex-direction: column; }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 56px;
    padding: var(--sp-2) var(--sp-4);
  }
  /* A row lit while it records: square between lines, the first and the last row take the card's corners (its radius less its padding). */
  .row:first-child { border-top-left-radius: calc(var(--radius-card) - var(--sp-1)); border-top-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .row:last-child { border-bottom-left-radius: calc(var(--radius-card) - var(--sp-1)); border-bottom-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  /* A straight line between rows: a shadow would follow the rounded corners. */
  .row + .row::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .row.recording { background: var(--accent-bg); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .name { font-size: var(--fs-body); font-weight: var(--fw-medium); color: var(--text); }
  .keys { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--sp-2); }
  .unbound { font-size: var(--fs-sm); color: var(--text-faint); }
  .record {
    display: inline-flex; align-items: center; height: 26px; padding: 0 var(--sp-3);
    border: 1px dashed var(--accent-border); border-radius: var(--radius-xs);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text);
  }
  .actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }
  .row-error { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--danger-text); }
  .row-error :global(svg) { flex-shrink: 0; }

  /* A narrow window: the keys and the buttons go under the command's name. */
  @container (max-width: 640px) {
    .row { flex-wrap: wrap; }
    .words { flex-basis: 100%; }
    .keys { flex: 1; justify-content: flex-start; }
  }
</style>

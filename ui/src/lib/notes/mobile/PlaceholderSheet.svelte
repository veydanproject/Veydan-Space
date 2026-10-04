<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Picker for `{{placeholder}}` typed in the editor; shows the current value unless the note is a template. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError } from '$lib/notes/mobile/api';
  import { onKeyboard } from '$lib/core/mobile/keyboard';
  import { t } from '$lib/core/mobile/i18n';
  import { searchPlaceholders, placeholderMarkup } from '$lib/notes/templates';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    /** Text typed after `{{` */
    query: string;
    /** Notes in the Templates folder keep the markup; others expand the value */
    isTemplate: boolean;
    title: string;
    bindings: string[];
    onclose: () => void;
    /** Text to insert in place of `{{query` */
    onpick: (text: string) => void;
  }

  let { open, query, isTemplate, title, bindings, onclose, onpick }: Props = $props();

  let expanded = $state(false);
  let kb = $state(0);
  let values = $state<Record<string, string>>({});
  let error = $state('');

  const rows = $derived(searchPlaceholders(query));

  $effect(() => {
    if (!open) { expanded = false; return; }
    if (isTemplate) return;
    api.notes
      .placeholderValues(title, bindings)
      .then((v) => (values = v))
      .catch((e) => (error = formatError(e)));
  });

  $effect(() => {
    if (!open || expanded) return;
    return onKeyboard((n) => (kb = n));
  });

  function pick(name: string) {
    const value = isTemplate ? '' : values[name] ?? '';
    onpick(value || placeholderMarkup(name));
  }

  function close() {
    expanded = false;
    onclose();
  }
</script>

{#if open && !expanded}
  <!-- Compact strip above the keyboard: first matches inline, tap the label for the full sheet -->
  <div class="bar" style:bottom="{kb}px" style:padding-bottom="{kb > 0 ? '8px' : 'calc(8px + var(--sab))'}">
    <button type="button" class="label" onclick={() => (expanded = true)}>
      <Icon name="code" size={16} />
    </button>
    <div class="strip">
      {#each rows.slice(0, 6) as name (name)}
        <button type="button" class="pill" onclick={() => pick(name)}>{name}</button>
      {/each}
      {#if rows.length === 0}<span class="muted">{$t('notes_placeholder_none')}</span>{/if}
    </div>
  </div>
{/if}

<BottomSheet open={open && expanded} title={$t('notes_placeholder_title')} onclose={close}>
  {#if error}<div class="m-error">{error}</div>{/if}
  <div class="m-list">
    {#each rows as name (name)}
      <button type="button" class="m-row" onclick={() => pick(name)}>
        <Icon name="code" size={18} />
        <span class="m-row-label mono">{placeholderMarkup(name)}</span>
        {#if !isTemplate}
          <span class="m-row-value" class:muted={!values[name]}>{values[name] || '—'}</span>
        {/if}
      </button>
    {/each}
    {#if rows.length === 0}
      <div class="m-row"><span class="m-row-label muted">{$t('notes_placeholder_none')}</span></div>
    {/if}
  </div>
</BottomSheet>

<style>
  .bar {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 21;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--m-nav);
    border-top: 1px solid var(--border);
  }
  .label {
    display: inline-flex; align-items: center; justify-content: center;
    width: 40px; height: 40px; flex-shrink: 0;
    border: 0; border-radius: 12px; background: var(--m-field); color: var(--text-2);
  }
  .strip { flex: 1; display: flex; gap: 8px; overflow-x: auto; scrollbar-width: none; }
  .strip::-webkit-scrollbar { display: none; }
  .pill {
    flex-shrink: 0; min-height: 40px; padding: 0 14px;
    border: 0; border-radius: 12px; background: var(--m-field); color: var(--text);
    font: inherit; font-size: 14px; font-family: var(--font-mono);
  }
  .muted { color: var(--text-3); }
  .m-row-value { max-width: 40%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>

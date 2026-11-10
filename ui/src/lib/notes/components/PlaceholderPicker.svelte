<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Autocomplete for `{{placeholder}}` while editing a template note. -->
<script lang="ts">
  import Suggest, { type SuggestItem } from '$lib/core/ui/Suggest.svelte';
  import { t } from '$lib/core/i18n';
  import { searchPlaceholders, placeholderMarkup } from '$lib/notes/templates';

  interface Props {
    query: string;
    /** Highlighted row; moved by arrow keys via `handleKeydown` */
    index?: number;
    /** Current value shown next to the name; omitted inside template notes */
    preview?: (name: string) => string;
    /** The editor: the list's floor is its dialog's footer. */
    anchor: HTMLElement | null | undefined;
    /** The line of the caret on the screen: the list goes under it. */
    caret: () => { left: number; top: number; bottom: number } | null;
    onpick: (name: string) => void;
  }

  let { query, index = $bindable(0), preview, anchor, caret, onpick }: Props = $props();

  let list = $state<Suggest | null>(null);

  const rows = $derived(searchPlaceholders(query));
  const items = $derived(rows.map((name): SuggestItem => ({
    key: name, text: placeholderMarkup(name), icon: 'code', mono: true,
    ...(preview ? { hint: preview(name) || '—' } : {}),
  })));

  /** Arrow navigation and Enter/Tab pick; returns true when the key was consumed. */
  export function handleKeydown(e: KeyboardEvent): boolean {
    if (e.key === 'Tab' && rows[index]) { e.preventDefault(); onpick(rows[index]); return true; }
    return list?.keydown(e) ?? false;
  }
</script>

<Suggest
  bind:this={list}
  bind:active={index}
  first={0}
  {items}
  {anchor}
  {caret}
  empty={$t('note_placeholder_none')}
  onpick={(item) => onpick(item.key)}
/>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Single-choice list in a bottom sheet (theme, language, default app); an option may carry a short code (a language's), shown as the computer shows it. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from './BottomSheet.svelte';

  interface Props {
    open: boolean;
    title: string;
    options: { id: string; label: string; icon?: string; code?: string; lang?: string }[];
    value: string;
    onclose: () => void;
    onpick: (id: string) => void;
  }

  let { open, title, options, value, onclose, onpick }: Props = $props();
</script>

<BottomSheet {open} {title} {onclose}>
  <div class="m-list" role="radiogroup" aria-label={title}>
    {#each options as o (o.id)}
      <button type="button" class="m-row" class:on={o.id === value} role="radio" aria-checked={o.id === value} lang={o.lang} onclick={() => { onpick(o.id); onclose(); }}>
        {#if o.icon}<Icon name={o.icon} size={20} />{/if}
        {#if o.code}<span class="code">{o.code}</span>{/if}
        <span class="m-row-label">{o.label}</span>
        {#if o.id === value}<span class="tick"><Icon name="check" size={18} /></span>{/if}
      </button>
    {/each}
  </div>
</BottomSheet>

<style>
  .m-row.on { color: var(--accent-text); font-weight: 600; }
  .tick { color: var(--accent-text); display: inline-flex; }
  /* A language's code as a small chip, as the computer's settings show it. */
  .code {
    flex-shrink: 0; min-width: 38px; padding: 1px 6px; border: 1px solid var(--border-2); border-radius: var(--radius-xs);
    font-family: var(--font-mono); font-size: var(--fs-xs); font-weight: var(--fw-bold); text-align: center;
    text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-2);
  }
  .on .code { border-color: var(--accent-border); color: var(--accent-text); }
</style>

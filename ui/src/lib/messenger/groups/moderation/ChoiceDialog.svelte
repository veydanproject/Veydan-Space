<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A moderator's decision that needs one more word than yes or no: for how
  long a mute, how far back a hiding, whether a ban hides what was written.
  A menu entry cannot hold a choice (it closes on the tap), and the shared
  confirmation is a plain yes or no, so the choice is asked here.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';

  interface Props {
    open: boolean;
    title: string;
    text?: string;
    /** Chips, one of which is `value`; none: no choice of span. */
    options?: { value: string; label: string }[];
    value?: string;
    /** The label of a box to tick; absent: no box. */
    check?: string;
    checked?: boolean;
    cta: string;
    danger?: boolean;
    busy?: boolean;
    error?: string;
    onconfirm: () => void;
  }
  let {
    open = $bindable(), title, text, options = [], value = $bindable(''), check, checked = $bindable(false),
    cta, danger = false, busy = false, error = '', onconfirm,
  }: Props = $props();
</script>

<Dialog bind:open {title} width="min(400px, calc(100vw - 24px))">
  <div class="body">
    {#if text}<p class="text">{text}</p>{/if}
    {#if options.length}
      <div class="chips" role="radiogroup" aria-label={title}>
        {#each options as o (o.value)}
          <button type="button" class="chip" class:active={value === o.value} role="radio" aria-checked={value === o.value}
            disabled={busy} onclick={() => (value = o.value)}>{o.label}</button>
        {/each}
      </div>
    {/if}
    {#if check}
      <label class="check">
        <input type="checkbox" bind:checked disabled={busy} />
        <span>{check}</span>
      </label>
    {/if}
    {#if error}<div class="error-msg">{error}</div>{/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('common_cancel')}</button>
    <button type="button" class="btn {danger ? 'btn-danger' : 'btn-primary'}" disabled={busy || (options.length > 0 && !value)} onclick={onconfirm}>{cta}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .text { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .check { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-body); cursor: pointer; user-select: none; }
  @media (pointer: coarse) { .chip { min-height: 40px; padding-inline: 14px; } }
</style>

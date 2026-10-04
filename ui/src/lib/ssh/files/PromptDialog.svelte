<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Single-line text input dialog for rename / new folder / new file. -->
<script lang="ts">
  import { tick } from 'svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    open: boolean;
    title: string;
    label?: string;
    value?: string;
    confirmLabel?: string;
    /** Pre-select this many leading chars (e.g. filename without extension). */
    selectTo?: number;
    onconfirm: (value: string) => void;
    oncancel: () => void;
  }

  let {
    open = $bindable(),
    title,
    label,
    value = '',
    confirmLabel,
    selectTo,
    onconfirm,
    oncancel,
  }: Props = $props();

  let current = $state('');
  let inputEl = $state<HTMLInputElement | null>(null);

  $effect(() => {
    if (open) {
      current = value;
      tick().then(() => {
        if (!inputEl) return;
        inputEl.focus();
        if (selectTo != null && selectTo > 0) inputEl.setSelectionRange(0, selectTo);
        else inputEl.select();
      });
    }
  });

  function confirm() {
    const v = current.trim();
    if (!v) return;
    onconfirm(v);
  }
</script>

<Dialog {open} {title} width="420px" onclose={oncancel}>
  <div class="form-group">
    {#if label}<label for="prompt-dialog-input">{label}</label>{/if}
    <input
      id="prompt-dialog-input"
      bind:this={inputEl}
      type="text"
      bind:value={current}
      spellcheck="false"
      autocomplete="off"
      onkeydown={(e) => {
        if (e.key === 'Enter') confirm();
        if (e.key === 'Escape') oncancel();
      }}
    />
  </div>
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={oncancel}>{$t('cancel')}</button>
    <button class="btn btn-primary" disabled={!current.trim()} onclick={confirm}>
      {confirmLabel ?? $t('files_op_ok')}
    </button>
  {/snippet}
</Dialog>

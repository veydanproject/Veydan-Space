<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    label: string;
    value: string;
    mono?: boolean;
  }
  let { label, value, mono = false }: Props = $props();

  let copied = $state(false);
  let el = $state<HTMLElement | null>(null);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // Older webviews: fall back to a selection the user can copy by hand.
      if (el) {
        const range = document.createRange();
        range.selectNodeContents(el);
        const sel = window.getSelection();
        sel?.removeAllRanges();
        sel?.addRange(range);
        document.execCommand?.('copy');
      }
    }
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="copy-field">
  <span class="label">{label}</span>
  <div class="row">
    <code bind:this={el} class:mono>{value}</code>
    <button class="icon-btn" title={$t('msg_copy')} onclick={copy}>
      <Icon name={copied ? 'check' : 'copy'} size={13} />
    </button>
  </div>
</div>

<style>
  .copy-field { display: flex; flex-direction: column; gap: 6px; }
  .label { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .row { display: flex; align-items: center; gap: var(--sp-2); }
  code {
    flex: 1; min-width: 0; word-break: break-all;
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body);
    background: var(--surface-2); padding: 8px 10px; border-radius: var(--radius-sm);
    user-select: all;
  }
</style>

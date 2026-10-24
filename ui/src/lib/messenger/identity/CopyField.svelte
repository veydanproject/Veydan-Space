<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
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
      copiedToast();
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
    <button class="icon-btn" title={$t('msg_copy')} aria-label={$t('msg_copy')} onclick={copy}>
      <Icon name={copied ? 'check' : 'copy'} size={13} />
    </button>
  </div>
</div>

<style>
  .copy-field { display: flex; flex-direction: column; gap: 6px; }
  .label { color: var(--text-2); font-size: var(--fs-base); font-weight: var(--fw-semibold); }
  .row { display: flex; align-items: flex-start; gap: var(--sp-2); }
  /* A key is long: it wraps inside its box instead of running out of it. */
  code {
    flex: 1; min-width: 0; overflow-wrap: anywhere;
    font-family: var(--font-mono); font-size: var(--fs-sm); line-height: 1.5; color: var(--text-body);
    background: var(--surface-2); padding: var(--sp-3); border-radius: var(--radius);
    user-select: all;
  }
</style>

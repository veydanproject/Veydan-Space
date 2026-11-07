<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A value shown large — a code of a sign-in, a key of activation — and, when `copy` is given, the button that puts it on the clipboard. Text alone: what it shows was held to its shape before it came here. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { copied } from '$lib/core/ui/toast.svelte';

  /** `large`: a short code read off the screen; a key is long, and copied rather than read. */
  let { text, copy = null, large = false }: { text: string; copy?: string | null; large?: boolean } = $props();

  async function put() {
    if (!copy) return;
    try {
      await navigator.clipboard.writeText(copy);
      copied();
    } catch {
      // No clipboard here: the value stays on the screen.
    }
  }
</script>

<div class="value">
  <span class="text" class:large>{text}</span>
  {#if copy}
    <button class="copy" type="button" onclick={put} aria-label={$t('common_copy')} title={$t('common_copy')}>
      <Icon name="copy" size={18} />
    </button>
  {/if}
</div>

<style>
  .value { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); min-width: 0; }
  .text {
    font-family: var(--font-mono); font-size: var(--fs-md); font-weight: var(--fw-bold);
    color: var(--text); overflow-wrap: anywhere; user-select: all;
  }
  .text.large { font-size: var(--fs-2xl); }
  .copy {
    position: relative; flex-shrink: 0; width: 40px; height: 40px; border: none; padding: 0; border-radius: var(--radius); cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; background: var(--accent-bg); color: var(--accent-text-2);
  }
  .copy::before { content: ''; position: absolute; inset: -2px; }
  .copy:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
</style>

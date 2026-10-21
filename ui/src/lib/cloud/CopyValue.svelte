<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A value shown large — a code of a sign-in, a key of activation — and, when `copy` is given, the button that puts it on the clipboard. Text alone: what it shows was held to its shape before it came here. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  /** `large`: a short code read off the screen; a key is long, and copied rather than read. */
  let { text, copy = null, large = false }: { text: string; copy?: string | null; large?: boolean } = $props();

  let copied = $state(false);

  async function put() {
    if (!copy) return;
    try {
      await navigator.clipboard.writeText(copy);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch {
      // No clipboard here: the value stays on the screen.
    }
  }
</script>

<div class="value">
  <span class="text" class:large>{text}</span>
  {#if copy}
    <button class="btn btn-ghost btn-sm copy" type="button" onclick={put} aria-label={$t('cloud_copy')}>
      <Icon name={copied ? 'check' : 'copy'} size={14} />{copied ? $t('cloud_copied') : $t('cloud_copy')}
    </button>
  {/if}
</div>

<style>
  .value { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; min-width: 0; }
  .text {
    font-family: var(--font-mono); font-size: var(--fs-md); font-weight: var(--fw-bold); letter-spacing: 0.06em;
    color: var(--text); overflow-wrap: anywhere; user-select: all;
  }
  .text.large { font-size: var(--fs-2xl); letter-spacing: 0.12em; }
  .copy { flex-shrink: 0; }
</style>

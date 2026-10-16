<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The buttons under a step: a row at the right on a computer, full-width and stacked with the main one on top on a phone. The main button is written last. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { isMobile } from '$lib/core/platform';

  let { children, start = false }: { children: Snippet; start?: boolean } = $props();
</script>

<div class="actions" class:m={isMobile} class:start>{@render children()}</div>

<style>
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; justify-content: flex-end; align-items: center; }
  .actions.start { justify-content: flex-start; }
  .actions.m { flex-direction: column-reverse; align-items: stretch; flex-wrap: nowrap; }
  .actions.m :global(.btn) { width: 100%; justify-content: center; min-height: 48px; font-size: var(--fs-body); }
</style>

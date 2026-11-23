<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The product's logo in a screen header: opens the sheet of the apps. Absent in a product of one module. -->
<script lang="ts">
  import { hub } from '$lib/core/mobile/hub.svelte';
  import { product } from '$lib/core/product';
</script>

{#if hub.available}
  <button type="button" class="hub" onclick={() => hub.open()} aria-label={product.name}>
    <img src="/logo.png" alt="" />
  </button>
{/if}

<style>
  .hub {
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
    width: 44px; height: 44px; padding: 0; border: none; border-radius: 50%; background: none; cursor: pointer;
  }
  .hub:active { background: var(--surface-3); }
  .hub img { width: 30px; height: 30px; object-fit: contain; }
</style>

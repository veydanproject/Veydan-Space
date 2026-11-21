<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The product's logo in a screen header: opens the sheet of the apps. Absent in a product of one module. -->
<script lang="ts">
  import { hub } from '$lib/core/mobile/hub.svelte';
  import { product } from '$lib/core/product';
</script>

{#if hub.available}
  <button type="button" class="icon-btn hub" onclick={() => hub.open()} aria-label={product.name}>
    <img src="/logo.png" alt="" />
  </button>
{/if}

<style>
  /* A button of the phone's head (.m-header): the common .icon-btn, 40, its zone 44 (mobile.css); the logo 30 in it. */
  .hub img { width: 30px; height: 30px; object-fit: contain; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import DesktopCloud from '$lib/cloud/CloudPage.svelte';
  import MobileCloud from '$lib/cloud/MobileCloudPage.svelte';
</script>

{#if isMobile}
  <MobileCloud />
{:else}
  <DesktopCloud />
{/if}

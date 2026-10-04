<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import DesktopSettings from '$lib/core/desktop/SettingsPage.svelte';
  import MobileSettings from '$lib/core/mobile/SettingsPage.svelte';
</script>

{#if isMobile}
  <MobileSettings />
{:else}
  <DesktopSettings />
{/if}

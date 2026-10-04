<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import DesktopMessenger from '$lib/messenger/pages/MessengerPage.svelte';
  import MobileMessenger from '$lib/messenger/mobile/MobileChats.svelte';
</script>

{#if isMobile}
  <MobileMessenger />
{:else}
  <DesktopMessenger />
{/if}

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import DesktopPass from '$lib/pass/desktop/PassPage.svelte';
  import MobilePasswords from '$lib/pass/routes/MobilePasswordsPage.svelte';
</script>

{#if isMobile}
  <MobilePasswords />
{:else}
  <DesktopPass />
{/if}

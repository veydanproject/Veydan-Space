<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import DesktopNotes from '$lib/notes/desktop/NotesPage.svelte';
  import MobileNotes from '$lib/notes/mobile/NotesPage.svelte';
</script>

{#if isMobile}
  <MobileNotes />
{:else}
  <DesktopNotes />
{/if}

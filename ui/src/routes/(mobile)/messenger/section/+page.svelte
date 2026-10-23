<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { page } from '$app/state';
  import Page from '$lib/messenger/mobile/MobileChats.svelte';
</script>

{#key page.url.searchParams.get('id')}
  <Page kind={page.url.searchParams.get('id') ?? 'dm'} />
{/key}

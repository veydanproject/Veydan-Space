<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { page } from '$app/state';
  import Page from '$lib/pass/routes/MobilePasswordPage.svelte';
</script>

<!-- Beside the list another row replaces this one in place: a page of its own, loaded anew. -->
{#key page.params.id}
  <Page />
{/key}

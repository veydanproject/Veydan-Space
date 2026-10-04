<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The TOTP drawer of a workspace page (an entity view of scope `workspace`). -->
<script lang="ts">
  import TotpGenerator from '$lib/pass/components/TotpGenerator.svelte';

  let { id, open = $bindable(false) }: { id: string; open?: boolean } = $props();
</script>

<TotpGenerator bind:open context="workspace" contextId={id} />

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The passwords drawer of a workspace page (an entity view of scope `workspace`). -->
<script lang="ts">
  import PasswordDrawer from '$lib/pass/components/PasswordDrawer.svelte';

  let { id, open = $bindable(false) }: { id: string; open?: boolean } = $props();
</script>

<PasswordDrawer bind:open context="workspace" contextId={id} />

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The SSH drawer of a workspace page (an entity view of scope `workspace`). -->
<script lang="ts">
  import SSHPanel from '$lib/ssh/components/SSHPanel.svelte';

  let { id, open = $bindable(false) }: { id: string; open?: boolean } = $props();
</script>

<SSHPanel bind:open context="workspace" workspaceId={id} />

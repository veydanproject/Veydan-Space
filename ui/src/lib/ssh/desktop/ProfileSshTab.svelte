<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The SSH tab of a browser profile's panel (an entity view of scope `profile`). -->
<script lang="ts">
  import SshProfileTab from '$lib/ssh/components/SshProfileTab.svelte';

  let { id, workspaceId = '' }: { id: string; workspaceId?: string } = $props();
</script>

<SshProfileTab profileId={id} {workspaceId} />

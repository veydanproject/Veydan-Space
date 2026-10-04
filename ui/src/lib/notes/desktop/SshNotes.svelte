<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The notes about an SSH connection, inside its form (an entity view of scope `ssh`). -->
<script lang="ts">
  import EntityNotes from '$lib/notes/components/EntityNotes.svelte';

  let { id, name = '' }: { id: string; name?: string } = $props();
</script>

<EntityNotes kind="ssh" {id} newTitle={name} />

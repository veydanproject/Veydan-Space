<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The notes about a proxy, inside its form (an entity view of scope `proxy`). -->
<script lang="ts">
  import EntityNotes from '$lib/notes/components/EntityNotes.svelte';

  let { id, name = '' }: { id: string; name?: string } = $props();
</script>

<EntityNotes kind="proxy" {id} newTitle={name} />

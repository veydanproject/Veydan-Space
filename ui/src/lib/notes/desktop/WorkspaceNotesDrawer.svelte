<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The notes drawer of a workspace page (an entity view of scope `workspace`). -->
<script lang="ts">
  import NotesPanel from '$lib/notes/components/NotesPanel.svelte';

  let { id, open = $bindable(false) }: { id: string; open?: boolean } = $props();
</script>

<NotesPanel bind:open context="workspace" contextId={id} />

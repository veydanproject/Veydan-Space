<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The notes' "more" sheet, mounted once by the mobile shell; the bottom bar opens it. -->
<script lang="ts">
  import NotesMoreSheet from '$lib/notes/mobile/NotesMoreSheet.svelte';
  import { notesMobileUi } from '$lib/notes/mobile/ui.svelte';
</script>

<NotesMoreSheet open={notesMobileUi.moreOpen} onclose={() => (notesMobileUi.moreOpen = false)} />

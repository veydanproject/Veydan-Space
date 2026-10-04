<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The passwords tab of a browser profile's panel (an entity view of scope `profile`). -->
<script lang="ts">
  import PasswordPanel from '$lib/pass/components/PasswordPanel.svelte';

  let { id }: { id: string } = $props();
</script>

<PasswordPanel profileId={id} />

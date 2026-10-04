<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The TOTP tab of a browser profile's panel (an entity view of scope `profile`). -->
<script lang="ts">
  import TotpPanel from '$lib/pass/components/TotpPanel.svelte';

  let { id }: { id: string } = $props();
</script>

<TotpPanel profileId={id} />

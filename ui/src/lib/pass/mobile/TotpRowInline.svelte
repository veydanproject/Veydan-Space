<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The live code of a TOTP entry in a row of a note's context sheet. -->
<script lang="ts">
  import { t } from '$lib/core/mobile/i18n';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';

  let { id }: { id: string } = $props();

  const period = $derived(totpStore.list.find((e) => e.id === id)?.period ?? 30);
</script>

<TotpLiveCode entryId={id} {period} copiedLabel={$t('notes_context_copied')} compact />

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The question «Позвонить?» of a call made from the journal (dial.svelte.ts). -->
<script lang="ts">
  import CallConfirm from './CallConfirm.svelte';
  import { dialer } from './dial.svelte';

  const asking = $derived(dialer.asking);
  const peer = $derived(asking ? { name: asking.target.name, picture: asking.target.picture, seed: asking.target.peer } : { name: '', picture: null, seed: '' });
</script>

<CallConfirm
  media={asking?.media ?? null}
  {peer}
  oncall={(m) => { if (asking) void dialer.go(asking.target, m, asking.phone); }}
  onclose={() => (dialer.asking = null)}
/>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The chevron at the end of a row that leads further (16, `--text-3`). -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
</script>

<span class="chev"><Icon name="chevron-right" size={16} /></span>

<style>
  .chev { flex: none; display: inline-flex; color: var(--text-3); }
</style>

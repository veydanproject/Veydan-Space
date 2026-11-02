<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- One tab of the chat's settings as a section of the core's Settings on a computer: the same view, its tab chosen by `settingsUi.tab`. -->
<script lang="ts">
  import SettingsView from './SettingsView.svelte';
  import { settingsUi, type SettingsTab } from './settingsTab.svelte';

  let { tab }: { tab: SettingsTab } = $props();
</script>

{#if settingsUi.tab === tab}<SettingsView hideTabs />{/if}

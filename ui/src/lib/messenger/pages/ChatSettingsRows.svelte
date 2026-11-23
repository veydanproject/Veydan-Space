<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The chat's settings in the settings of a phone: a row per tab, leading to the chat's settings screen on it. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import NavRow from '$lib/core/ui/NavRow.svelte';
  import { SETTINGS_ICONS, SETTINGS_TABS, openSettings } from './settingsTab.svelte';
</script>

{#each SETTINGS_TABS as id (id)}
  <NavRow icon={SETTINGS_ICONS[id]} title={$t(`msg_settings_tab_${id}` as 'msg_settings_tab_profile')} onclick={() => openSettings(id, true)} />
{/each}

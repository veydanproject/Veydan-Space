<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The Tor page (/tor): what runs, the installation and the settings of the daemon. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import TorInstall from '$lib/tor/desktop/TorInstall.svelte';
  import TorInstances from '$lib/tor/desktop/TorInstances.svelte';
  import TorSettingsForm from '$lib/tor/desktop/TorSettingsForm.svelte';
</script>

<div class="page tor-page">
  <a href="/settings#tor" class="back-link">
    <Icon name="chevron-left" size={15} />
    <span>{$t('settings_title')}</span>
  </a>
  <div class="page-header">
    <h1>{$t('settings_tor_title')}</h1>
  </div>

  <div class="card" id="state">
    <div class="card-title">{$t('tor_page_section_state')}</div>
    <TorInstances />
  </div>

  <div class="card" id="install">
    <div class="card-title">{$t('tor_page_section_install')}</div>
    <TorInstall />
  </div>

  <TorSettingsForm />
</div>

<style>
  .tor-page { max-width: 760px; }
  .back-link {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 0.25rem;
    color: var(--text-2); font-size: var(--fs-sm); text-decoration: none;
  }
  .back-link:hover { color: var(--text); }
  .card { display: flex; flex-direction: column; gap: var(--sp-4); padding: var(--sp-6); }
  .card-title {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-dim);
    text-transform: uppercase; letter-spacing: 1px;
  }
</style>

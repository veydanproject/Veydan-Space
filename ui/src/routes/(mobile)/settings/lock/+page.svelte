<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import LockSetupForm from '$lib/core/lock/LockSetupForm.svelte';
</script>

<div class="m-page">
  <div class="m-header">
    <a class="m-ibtn" href="/settings" aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </a>
    <h1 class="m-title">{$t('settings_lock_section')}</h1>
  </div>

  <div class="m-body">
    <p class="m-hint lead">{$t('settings_lock_hint')}</p>
    <LockSetupForm />
  </div>
</div>

<style>
  /* The first paragraph keeps a line's distance from the header. */
  .lead { margin-top: var(--sp-2); }
</style>

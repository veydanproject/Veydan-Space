<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Tiny lock next to the passwords title: red open without a PIN, green closed with one.
     Click opens the lock settings. -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';

  interface Props {
    size?: number;
  }

  let { size = 10 }: Props = $props();

  const protected_ = $derived(appLock.status.enabled);
  const setupHref = isMobile ? '/settings/lock' : '/settings';
</script>

{#if appLock.ready}
  <button
    type="button"
    class="badge"
    class:on={protected_}
    title={protected_ ? $t('pw_lock_on_tip') : $t('pw_lock_off_tip')}
    aria-label={protected_ ? $t('pw_lock_on_tip') : $t('pw_lock_off_tip')}
    onclick={() => goto(setupHref)}
  >
    <Icon name={protected_ ? 'lock' : 'lock-open'} {size} />
  </button>
{/if}

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    margin: 0;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--danger-text);
    cursor: pointer;
    line-height: 0;
    opacity: 0.85;
  }
  .badge.on { color: var(--success-text); }
  .badge:hover { opacity: 1; background: var(--surface-hover); }
</style>

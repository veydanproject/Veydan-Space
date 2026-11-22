<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The plate of a vault that cannot be changed now (design 16), over the phone's
  password list and in a password's card: locked, «Пароли заблокированы.
  Разблокируйте, чтобы изменять.»; the vault of another lock, the vault's own
  error (pw_err_mismatch). `vaultWritable()` is the one condition both screens
  hide «+», «Изменить» and «Удалить» by.
-->
<script lang="ts" module>
  import { appLock } from '$lib/core/lock/store.svelte';

  /** The vault takes changes: not locked, not another lock's. */
  export function vaultWritable(): boolean {
    return !appLock.locked && appLock.status.vault !== 'mismatch';
  }
</script>

<script lang="ts">
  import Notice from '$lib/core/ui/Notice.svelte';
  import { t } from '$lib/core/mobile/i18n';
</script>

<!-- Not before the first status read: no plate flashes over an open vault. -->
{#if appLock.ready && !vaultWritable()}
  <div class="lock-notice">
    <Notice tone="warn" role="status" icon="lock">{appLock.locked ? $t('pw_locked_notice') : $t('pw_err_mismatch')}</Notice>
  </div>
{/if}

<style>
  .lock-notice { margin-bottom: var(--sp-3); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The secret of the lock in a restore (cloud-recovery.md §7), shown only for
  an account that has a lock: «Помню» — its PIN or password, which stays —
  or «Задать новый», which the 12 words allow. The restore by the words and
  the one through the trusted people both.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import LockSecretFields from '$lib/core/lock/LockSecretFields.svelte';
  import { isMobile } from '$lib/core/platform';
  import type { RestoreSecretState } from '$lib/cloud/restoreSecret';

  interface Props {
    state: RestoreSecretState;
    disabled?: boolean;
  }

  let { state = $bindable(), disabled = false }: Props = $props();

  function pick(mode: RestoreSecretState['mode']) {
    if (state.mode !== mode) state = { ...state, mode };
  }
</script>

<div class="secret" class:mobile={isMobile}>
  <div class="block">
    <h2>{$t('cloud_restore_lock_title')}</h2>
    <p class="text">{$t('cloud_restore_lock_text')}</p>
  </div>
  <div class="seg" role="group" aria-label={$t('cloud_restore_lock_title')}>
    <button type="button" class="seg-btn" class:active={state.mode === 'old'} aria-pressed={state.mode === 'old'} {disabled} onclick={() => pick('old')}>
      {$t('cloud_restore_lock_old')}
    </button>
    <button type="button" class="seg-btn" class:active={state.mode === 'new'} aria-pressed={state.mode === 'new'} {disabled} onclick={() => pick('new')}>
      {$t('cloud_restore_lock_new')}
    </button>
  </div>
  {#if state.mode === 'old'}
    <input
      class="lock-field"
      type="password"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      aria-label={$t('cloud_restore_old_label')}
      placeholder={$t('cloud_restore_old_label')}
      bind:value={state.old}
      {disabled}
    />
  {:else}
    <LockSecretFields bind:kind={state.kind} bind:password={state.password} bind:confirm={state.confirm} bind:hint={state.hint} {disabled} />
  {/if}
</div>

<style>
  .secret { display: flex; flex-direction: column; gap: var(--sp-3); }
  .block { display: flex; flex-direction: column; gap: var(--sp-1); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .seg { display: flex; width: 100%; }
  .seg .seg-btn { flex: 1; justify-content: center; }
  /* As the fields of LockSecretFields beside it. */
  .lock-field {
    width: 100%;
    height: var(--control-h-lg);
    padding: 0 var(--sp-3);
    font: inherit;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    background-color: var(--surface-3);
    color: var(--text-body);
  }
  .mobile .seg-btn { min-height: 44px; }
  .mobile .lock-field { height: auto; min-height: 48px; font-size: var(--fs-md); }
</style>

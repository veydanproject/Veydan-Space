<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- PIN/password switch, new secret + confirmation, optional hint. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { lockInputAttrs, lockSecretIssue } from '$lib/core/lock/secret';
  import { isMobile } from '$lib/core/platform';
  import type { LockKind } from '$lib/core/types';

  interface Props {
    kind: LockKind;
    password: string;
    confirm: string;
    hint: string;
    disabled?: boolean;
  }

  let {
    kind = $bindable('pin'),
    password = $bindable(''),
    confirm = $bindable(''),
    hint = $bindable(''),
    disabled = false,
  }: Props = $props();

  const attrs = $derived(lockInputAttrs(kind));
  const issue = $derived(lockSecretIssue(kind, password, confirm));

  function pick(next: LockKind) {
    if (kind === next) return;
    kind = next;
    password = '';
    confirm = '';
  }
</script>

<div class="fields" class:mobile={isMobile}>
  <div class="seg">
    <button type="button" class="seg-btn" class:active={kind === 'pin'} {disabled} onclick={() => pick('pin')}>
      {$t('lock_kind_pin')}
    </button>
    <button type="button" class="seg-btn" class:active={kind === 'password'} {disabled} onclick={() => pick('password')}>
      {$t('lock_kind_password')}
    </button>
  </div>
  <div class="row">
    <input
      class="lock-field"
      class:pin={kind === 'pin'}
      type={kind === 'pin' ? 'text' : 'password'}
      inputmode={attrs.inputmode}
      pattern={attrs.pattern}
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      bind:value={password}
      placeholder={kind === 'pin' ? $t('settings_lock_new_pin') : $t('settings_lock_new_password')}
      {disabled}
    />
    <input
      class="lock-field"
      class:pin={kind === 'pin'}
      type={kind === 'pin' ? 'text' : 'password'}
      inputmode={attrs.inputmode}
      pattern={attrs.pattern}
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      bind:value={confirm}
      placeholder={kind === 'pin' ? $t('settings_lock_confirm_pin') : $t('settings_lock_confirm_password')}
      {disabled}
    />
  </div>
  {#if password && issue === 'short'}
    <span class="note">{$t('settings_lock_min_len')}</span>
  {:else if issue === 'digits'}
    <span class="note err">{$t('settings_lock_pin_digits')}</span>
  {/if}
  <label class="hint">
    <span>{$t('settings_lock_hint_label')}</span>
    <input class="lock-field" type="text" bind:value={hint} placeholder={$t('settings_lock_hint_ph')} autocomplete="off" {disabled} />
    <span>{$t('settings_lock_hint_note')}</span>
  </label>
</div>

<style>
  .fields { display: flex; flex-direction: column; gap: var(--sp-2); }
  .seg { display: flex; width: 100%; }
  .seg .seg-btn { flex: 1; justify-content: center; }
  .row { display: flex; gap: var(--sp-2); }
  .lock-field {
    flex: 1;
    min-width: 0;
    width: 100%;
    /* The size of the fields of the Sync card beside it (core/sync/SyncSettings.svelte) */
    height: var(--control-h-lg);
    /* In a column (a label) flex: 1 would make the basis its height: never below a field */
    min-height: var(--control-h-lg);
    padding: 0 var(--sp-3);
    font: inherit;
    font-size: 0.82rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    /* background-color only: a select keeps the chevron of base.css */
    background-color: var(--surface-3);
    color: var(--text-body);
  }
  .lock-field.pin { -webkit-text-security: disc; }
  .note { font-size: var(--fs-xs); color: var(--text-2); }
  .note.err { color: var(--danger-text); }
  .hint { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-2); }
  .mobile .seg-btn { min-height: 44px; }
  .mobile .row { flex-direction: column; }
  .mobile .lock-field { height: auto; min-height: 48px; font-size: 16px; }
</style>

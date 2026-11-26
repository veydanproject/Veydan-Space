<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- PIN/password switch, new secret + confirmation, optional hint. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
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

  let showNew = $state(false);
  let showConfirm = $state(false);

  const attrs = $derived(lockInputAttrs(kind));
  const issue = $derived(lockSecretIssue(kind, password, confirm));

  function pick(next: LockKind) {
    if (kind === next) return;
    kind = next;
    password = '';
    confirm = '';
    showNew = false;
    showConfirm = false;
  }
</script>

<div class="fields" class:mobile={isMobile}>
  <div class="tab-bar plain" role="tablist">
    <button type="button" class="tab" role="tab" aria-selected={kind === 'pin'} class:active={kind === 'pin'} {disabled} onclick={() => pick('pin')}>
      {$t('lock_kind_pin')}
    </button>
    <button type="button" class="tab" role="tab" aria-selected={kind === 'password'} class:active={kind === 'password'} {disabled} onclick={() => pick('password')}>
      {$t('lock_kind_password')}
    </button>
  </div>
  <div class="row">
    <label class="field">
      <span>{kind === 'pin' ? $t('settings_lock_new_pin') : $t('settings_lock_new_password')}</span>
      <span class="wrap">
        <input
          class="lock-field"
          class:masked={kind === 'pin' && !showNew}
          type={kind === 'pin' || showNew ? 'text' : 'password'}
          inputmode={attrs.inputmode}
          pattern={attrs.pattern}
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          bind:value={password}
          {disabled}
        />
        <button type="button" class="eye" onclick={() => (showNew = !showNew)} aria-label={$t(showNew ? 'pw_btn_hide' : 'pw_btn_reveal')} {disabled}>
          <Icon name={showNew ? 'eye-off' : 'eye'} size={18} />
        </button>
      </span>
    </label>
    <label class="field">
      <span>{kind === 'pin' ? $t('settings_lock_confirm_pin') : $t('settings_lock_confirm_password')}</span>
      <span class="wrap">
        <input
          class="lock-field"
          class:masked={kind === 'pin' && !showConfirm}
          type={kind === 'pin' || showConfirm ? 'text' : 'password'}
          inputmode={attrs.inputmode}
          pattern={attrs.pattern}
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          bind:value={confirm}
          {disabled}
        />
        <button type="button" class="eye" onclick={() => (showConfirm = !showConfirm)} aria-label={$t(showConfirm ? 'pw_btn_hide' : 'pw_btn_reveal')} {disabled}>
          <Icon name={showConfirm ? 'eye-off' : 'eye'} size={18} />
        </button>
      </span>
    </label>
  </div>
  {#if password && issue === 'short'}
    <span class="note">{$t('settings_lock_min_len')}</span>
  {:else if issue === 'digits'}
    <span class="error-msg">{$t('settings_lock_pin_digits')}</span>
  {/if}
  <label class="hint">
    <span>{$t('settings_lock_hint_label')}</span>
    <input class="lock-field" type="text" bind:value={hint} placeholder={$t('settings_lock_hint_ph')} autocomplete="off" {disabled} />
    <span>{$t('settings_lock_hint_note')}</span>
  </label>
</div>

<style>
  .fields { display: flex; flex-direction: column; gap: var(--sp-2); }
  .row { display: flex; gap: var(--sp-2); }
  .field { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-2); }
  .wrap { position: relative; display: flex; }
  .lock-field {
    flex: 1;
    min-width: 0;
    width: 100%;
    height: var(--control-h);
    min-height: var(--control-h);
    padding: 0 var(--sp-3);
    font: inherit;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    /* background-color only: a select keeps the chevron of base.css */
    background-color: var(--surface-3);
    color: var(--text-body);
  }
  .wrap .lock-field { padding-right: calc(var(--control-h) + var(--sp-1)); }
  .lock-field.masked { -webkit-text-security: disc; }
  .eye {
    position: absolute; top: 0; right: 0; width: var(--control-h); height: 100%;
    display: inline-flex; align-items: center; justify-content: center;
    padding: 0; border: 0; background: none; color: var(--text-2); cursor: pointer;
  }
  .eye:hover:not(:disabled) { color: var(--text); }
  .note { font-size: var(--fs-xs); color: var(--text-2); }
  .hint { display: flex; flex-direction: column; gap: 4px; }
  .hint .lock-field { height: var(--control-h); min-height: var(--control-h); }
  .mobile .row { flex-direction: column; }
  .mobile .lock-field { height: var(--touch); min-height: var(--touch); font-size: var(--fs-md); }
  .mobile .wrap .lock-field { padding-right: calc(var(--touch) + var(--sp-1)); }
  .mobile .eye { width: var(--touch); }
  .mobile .field { font-size: var(--fs-sm); }
</style>

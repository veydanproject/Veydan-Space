<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Three steps: recovery key -> new PIN/password -> new recovery key. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { api } from '$lib/core/api';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { lockSecretIssue } from '$lib/core/lock/secret';
  import type { LockKind } from '$lib/core/types';
  import LockSecretFields from './LockSecretFields.svelte';
  import RecoveryKeyCard from './RecoveryKeyCard.svelte';
  import { recoveryStep } from './gate';

  interface Props {
    onback: () => void;
    /** Runs after the new recovery key is acknowledged; the app is unlocked by then. */
    ondone: () => void;
  }

  let { onback, ondone }: Props = $props();

  // A key issued before a lock is shown again after the unlock (gate.ts).
  let step = $state<1 | 2 | 3>(recoveryStep({ locked: appLock.locked, newKey: appLock.newRecoveryKey }));
  let code = $state('');
  let kind = $state<LockKind>('pin');
  let password = $state('');
  let confirm = $state('');
  let hint = $state('');
  const newCode = $derived(appLock.newRecoveryKey ?? '');
  let error = $state('');
  let busy = $state(false);

  const hasRecovery = $derived(appLock.status.has_recovery);
  const secretValid = $derived(lockSecretIssue(kind, password, confirm) === null);

  /** Uppercase groups of 4 while typing; the backend ignores dashes anyway. */
  function formatCode(raw: string): string {
    const clean = raw.toUpperCase().replace(/[^0-9A-Z]/g, '').slice(0, 24);
    return clean.replace(/(.{4})(?=.)/g, '$1-');
  }

  function onCodeInput(e: Event) {
    code = formatCode((e.currentTarget as HTMLInputElement).value);
  }

  async function paste() {
    try {
      code = formatCode(await navigator.clipboard.readText());
    } catch {}
  }

  async function run(task: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await task();
    } catch (e) {
      const key = passwordErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      busy = false;
    }
  }

  const check = () =>
    run(async () => {
      await api.lock.recoveryCheck(code);
      step = 2;
    });

  const recover = () =>
    run(async () => {
      await appLock.recover(code, { password: password.trim(), kind, hint: hint.trim() || null });
      step = 3;
    });
</script>

<div class="flow">
  <div class="head">
    <button class="back" type="button" onclick={onback} aria-label={$t('lock_recover_back')} disabled={step === 3}>
      <Icon name="chevron-left" size={20} />
    </button>
    <div class="titles">
      <h3>{$t('lock_recover_title')}</h3>
      <span class="step">{$t('lock_recover_step', { n: String(step) })}</span>
    </div>
  </div>

  {#if step === 1}
    <p class="lead">{$t('lock_recover_step1')}</p>
    {#if hasRecovery}
      <p class="sub">{$t('lock_recover_step1_hint')}</p>
      <input
        class="field code mono"
        type="text"
        value={code}
        oninput={onCodeInput}
        placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
        autocomplete="off"
        autocapitalize="characters"
        spellcheck="false"
        disabled={busy}
      />
      <div class="row">
        <button class="btn btn-ghost" type="button" onclick={paste} disabled={busy}>{$t('lock_recover_paste')}</button>
        <button class="btn btn-primary grow" type="button" onclick={check} disabled={busy || code.replace(/-/g, '').length !== 24}>
          {$t('lock_recover_continue')}
        </button>
      </div>
    {:else}
      <p class="sub none">{$t('lock_recover_none')}</p>
      <button class="btn btn-ghost wide" type="button" onclick={onback}>{$t('lock_recover_back')}</button>
    {/if}
  {:else if step === 2}
    <p class="lead">{$t('lock_recover_step2')}</p>
    <LockSecretFields bind:kind bind:password bind:confirm bind:hint disabled={busy} />
    <button class="btn btn-primary wide" type="button" onclick={recover} disabled={busy || !secretValid}>
      {$t('lock_recover_continue')}
    </button>
  {:else}
    <RecoveryKeyCard code={newCode} title={$t('lock_recover_step3')} intro={$t('lock_recover_step3_hint')} {ondone} />
  {/if}

  {#if error}<p class="err">{error}</p>{/if}
</div>

<style>
  .flow { display: flex; flex-direction: column; gap: var(--sp-3); width: 100%; }
  .head { display: flex; align-items: center; gap: var(--sp-2); }
  .back {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }
  .back:disabled { opacity: 0.3; cursor: default; }
  .titles { display: flex; flex-direction: column; }
  .titles h3 { margin: 0; font-size: var(--fs-md); font-weight: 600; color: var(--text); }
  .step { font-size: var(--fs-xs); color: var(--text-3); }
  .lead { margin: 0; font-size: var(--fs-base); font-weight: 600; color: var(--text); }
  .sub { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .sub.none { color: var(--warn-text); }
  .field {
    width: 100%;
    min-height: 40px;
    padding: 6px 10px;
    font: inherit;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-2);
    color: var(--text);
  }
  .field.code { letter-spacing: 0.08em; text-align: center; }
  .row { display: flex; gap: var(--sp-2); }
  .grow { flex: 1; justify-content: center; }
  .wide { width: 100%; justify-content: center; }
  .err { margin: 0; font-size: var(--fs-xs); color: var(--danger-text); }
</style>

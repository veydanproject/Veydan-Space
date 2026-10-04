<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Turn the app lock on/off, change the secret, auto-lock timeout, recovery key. -->
<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { registry } from '$lib/core/registry';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { lockInputAttrs, lockSecretIssue } from '$lib/core/lock/secret';
  import { isMobile } from '$lib/core/platform';
  import type { LockKind } from '$lib/core/types';
  import LockSecretFields from './LockSecretFields.svelte';
  import RecoveryKeyCard from './RecoveryKeyCard.svelte';

  const TIMEOUTS = [0, 1, 5, 15, 30, 60];

  let current = $state('');
  let kind = $state<LockKind>('pin');
  let password = $state('');
  let confirm = $state('');
  let hint = $state('');
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** Set right after a key is created; the card replaces the form until acknowledged. */
  let recoveryCode = $state<string | null>(null);
  let recoveryTitle = $state<string | undefined>(undefined);

  const enabled = $derived(appLock.status.enabled);
  const currentAttrs = $derived(lockInputAttrs(appLock.status.kind));
  const secretValid = $derived(lockSecretIssue(kind, password, confirm) === null);
  const canSave = $derived(secretValid && (!enabled || current.length > 0));

  let synced = false;
  $effect(() => {
    if (!appLock.ready || synced) return;
    synced = true;
    syncFromStatus();
  });

  onMount(() => {
    void appLock.refresh();
    void appLock.listen();
  });

  function syncFromStatus() {
    kind = appLock.status.kind;
    hint = appLock.status.hint ?? '';
  }

  async function run(task: () => Promise<void>) {
    busy = true;
    error = null;
    try {
      await task();
      current = '';
      password = '';
      confirm = '';
      syncFromStatus();
    } catch (e) {
      const key = passwordErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      busy = false;
    }
  }

  const save = () =>
    run(async () => {
      const code = await appLock.setSecret(
        { password: password.trim(), kind, hint: hint.trim() || null },
        current || undefined,
      );
      if (code) {
        recoveryTitle = undefined;
        recoveryCode = code;
      }
    });

  const remove = () => run(() => appLock.setSecret(null, current).then(() => {}));

  const regenerate = () =>
    run(async () => {
      recoveryTitle = $t('settings_lock_recovery_new_title');
      recoveryCode = await appLock.regenerateRecovery(current);
    });

  const setTimeoutMin = (e: Event) =>
    run(() => appLock.setTimeout(Number((e.currentTarget as HTMLSelectElement).value)));

  /**
   * Last resort when the synced vault cannot be opened: the module that owns
   * the key's data (pass) deletes it, the lock stays. The messenger's key on
   * this device is under the same key and goes too: the confirmation adds
   * what each module says it loses (`ModuleDef.vaultResetNote`, platform-spec 8).
   */
  const vaultReset = registry.active.find((m) => m.vaultReset)?.vaultReset;
  const resetNotes = registry.active.flatMap((m) => (m.vaultResetNote ? [m.vaultResetNote] : []));
  const resetVault = () =>
    run(async () => {
      if (!vaultReset) return;
      const text = [$t('pw_reset_confirm'), ...resetNotes.map((key) => $t(key as TranslationKey))].join('\n\n');
      if (!(await ask({ title: $t('pw_reset'), message: text, confirmLabel: $t('pw_reset') }))) return;
      await vaultReset(current);
      await appLock.refresh();
    });
</script>

{#if recoveryCode}
  <RecoveryKeyCard code={recoveryCode} title={recoveryTitle} ondone={() => (recoveryCode = null)} />
{:else}
  <div class="lock-settings" class:mobile={isMobile}>
    {#if !enabled}
      <div class="warning">
        <Icon name="alert-triangle" size={16} />
        <span>{$t('settings_lock_warning')}</span>
      </div>
    {:else}
      <input
        class="lock-field"
        class:pin={appLock.status.kind === 'pin'}
        type={appLock.status.kind === 'pin' ? 'text' : 'password'}
        inputmode={currentAttrs.inputmode}
        pattern={currentAttrs.pattern}
        bind:value={current}
        placeholder={$t('settings_lock_current')}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        disabled={busy}
      />
    {/if}

    <LockSecretFields bind:kind bind:password bind:confirm bind:hint disabled={busy} />

    <div class="row actions">
      <button class="btn btn-primary btn-sm" disabled={busy || !canSave} onclick={save}>
        {enabled ? $t('settings_lock_change') : $t('settings_lock_enable')}
      </button>
      {#if enabled}
        <button class="btn btn-ghost btn-sm" disabled={busy || !current} onclick={remove}>{$t('settings_lock_disable')}</button>
      {/if}
    </div>

    {#if enabled}
      <label class="row timeout">
        <span>{$t('settings_lock_timeout')}</span>
        <select class="lock-field" value={String(appLock.status.timeout_min)} onchange={setTimeoutMin} disabled={busy}>
          {#each TIMEOUTS as m (m)}
            <option value={String(m)}>{m === 0 ? $t('settings_lock_timeout_never') : $t('settings_lock_timeout_min', { n: String(m) })}</option>
          {/each}
        </select>
      </label>

      <div class="recovery-row">
        <div class="recovery-text">
          <span class="label">
            <Icon name="key" size={14} />
            {$t('settings_lock_recovery')}:
            <strong class:missing={!appLock.status.has_recovery}>
              {appLock.status.has_recovery ? $t('settings_lock_recovery_has') : $t('settings_lock_recovery_none')}
            </strong>
          </span>
          <span class="sub">{$t('settings_lock_recovery_new_hint')}</span>
        </div>
        <button class="btn btn-ghost btn-sm" disabled={busy || !current} onclick={regenerate}>
          {$t('settings_lock_recovery_new')}
        </button>
      </div>

      {#if appLock.status.vault === 'mismatch' && vaultReset}
        <div class="mismatch">
          <span class="sub">{$t('pw_mismatch')}</span>
          <button class="reset" type="button" disabled={busy || !current} onclick={resetVault}>
            {$t('pw_reset')}
          </button>
        </div>
      {/if}
    {/if}

    {#if error}<div class="error-msg">{error}</div>{/if}
  </div>
{/if}

<style>
  .lock-settings { display: flex; flex-direction: column; gap: var(--sp-3); }
  .row { display: flex; gap: var(--sp-2); align-items: center; flex-wrap: wrap; }
  .lock-field {
    flex: 1;
    min-width: 0;
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
  /* A select, not the flex .field the settings page gives its labels: its text stays centred */
  select.lock-field { display: block; padding-right: 2rem; }
  .warning {
    display: flex;
    gap: var(--sp-2);
    align-items: flex-start;
    padding: var(--sp-3);
    border-radius: var(--radius-md);
    background: var(--warn-bg, var(--surface-2));
    color: var(--warn-text);
    font-size: var(--fs-sm);
    line-height: 1.45;
  }
  .warning :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .timeout span { font-size: var(--fs-sm); color: var(--text-2); white-space: nowrap; }
  .timeout .lock-field { flex: 0 1 180px; }
  .recovery-row {
    display: flex;
    gap: var(--sp-3);
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    padding-top: var(--sp-2);
    border-top: 1px solid var(--border);
  }
  .recovery-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .label { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-sm); color: var(--text); }
  .label strong.missing { color: var(--warn-text); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); }
  .mismatch { display: flex; flex-direction: column; gap: var(--sp-1); align-items: flex-start; }
  .reset {
    border: none;
    background: transparent;
    padding: 0;
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--danger-text);
    text-decoration: underline;
    cursor: pointer;
  }
  .reset:disabled { opacity: 0.4; cursor: default; }
  .lock-field.pin { -webkit-text-security: disc; }
  .mobile .lock-field { height: auto; min-height: 48px; font-size: 16px; }
  .mobile .actions { flex-direction: column; align-items: stretch; }
  .mobile .actions .btn { width: 100%; min-height: 44px; justify-content: center; }
  .mobile .timeout { flex-direction: column; align-items: stretch; }
  .mobile .timeout .lock-field { flex: 1 1 auto; width: 100%; }
  .mobile .recovery-row { flex-direction: column; align-items: stretch; }
  .mobile .recovery-row .btn { width: 100%; min-height: 44px; justify-content: center; }
</style>

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
  <!-- Off: the warning, then one card of the new secret, the enable at its foot. On: a card of the current
       secret and the new one (the disable on the left of its foot, the save on the right), then a card of
       rows: the auto-lock and the recovery key, its kind in words. -->
  <div class="lock-settings" class:mobile={isMobile}>
    {#if !enabled}
      <div class="warning" role="note">
        <Icon name="alert-triangle" size={18} />
        <span>{$t('settings_lock_warning')}</span>
      </div>
    {/if}

    <div class="card form">
      {#if enabled}
        <input
          class="lock-field"
          class:pin={appLock.status.kind === 'pin'}
          type={appLock.status.kind === 'pin' ? 'text' : 'password'}
          inputmode={currentAttrs.inputmode}
          pattern={currentAttrs.pattern}
          bind:value={current}
          placeholder={$t('settings_lock_current')}
          aria-label={$t('settings_lock_current')}
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          disabled={busy}
        />
        <div class="sep"></div>
      {/if}

      <LockSecretFields bind:kind bind:password bind:confirm bind:hint disabled={busy} />

      {#if error}
        <div class="fail" role="alert"><Icon name="alert-triangle" size={18} /><span>{error}</span></div>
      {/if}

      <div class="foot">
        {#if enabled}
          <button type="button" class="btn btn-ghost off" disabled={busy || !current} onclick={remove}>{$t('settings_lock_disable')}</button>
        {/if}
        <button type="button" class="btn btn-primary" disabled={busy || !canSave} onclick={save}>
          {enabled ? $t('settings_lock_change') : $t('settings_lock_enable')}
        </button>
      </div>
    </div>

    {#if enabled}
      <div class="card rows">
        <label class="row-item">
          <span class="words"><span class="title">{$t('settings_lock_timeout')}</span></span>
          <select class="lock-field timeout" value={String(appLock.status.timeout_min)} onchange={setTimeoutMin} disabled={busy}>
            {#each TIMEOUTS as m (m)}
              <option value={String(m)}>{m === 0 ? $t('settings_lock_timeout_never') : $t('settings_lock_timeout_min', { n: String(m) })}</option>
            {/each}
          </select>
        </label>

        <div class="row-item recovery">
          <span class="words">
            <span class="title">{$t('settings_lock_recovery')}</span>
            <strong class="kind" class:missing={!appLock.status.has_recovery}>
              {#if appLock.status.recovery_version === 2}
                {$t('settings_lock_recovery_words')}
              {:else if appLock.status.recovery_version === 1}
                {$t('settings_lock_recovery_code')}
              {:else}
                {$t('settings_lock_recovery_none')}
              {/if}
            </strong>
            <!-- The Recovery Key of 12 words is reissued only by the cloud (platform-spec 8.1): the escrow and the shares follow it. -->
            <span class="sub">
              {#if appLock.status.recovery_version === 2}
                {$t('settings_lock_recovery_v2_hint')}
              {:else if appLock.status.recovery_version === 1}
                {$t('settings_lock_recovery_v1_hint')}
              {:else}
                {$t('settings_lock_recovery_new_hint')}
              {/if}
            </span>
          </span>
          {#if appLock.status.recovery_version !== 2}
            <button type="button" class="btn btn-ghost" disabled={busy || !current} onclick={regenerate}>
              {$t('settings_lock_recovery_new')}
            </button>
          {/if}
        </div>
      </div>

      {#if appLock.status.vault === 'mismatch' && vaultReset}
        <div class="fail mismatch" role="alert">
          <Icon name="alert-triangle" size={18} />
          <span class="mismatch-words">
            <span>{$t('pw_mismatch')}</span>
            <button class="reset" type="button" disabled={busy || !current} onclick={resetVault}>
              {$t('pw_reset')}
            </button>
          </span>
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .lock-settings { display: flex; flex-direction: column; gap: var(--sp-4); container-type: inline-size; }
  .card.form { padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  .sep { height: 1px; background: var(--divider); margin: var(--sp-1) 0; }
  .lock-field {
    flex: 1;
    min-width: 0;
    /* The size of the fields of the Sync card beside it (core/sync/SyncSettings.svelte) */
    height: var(--control-h-lg);
    /* In a column (a label) flex: 1 would make the basis its height: never below a field */
    min-height: var(--control-h-lg);
    padding: 0 var(--sp-3);
    font: inherit;
    font-size: var(--fs-sm);
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
    gap: var(--sp-3);
    align-items: flex-start;
    padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md);
    background: var(--warn-bg);
    color: var(--warn-text);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .warning :global(svg) { flex-shrink: 0; margin-top: 1px; }
  .fail {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); background: var(--danger-bg); color: var(--text); font-size: var(--fs-base); line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .fail :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--danger-text); }
  .foot { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--divider); }
  .off { margin-right: auto; color: var(--danger-text); }
  .card.rows { padding: var(--sp-1); display: flex; flex-direction: column; }
  .row-item { position: relative; display: flex; align-items: center; gap: var(--sp-3); min-height: 60px; padding: var(--sp-2) var(--sp-4); }
  /* A straight line between rows: a shadow would follow the rounded corners. */
  .row-item + .row-item::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: var(--fs-body); font-weight: var(--fw-semibold); color: var(--text); }
  .timeout { flex: 0 0 180px; }
  .kind { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--accent-text); }
  .kind.missing { color: var(--warn-text); }
  .sub { font-size: var(--fs-sm); line-height: 1.4; color: var(--text-2); }
  .mismatch-words { display: flex; flex-direction: column; gap: var(--sp-1); align-items: flex-start; }
  .reset {
    border: none;
    background: transparent;
    padding: 0;
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    color: var(--danger-text);
    text-decoration: underline;
    cursor: pointer;
  }
  .reset:disabled { opacity: 0.4; cursor: default; }
  .lock-field.pin { -webkit-text-security: disc; }
  .mobile .lock-field { height: auto; min-height: var(--sp-12); font-size: var(--fs-md); }
  .mobile .foot { flex-direction: column-reverse; align-items: stretch; }
  .mobile .foot .btn { width: 100%; min-height: var(--control-h-lg); justify-content: center; }
  .mobile .off { margin-right: 0; }
  .mobile .timeout { flex: 0 0 140px; }
  .mobile .recovery { flex-direction: column; align-items: stretch; }
  .mobile .recovery .btn { width: 100%; min-height: var(--control-h-lg); justify-content: center; }
</style>

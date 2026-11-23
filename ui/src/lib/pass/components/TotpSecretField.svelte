<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The key of a TOTP entry in its edit form (Задачи 5 · 05): masked, shown in groups of four until it
  hides itself, copied; a lock in place of both while the app is locked. «Сменить ключ» opens the
  block of a new key: the warning, «Ввести заново» / «Сканировать QR», «Оставить прежний». The form
  saves what `change` holds; the shown key lives here only while it is on screen.
-->
<script module lang="ts">
  export interface KeyChange {
    tab: 'manual' | 'qr';
    secret: string;
    uri: string;
  }
</script>

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { secretCopied } from '$lib/core/ui/toast.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { api } from '$lib/pass/api';
  import { AutoHide } from '$lib/pass/autohide.svelte';
  import { copySecret } from '$lib/pass/clipboard';
  import { readQr } from '$lib/pass/qr';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';

  interface Props {
    entryId: string;
    /** The new key being entered; null keeps the old one. */
    change?: KeyChange | null;
    /** The error of the new key (empty, not a key), under its field. */
    error?: string;
  }

  let { entryId, change = $bindable(null), error = '' }: Props = $props();

  let revealed = $state('');
  const hide = new AutoHide(() => (revealed = ''));
  let failure = $state('');
  let unlockOpen = $state(false);
  let afterUnlock: (() => void) | null = null;

  let qrLoading = $state(false);
  let qrError = $state('');
  let fileInput: HTMLInputElement | null = $state(null);

  const unlocked = $derived(!appLock.locked);
  /** Four characters to a group, as a service prints a key. */
  const grouped = $derived(revealed.replace(/(.{4})/g, '$1 ').trim());

  /** Counts the asks for the key: one answered after the key was hidden, the entry changed or the
   *  form closed is dropped, not shown. */
  let asked = 0;

  function forget() {
    asked++;
    revealed = '';
    hide.stop();
  }

  // On another entry and when the form closes.
  $effect(() => {
    void entryId;
    return forget;
  });

  $effect(() => {
    if (!unlocked) forget();
  });

  function locked(e: unknown): boolean {
    const code = e && typeof e === 'object' && 'code' in e ? String((e as { code: string }).code) : '';
    return code === 'vault_locked';
  }

  function askUnlock(then: () => void) {
    afterUnlock = then;
    unlockOpen = true;
  }

  function onUnlocked() {
    const next = afterUnlock;
    afterUnlock = null;
    next?.();
  }

  async function reveal() {
    failure = '';
    if (revealed) {
      forget();
      return;
    }
    const ask = ++asked;
    try {
      const { value } = await api.totp.reveal(entryId);
      if (ask !== asked || appLock.locked) return;
      revealed = value;
      hide.start();
    } catch (e) {
      if (ask !== asked) return;
      if (locked(e)) askUnlock(() => void reveal());
      else failure = formatError(e);
    }
  }

  async function copy() {
    if (appLock.locked) {
      askUnlock(() => void copy());
      return;
    }
    failure = '';
    try {
      await api.totp.copy(entryId);
      secretCopied($t('toast_copied'));
    } catch (e) {
      if (locked(e)) {
        askUnlock(() => void copy());
        return;
      }
      // No clipboard of the backend (the phone): the WebView's.
      try {
        await copySecret((await api.totp.reveal(entryId)).value, { toast: $t('toast_copied') });
      } catch (err) {
        failure = formatError(err);
      }
    }
  }

  function startChange() {
    if (appLock.locked) {
      askUnlock(startChange);
      return;
    }
    change = { tab: 'manual', secret: '', uri: '' };
  }

  function keep() {
    change = null;
    qrError = '';
  }

  function pickTab(tab: KeyChange['tab']) {
    if (!change) return;
    change = { ...change, tab };
    qrError = '';
  }

  async function handleQrFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file || !change) return;
    qrError = '';
    change = { ...change, uri: '' };
    qrLoading = true;
    try {
      const data = await readQr(file);
      if (!data) {
        qrError = $t('totp_qr_not_found');
        return;
      }
      // Read by the backend before it is taken: a QR of something else says so here, not on «Сохранить»,
      // in words of its own (the link is not repeated under the field).
      try {
        await api.totp.previewUri(data);
      } catch {
        qrError = $t('totp_error_invalid');
        return;
      }
      if (change) change = { ...change, uri: data };
    } catch (err) {
      qrError = formatError(err);
    } finally {
      qrLoading = false;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<div class="form-group">
  <span class="label" id="totp-key-{entryId}">{$t('totp_field_secret')}</span>
  <!-- The row of a password in its card: the value, then «Показать / Скрыть» and «Копировать», or the lock. -->
  <div class="key" role="group" aria-labelledby="totp-key-{entryId}">
    <span class="value mono">{unlocked && revealed ? grouped : '••••••••'}</span>
    {#if unlocked}
      <button type="button" class="icon-btn" onclick={reveal} aria-label={$t(revealed ? 'pw_btn_hide' : 'pw_btn_reveal')} title={$t(revealed ? 'pw_btn_hide' : 'pw_btn_reveal')}>
        <Icon name={revealed ? 'eye-off' : 'eye'} size={14} />
      </button>
      <button type="button" class="icon-btn" onclick={copy} aria-label={$t('common_copy')} title={$t('common_copy')}>
        <Icon name="copy" size={14} />
      </button>
    {:else}
      <button type="button" class="icon-btn" onclick={() => askUnlock(() => void reveal())} aria-label={$t('lock_title')} title={$t('lock_title')}>
        <Icon name="lock-square-rounded" size={14} />
      </button>
    {/if}
  </div>
  {#if unlocked && revealed && hide.left}<span class="autohide">{$t('pw_autohide_in', { n: String(hide.left) })}</span>{/if}
  {#if failure}<div class="error-msg">{failure}</div>{/if}

  {#if !change}
    <div class="change">
      <button type="button" class="btn btn-ghost btn-sm" onclick={startChange}>
{$t('totp_key_change')}
      </button>
    </div>
  {/if}
</div>

{#if change}
  <div class="new-key">
    <Notice tone="warn">{$t('totp_key_change_warning')}</Notice>
    <div class="tab-bar">
      <button type="button" class="tab" class:active={change.tab === 'manual'} onclick={() => pickTab('manual')}>
        <Icon name="keyboard" size={13} /> {$t('totp_key_tab_manual')}
      </button>
      <button type="button" class="tab" class:active={change.tab === 'qr'} onclick={() => pickTab('qr')}>
        <Icon name="scan" size={13} /> {$t('totp_key_tab_qr')}
      </button>
    </div>
    {#if change.tab === 'manual'}
      <div class="form-group">
        <label for="totp-new-key-{entryId}">{$t('totp_key_new')}</label>
        <input
          id="totp-new-key-{entryId}"
          class="mono"
          type="text"
          bind:value={change.secret}
          placeholder={$t('totp_field_secret_placeholder')}
          autocomplete="off"
          spellcheck="false"
          aria-invalid={!!error}
        />
        {#if error}<div class="error-msg">{error}</div>{/if}
      </div>
    {:else}
      <div class="qr-section">
        <p class="hint">{$t('totp_qr_hint')}</p>
        <button type="button" class="btn btn-ghost" onclick={() => fileInput?.click()} disabled={qrLoading}>
          <Icon name="image" size={14} /> {$t('totp_qr_select')}
        </button>
        <input bind:this={fileInput} type="file" accept="image/*" style="display:none" onchange={handleQrFile} />
        {#if qrLoading}<span class="hint">{$t('loading')}</span>{/if}
        {#if qrError}<div class="error-msg">{qrError}</div>{/if}
        {#if change.uri}
          <div class="qr-detected">
            <span class="ok"><Icon name="check" size={14} /></span>
            {$t('totp_key_qr_found')}
          </div>
        {/if}
        {#if error && !change.uri && !qrError}<div class="error-msg">{error}</div>{/if}
      </div>
    {/if}
    <div>
      <button type="button" class="btn btn-ghost btn-sm" onclick={keep}>{$t('totp_key_keep')}</button>
    </div>
  </div>
{/if}

<VaultUnlock bind:open={unlockOpen} onunlocked={onUnlocked} />

<style>
  /* The caption of a field (.form-group label), over a value that is no field. */
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }
  /* The value and its two buttons in one row; a long key wraps inside it. */
  .key { display: flex; align-items: center; gap: var(--sp-2); min-height: 32px; }
  .value { flex: 1; min-width: 0; overflow-wrap: anywhere; font-size: var(--fs-base); color: var(--text); }
  /* «Скроется через N с», as under a shown password. */
  .autohide { font-size: var(--fs-xs); color: var(--text-2); }
  .change { display: flex; }
  .new-key { display: flex; flex-direction: column; gap: var(--sp-3); }
  .tab { justify-content: center; }
  .qr-section {
    display: flex; flex-direction: column; gap: var(--sp-2);
    padding: var(--sp-3); background: var(--surface-3);
    border-radius: var(--radius-md); border: 1px solid var(--border);
  }
  .qr-detected { display: flex; align-items: center; gap: 0.4rem; font-size: var(--fs-sm); color: var(--success-text); }
  .ok { display: flex; color: var(--success); }
  .hint { margin: 0; }
  input.mono::placeholder { font-family: var(--font-ui); }
</style>

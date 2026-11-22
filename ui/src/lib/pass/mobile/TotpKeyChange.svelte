<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The new key of a 2FA entry in the phone's form, opened by «Сменить ключ» of
  its «⋮» (design 16): the computer's block of TotpSecretField in the phone's
  controls — the warning, «Ввести заново» / «Сканировать QR» (a QR from an
  image, readQr), «Оставить прежний». The form saves what `change` holds.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { formatError } from '$lib/core/utils';
  import { api } from '$lib/pass/api';
  import { readQr } from '$lib/pass/qr';
  import type { KeyChange } from '$lib/pass/components/TotpSecretField.svelte';
  import { onDestroy } from 'svelte';

  interface Props {
    change: KeyChange;
    /** The error of the new key (empty, not a key), under its field. */
    error?: string;
    /** «Оставить прежний». */
    onkeep: () => void;
  }

  let { change = $bindable(), error = '', onkeep }: Props = $props();

  let qrLoading = $state(false);
  let qrError = $state('');
  let fileInput = $state<HTMLInputElement | null>(null);
  /** The current reading of a QR: a later pick, «Оставить прежний» or leaving the block drops an older one. */
  let qrRun = 0;

  onDestroy(() => {
    qrRun++;
  });

  function keep() {
    qrRun++;
    qrLoading = false;
    onkeep();
  }

  function pickTab(tab: KeyChange['tab']) {
    change = { ...change, tab };
    qrError = '';
  }

  async function handleQrFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    const run = ++qrRun;
    qrError = '';
    change = { ...change, uri: '' };
    qrLoading = true;
    try {
      const data = await readQr(file);
      if (run !== qrRun) return;
      if (!data) {
        qrError = $t('totp_qr_not_found');
        return;
      }
      // Read by the backend before it is taken: a QR of something else says so here, not on «Сохранить».
      try {
        await api.totp.previewUri(data);
      } catch {
        if (run === qrRun) qrError = $t('totp_error_invalid');
        return;
      }
      if (run !== qrRun) return;
      change = { ...change, uri: data };
    } catch (err) {
      if (run === qrRun) qrError = formatError(err);
    } finally {
      if (run === qrRun) qrLoading = false;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<div class="new-key">
  <Notice tone="warn">{$t('totp_key_change_warning')}</Notice>
  <div class="seg tabs" role="tablist">
    <button type="button" class="seg-btn" role="tab" aria-selected={change.tab === 'manual'} class:active={change.tab === 'manual'} onclick={() => pickTab('manual')}>
      <Icon name="keyboard" size={14} />{$t('totp_key_tab_manual')}
    </button>
    <button type="button" class="seg-btn" role="tab" aria-selected={change.tab === 'qr'} class:active={change.tab === 'qr'} onclick={() => pickTab('qr')}>
      <Icon name="image" size={14} />{$t('totp_key_tab_qr')}
    </button>
  </div>
  {#if change.tab === 'manual'}
    <div class="m-field">
      <label for="totp-new-key">{$t('totp_key_new')}</label>
      <input
        id="totp-new-key"
        class="mono"
        bind:value={change.secret}
        placeholder={$t('totp_field_secret_placeholder')}
        autocomplete="off"
        autocapitalize="characters"
        spellcheck="false"
        aria-invalid={!!error}
      />
      {#if error}<div class="m-error">{error}</div>{/if}
    </div>
  {:else}
    <div class="qr">
      <p class="hint">{$t('totp_qr_hint')}</p>
      <button type="button" class="btn btn-ghost" onclick={() => fileInput?.click()} disabled={qrLoading}>
        <Icon name={qrLoading ? 'loader' : 'image'} size={18} class={qrLoading ? 'spin' : ''} />{$t('totp_qr_select')}
      </button>
      <input bind:this={fileInput} type="file" accept="image/*" hidden onchange={handleQrFile} />
      {#if qrError}<div class="m-error">{qrError}</div>{/if}
      {#if change.uri}
        <div class="found"><Icon name="check" size={16} />{$t('totp_key_qr_found')}</div>
      {/if}
      {#if error && !change.uri && !qrError}<div class="m-error">{error}</div>{/if}
    </div>
  {/if}
  <div>
    <button type="button" class="btn btn-ghost btn-sm" onclick={keep}>{$t('totp_key_keep')}</button>
  </div>
</div>

<style>
  .new-key { display: flex; flex-direction: column; gap: var(--sp-3); margin-bottom: var(--sp-3); }
  .tabs { display: flex; width: 100%; }
  .tabs .seg-btn { flex: 1; justify-content: center; }
  .new-key .m-field { margin-bottom: 0; }
  .qr { display: flex; flex-direction: column; gap: var(--sp-2); }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
  .found { display: flex; align-items: center; gap: var(--sp-1-5); font-size: var(--fs-sm); color: var(--success-text); }
  input.mono::placeholder { font-family: var(--font-ui); }
</style>

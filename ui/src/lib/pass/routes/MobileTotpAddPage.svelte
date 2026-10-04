<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import type { TotpPreview } from '$lib/pass/types';
  import { api } from '$lib/pass/api';
  import { formatError } from '$lib/core/utils';
  import { directory } from '$lib/core/directory';
  import { t } from '$lib/core/mobile/i18n';
  import LabelField from '$lib/pass/mobile/LabelField.svelte';
  import TotpNoteLinks from '$lib/pass/components/TotpNoteLinks.svelte';
  import { syncTotpNotes } from '$lib/pass/totp-notes';
  import { onMount } from 'svelte';

  type Mode = 'manual' | 'uri';
  let mode = $state<Mode>('manual');

  let name = $state('');
  let issuer = $state('');
  let secret = $state('');
  let algorithm = $state('SHA1');
  let digits = $state(6);
  let period = $state(30);
  let advanced = $state(false);

  // Scanner hands the decoded otpauth link over via ?uri=
  const scanned = page.url.searchParams.get('uri') ?? '';
  let uri = $state(scanned);
  if (scanned) mode = 'uri';
  let preview = $state<TotpPreview | null>(null);

  let tags = $state<string[]>([]);
  let notes = $state<{ id: string; title: string }[]>([]);
  let noteIds = $state<string[]>([]);
  let error = $state('');
  let saving = $state(false);

  onMount(() => {
    // Notes to link come through the catalog; a product without them offers none.
    directory.get('note')?.ensureLoaded().then(() => {
      notes = directory.list('note').map((n) => ({ id: n.id, title: n.name }));
    }).catch(() => {});
  });

  // Live preview of a pasted otpauth link; parse errors stay silent until Save.
  $effect(() => {
    const value = uri.trim();
    if (mode !== 'uri' || !value.startsWith('otpauth://')) {
      preview = null;
      return;
    }
    api.totp.previewUri(value).then((p) => (preview = p)).catch(() => (preview = null));
  });

  async function save() {
    error = '';
    if (mode === 'manual') {
      if (!name.trim()) return (error = $t('totp_error_name'));
      if (!secret.trim()) return (error = $t('totp_error_secret'));
    } else if (!uri.trim()) {
      return (error = $t('totp_error_invalid'));
    }

    saving = true;
    try {
      const created = await api.totp.add(
        mode === 'manual'
          ? { name: name.trim(), issuer: issuer.trim() || null, secret, algorithm, digits, period, tags }
          : { name: name.trim(), issuer: issuer.trim() || null, uri: uri.trim(), tags },
      );
      await syncTotpNotes(created.id, noteIds, []);
      goto('/totp', { replaceState: true });
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="m-page">
  <div class="m-header">
    <a class="m-ibtn" href="/totp" aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </a>
    <h1 class="m-title">{$t('totp_add_title')}</h1>
  </div>

  <div class="m-body">
  <div class="seg mode">
    <button class="seg-btn" class:active={mode === 'manual'} onclick={() => (mode = 'manual')}>
      <Icon name="pencil" size={14} />
      {$t('totp_tab_manual')}
    </button>
    <button class="seg-btn" class:active={mode === 'uri'} onclick={() => (mode = 'uri')}>
      <Icon name="link" size={14} />
      {$t('totp_tab_uri')}
    </button>
    <a class="seg-btn" href="/totp/scan">
      <Icon name="scan" size={14} />
      {$t('totp_tab_scan')}
    </a>
  </div>

  {#if mode === 'uri'}
    <div class="m-field">
      <label for="uri">{$t('totp_field_uri')}</label>
      <textarea id="uri" class="mono" rows="3" bind:value={uri} placeholder={$t('totp_field_uri_placeholder')}></textarea>
    </div>
    {#if preview}
      <div class="preview">
        <Icon name="check-circle" size={16} />
        <span>{preview.issuer ? `${preview.issuer} · ` : ''}{preview.name}</span>
        <span class="mono">{preview.secret_masked} · {preview.algorithm}/{preview.digits}/{preview.period}s</span>
      </div>
    {/if}
  {/if}

  <div class="m-field">
    <label for="name">{$t('totp_field_name')}</label>
    <input id="name" bind:value={name} placeholder={$t('totp_field_name_placeholder')} autocomplete="off" />
  </div>
  <div class="m-field">
    <label for="issuer">{$t('totp_field_issuer')}</label>
    <input id="issuer" bind:value={issuer} placeholder={$t('totp_field_issuer_placeholder')} autocomplete="off" />
  </div>

  {#if mode === 'manual'}
    <div class="m-field">
      <label for="secret">{$t('totp_field_secret')}</label>
      <input
        id="secret"
        class="mono"
        bind:value={secret}
        placeholder={$t('totp_field_secret_placeholder')}
        autocomplete="off"
        autocapitalize="characters"
        spellcheck="false"
      />
    </div>

    <button class="advanced-toggle" onclick={() => (advanced = !advanced)}>
      <Icon name={advanced ? 'chevron-up' : 'chevron-down'} size={16} />
      {$t('totp_advanced')}
    </button>
    {#if advanced}
      <div class="advanced">
        <div class="m-field">
          <label for="alg">{$t('totp_field_algorithm')}</label>
          <select id="alg" bind:value={algorithm}>
            <option>SHA1</option>
            <option>SHA256</option>
            <option>SHA512</option>
          </select>
        </div>
        <div class="m-field">
          <label for="digits">{$t('totp_field_digits')}</label>
          <select id="digits" bind:value={digits}>
            <option value={6}>6</option>
            <option value={8}>8</option>
          </select>
        </div>
        <div class="m-field">
          <label for="period">{$t('totp_field_period')}</label>
          <input id="period" type="number" min="10" max="120" bind:value={period} />
        </div>
      </div>
    {/if}
  {/if}

  <!-- The optional labels after the required secret and its parameters, as in the desktop dialog -->
  <LabelField {tags} onchange={(next) => (tags = next)} />
  {#if directory.owned('note')}
    <TotpNoteLinks mobile {notes} selected={noteIds} onchange={(ids) => (noteIds = ids)} />
  {/if}

  {#if error}
    <div class="m-error">{error}</div>
  {/if}

  <button type="button" class="m-btn-grad save" onclick={save} disabled={saving}>
    <Icon name="check" size={18} />
    {$t('totp_btn_save')}
  </button>
  </div>
</div>

<style>
  .mode {
    display: flex;
    width: 100%;
    margin-bottom: var(--sp-4);
  }
  .mode .seg-btn { flex: 1; justify-content: center; text-decoration: none; }
  textarea {
    width: 100%;
    resize: none;
    font-size: 14px;
  }
  .preview {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    margin-bottom: var(--sp-3);
    border-radius: var(--radius-md, 10px);
    background: var(--success-bg);
    color: var(--success-text);
    font-size: var(--fs-sm);
  }
  .preview .mono { flex-basis: 100%; color: var(--text-2); font-size: var(--fs-xs); }
  .advanced-toggle {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    min-height: var(--touch);
    background: none;
    border: 0;
    padding: 0;
    color: var(--text-2);
    font: inherit;
    font-size: var(--fs-sm);
  }
  .advanced {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: var(--sp-3);
  }
  .save { margin-top: var(--sp-4); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { leave } from '$lib/core/ui/back';
  import type { TotpPreview } from '$lib/pass/types';
  import { api } from '$lib/pass/api';
  import { formatError } from '$lib/core/utils';
  import { directory } from '$lib/core/directory';
  import { t } from '$lib/core/mobile/i18n';
  import LabelField from '$lib/pass/mobile/LabelField.svelte';
  import TotpNoteLinks from '$lib/pass/components/TotpNoteLinks.svelte';
  import { syncTotpNotes } from '$lib/pass/totp-notes';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import { leaveGuard } from '$lib/pass/mobile/leaveGuard';
  import { readQr } from '$lib/pass/qr';
  import { takeScanned } from '$lib/pass/store/scanned';
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

  // The scanner hands the otpauth link over in memory, not in the address (its secret stays out of the history).
  const scanned = takeScanned();
  let uri = $state(scanned);
  if (scanned) mode = 'uri';
  /** «⋮» of the head: a QR from an image (design 16). */
  let menu = $state(false);
  let qrLoading = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);
  let preview = $state<TotpPreview | null>(null);

  let tags = $state<string[]>([]);
  let notes = $state<{ id: string; title: string }[]>([]);
  let noteIds = $state<string[]>([]);
  let error = $state('');
  let saving = $state(false);
  // The field the error is about: its line goes under it; any other above the button.
  const errField = $derived(
    !error ? '' : error === $t('totp_error_name') ? 'name' : error === $t('totp_error_secret') ? 'secret' : error === $t('totp_error_invalid') && mode === 'uri' ? 'uri' : '',
  );

  const guard = leaveGuard(
    () =>
      !!(name.trim() || issuer.trim() || secret.trim() || uri.trim()) ||
      algorithm !== 'SHA1' ||
      digits !== 6 ||
      period !== 30 ||
      tags.length > 0 ||
      noteIds.length > 0,
  );

  /** The arrow and «Отмена»: back to the list once the question allows it. */
  async function cancel() {
    if (await guard.mayLeave()) void leave('/totp');
  }

  /** «Выбрать изображение»: the QR of a screenshot becomes the link the form adds, as the computer's QR tab. */
  async function pickedImage(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    menu = false;
    error = '';
    qrLoading = true;
    try {
      const data = await readQr(file);
      if (!data) {
        error = $t('totp_qr_not_found');
        return;
      }
      if (!data.startsWith('otpauth://')) {
        error = $t('totp_scan_not_otpauth');
        return;
      }
      uri = data;
      mode = 'uri';
    } catch (err) {
      error = formatError(err);
    } finally {
      qrLoading = false;
      if (fileInput) fileInput.value = '';
    }
  }

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
      guard.pass();
      void leave('/totp');
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="m-page">
  <div class="m-header">
    <button type="button" class="icon-btn" onclick={cancel} aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </button>
    <h1 class="m-title">{$t('totp_add_title')}</h1>
    <button type="button" class="icon-btn" onclick={() => (menu = true)} disabled={qrLoading} aria-label={$t('nav_more')}>
      <Icon name={qrLoading ? 'loader' : 'more-vertical'} size={22} class={qrLoading ? 'spin' : ''} />
    </button>
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
    <!-- The scanner takes this page's place and gives it back with the link: one step of the history. -->
    <a class="seg-btn" href="/totp/scan" data-sveltekit-replacestate>
      <Icon name="scan" size={14} />
      {$t('totp_tab_scan')}
    </a>
  </div>

  {#if mode === 'uri'}
    <div class="m-field">
      <label for="uri">{$t('totp_field_uri')}</label>
      <textarea id="uri" class="mono" rows="3" bind:value={uri} placeholder={$t('totp_field_uri_placeholder')} aria-invalid={errField === 'uri'}></textarea>
      {#if errField === 'uri'}<div class="m-error">{error}</div>{/if}
    </div>
    {#if preview}
      <div class="preview">
        <Icon name="check" size={16} />
        <span>{preview.issuer ? `${preview.issuer} · ` : ''}{preview.name}</span>
        <span class="mono">{preview.secret_masked} · {preview.algorithm}/{preview.digits}/{preview.period}s</span>
      </div>
    {/if}
  {/if}

  <div class="m-field">
    <label for="name">{$t('totp_field_name')}</label>
    <input id="name" bind:value={name} placeholder={$t('totp_field_name_placeholder')} autocomplete="off" aria-invalid={errField === 'name'} />
    {#if errField === 'name'}<div class="m-error">{error}</div>{/if}
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
        aria-invalid={errField === 'secret'}
      />
      {#if errField === 'secret'}<div class="m-error">{error}</div>{/if}
    </div>

    <button class="advanced-toggle" onclick={() => (advanced = !advanced)}>
      <Icon name={advanced ? 'chevron-up' : 'chevron-down'} size={16} />
      {$t('totp_advanced')}
    </button>
    {#if advanced}
      <div class="advanced">
        <div class="m-field">
          <label for="alg">{$t('totp_field_algorithm')}</label>
          <CustomSelect
            id="alg"
            options={[
              { value: 'SHA1', label: 'SHA1' },
              { value: 'SHA256', label: 'SHA256' },
              { value: 'SHA512', label: 'SHA512' },
            ]}
            bind:value={algorithm}
          />
        </div>
        <div class="m-field">
          <label for="digits">{$t('totp_field_digits')}</label>
          <CustomSelect
            id="digits"
            options={[{ value: '6', label: '6' }, { value: '8', label: '8' }]}
            value={String(digits)}
            onchange={(v) => (digits = Number(v))}
          />
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

  </div>
  <!-- «Отмена | Сохранить» in the page's foot, over the keyboard; the error that is not a field's over them. -->
  <div class="page-foot">
    {#if error && !errField}<Notice tone="danger">{error}</Notice>{/if}
    <button type="button" class="btn btn-ghost" onclick={cancel} disabled={saving}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-primary" onclick={save} disabled={saving}>
      <Icon name="check" size={18} />
      {$t('common_save')}
    </button>
  </div>
</div>

<input bind:this={fileInput} type="file" accept="image/*" hidden onchange={pickedImage} />
<BottomSheet open={menu} title={$t('totp_add_title')} onclose={() => (menu = false)}>
  <div class="m-list">
    <button type="button" class="list-row dense" onclick={() => fileInput?.click()}>
      <Icon name="image" size={20} />
      <span class="list-row-body">
        <span class="list-row-label">{$t('totp_qr_select')}</span>
        <span class="list-row-line">{$t('totp_qr_hint')}</span>
      </span>
    </button>
  </div>
</BottomSheet>

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
  }
  .preview {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    margin-bottom: var(--sp-3);
    border-radius: var(--radius-md);
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
</style>

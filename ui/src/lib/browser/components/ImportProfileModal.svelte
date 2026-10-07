<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/browser/api';
  import type { Profile, ProfileExport } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { formatError } from '$lib/core/utils';
  import { torLabel } from '$lib/browser/proxy-label';

  interface Props {
    workspaceId: string;
    open: boolean;
    onclose: () => void;
    onimported: (p: Profile) => void;
  }

  let { workspaceId, open, onclose, onimported }: Props = $props();

  type Tab = 'file' | 'json';
  let activeTab = $state<Tab>('file');
  let pastedJson = $state('');
  let fileInput = $state<HTMLInputElement | null>(null);
  let selectedFile = $state<File | null>(null);
  let preview = $state<ProfileExport | null>(null);
  let previewError = $state('');
  let loading = $state(false);
  let error = $state('');

  function reset() {
    pastedJson = '';
    selectedFile = null;
    preview = null;
    previewError = '';
    error = '';
  }

  $effect(() => {
    if (!open) reset();
  });

  function handleFileChange(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0] ?? null;
    selectedFile = file;
    preview = null;
    previewError = '';
    error = '';

    if (!file) return;

    if (file.name.endsWith('.json')) {
      const reader = new FileReader();
      reader.onload = () => tryParsePreview(reader.result as string);
      reader.readAsText(file);
    } else if (!file.name.endsWith('.zip')) {
      previewError = $t('import_invalid_type');
      selectedFile = null;
    }
    // ZIP: no preview, just show filename
  }

  function handleJsonInput() {
    preview = null;
    previewError = '';
    if (pastedJson.trim()) tryParsePreview(pastedJson);
  }

  function tryParsePreview(json: string) {
    try {
      const parsed = JSON.parse(json) as ProfileExport;
      if (!parsed.version || !parsed.profile) {
        previewError = $t('profile_import_not_export');
        return;
      }
      preview = parsed;
    } catch {
      previewError = $t('profile_import_bad_json');
    }
  }

  async function doImport() {
    loading = true;
    error = '';
    try {
      let imported: Profile;

      if (activeTab === 'json') {
        imported = await api.profiles.importJson(pastedJson, workspaceId);
      } else if (selectedFile) {
        if (selectedFile.name.endsWith('.json')) {
          const text = await selectedFile.text();
          imported = await api.profiles.importJson(text, workspaceId);
        } else {
          const buf = await selectedFile.arrayBuffer();
          const b64 = arrayBufferToBase64(buf);
          imported = await api.profiles.importZipData(b64, workspaceId);
        }
      } else {
        error = $t('import_no_data');
        return;
      }

      onimported(imported);
      onclose();
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  function arrayBufferToBase64(buf: ArrayBuffer): string {
    const bytes = new Uint8Array(buf);
    let binary = '';
    for (let i = 0; i < bytes.byteLength; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
  }

  const canImport = $derived(
    !loading &&
    (activeTab === 'json' ? pastedJson.trim().length > 0 : selectedFile !== null)
  );
</script>

<Dialog {open} title={$t('import_title')} width="420px" {onclose}>

      <div class="tab-bar">
        <button
          class="tab"
          class:active={activeTab === 'file'}
          onclick={() => { activeTab = 'file'; reset(); }}
        >
          <Icon name="file" size={13} />
          {$t('import_tab_file')}
        </button>
        <button
          class="tab"
          class:active={activeTab === 'json'}
          onclick={() => { activeTab = 'json'; reset(); }}
        >
          <Icon name="code" size={13} />
          {$t('import_tab_json')}
        </button>
      </div>

      {#if activeTab === 'file'}
        <div class="file-section">
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="file-drop"
            class:has-file={selectedFile !== null}
            onclick={() => fileInput?.click()}
            role="button"
            tabindex="0"
            onkeydown={(e) => e.key === 'Enter' && fileInput?.click()}
          >
            {#if selectedFile}
              <Icon name="file-check" size={20} />
              <span class="file-name">{selectedFile.name}</span>
              <span class="file-meta">{(selectedFile.size / 1024).toFixed(1)} KB</span>
            {:else}
              <Icon name="upload" size={20} />
              <span>{$t('import_drop_hint')}</span>
              <span class="file-meta">{$t('import_drop_type')}</span>
            {/if}
          </div>
          <input
            type="file"
            accept=".json,.zip"
            style="display:none"
            bind:this={fileInput}
            onchange={handleFileChange}
          />
        </div>
      {:else}
        <textarea
          class="json-input mono"
          placeholder={$t('import_json_placeholder')}
          bind:value={pastedJson}
          oninput={handleJsonInput}
          rows="8"
          spellcheck="false"
        ></textarea>
      {/if}

      {#if previewError}
        <div class="error-msg">{previewError}</div>
      {/if}

      {#if preview}
        <div class="preview">
          <div class="preview-title">{$t('import_preview_title')}</div>
          <div class="preview-row">
            <span class="preview-label">{$t('table_col_name')}</span>
            <span class="preview-value">{preview.profile.name}</span>
          </div>
          <div class="preview-row">
            <span class="preview-label">OS</span>
            <span class="preview-value">{preview.profile.fingerprint_preset} / {preview.profile.browser_type}</span>
          </div>
          <div class="preview-row">
            <span class="preview-label">{$t('profile_field_locale')}</span>
            <span class="preview-value">{preview.profile.locale}</span>
          </div>
          {#if preview.proxy}
            <div class="preview-row">
              <span class="preview-label">{$t('panel_proxy')}</span>
              <span class="preview-value proxy-value">
                <Icon name="globe" size={11} />
                {#if preview.proxy.proxy_type === 'tor'}
                  {torLabel(preview.proxy.country, $t)}
                {:else}
                  {preview.proxy.proxy_type}://{preview.proxy.host}:{preview.proxy.port}
                {/if}
              </span>
            </div>
          {/if}
        </div>
      {/if}

      {#if selectedFile?.name.endsWith('.zip') && !previewError}
        <div class="zip-note">
          <Icon name="info" size={12} />
          {$t('import_zip_note')}
        </div>
      {/if}

      {#if error}
        <div class="error-msg">{error}</div>
      {/if}

  {#snippet footer()}
        <button class="btn btn-ghost" onclick={onclose}>{$t('profile_btn_cancel')}</button>
        <button class="btn btn-primary" disabled={!canImport} onclick={doImport}>
          {loading ? $t('import_btn_loading') : $t('import_btn')}
        </button>
  {/snippet}

</Dialog>

<style>
  /* .tab-bar / .tab styling is global (base.css); only local deltas kept below */
  .tab-bar { padding-top: 0.3rem; }
  .tab { justify-content: center; }

  .file-section {
    display: flex;
    flex-direction: column;
  }

  .file-drop {
    border: 2px dashed var(--border-2);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    padding: var(--sp-6);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
    color: var(--text-2);
    font-size: var(--fs-base);
    transition: border-color 0.15s, background 0.15s;
  }
  .file-drop:hover, .file-drop:focus {
    border-color: var(--accent-border);
    background: var(--accent-tint);
    outline: none;
  }
  .file-drop.has-file {
    border-color: var(--accent-border);
    color: var(--text);
  }

  .file-name { font-weight: var(--fw-medium); color: var(--text); }
  .file-meta { font-size: var(--fs-sm); color: var(--text-3); }

  .json-input {
    width: 100%;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-size: var(--fs-sm);
    padding: 0.6rem var(--sp-3);
    resize: vertical;
    box-sizing: border-box;
  }
  .json-input:focus {
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--accent-bg);
  }

  .preview {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 0.6rem var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .preview-title {
    font-size: var(--fs-2xs);
    font-weight: var(--fw-bold);
    text-transform: uppercase;
    color: var(--text-dim);
    letter-spacing: 0.7px;
    margin-bottom: 0.2rem;
  }

  .preview-row {
    display: flex;
    gap: var(--sp-2);
    font-size: var(--fs-sm);
  }

  .preview-label {
    color: var(--text-3);
    width: 52px;
    flex-shrink: 0;
  }

  .preview-value { color: var(--text); }

  .proxy-value {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }

  .zip-note {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/browser/api';
  import type { ExportOptions, Profile, Proxy } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { formatError } from '$lib/core/utils';

  interface Props {
    profile: Profile;
    proxy: Proxy | null;
    open: boolean;
    onclose: () => void;
  }

  let { profile, proxy, open, onclose }: Props = $props();

  let includeProxy = $state(false);
  let includeProxyPassword = $state(false);
  let includeFiles = $state(false);
  let loading = $state<'json' | 'zip' | 'preview' | null>(null);
  let error = $state('');
  let successMsg = $state('');
  let previewJson = $state<string | null>(null);
  let copied = $state(false);

  $effect(() => {
    if (open) {
      includeProxy = !!proxy;
      includeProxyPassword = false;
      includeFiles = false;
      error = '';
      successMsg = '';
      previewJson = null;
      copied = false;
    }
  });

  function getOptions(): ExportOptions {
    return {
      include_proxy: includeProxy,
      include_proxy_password: includeProxy && includeProxyPassword,
      include_files: includeFiles,
    };
  }

  async function togglePreview() {
    if (previewJson !== null) { previewJson = null; return; }
    loading = 'preview';
    error = '';
    try {
      previewJson = await api.profiles.exportJson(profile.id, getOptions());
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = null;
    }
  }

  async function copyJson() {
    if (!previewJson) return;
    await navigator.clipboard.writeText(previewJson);
    copied = true;
    setTimeout(() => (copied = false), 2000);
  }

  async function saveJson() {
    loading = 'json';
    error = '';
    successMsg = '';
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const path = await save({
        defaultPath: `${profile.name.replace(/[^a-z0-9_-]/gi, '_')}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!path) { loading = null; return; }

      await api.profiles.exportJsonToFile(profile.id, getOptions(), path);
      successMsg = `JSON saved: ${path}`;
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = null;
    }
  }

  async function saveZip() {
    loading = 'zip';
    error = '';
    successMsg = '';
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const path = await save({
        defaultPath: `${profile.name.replace(/[^a-z0-9_-]/gi, '_')}.zip`,
        filters: [{ name: 'ZIP archive', extensions: ['zip'] }],
      });
      if (!path) { loading = null; return; }

      await api.profiles.exportZip(profile.id, getOptions(), path);
      successMsg = `ZIP saved: ${path}`;
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = null;
    }
  }
</script>

<Dialog {open} title={$t('export_title')} width="480px" {onclose}>

      <div class="profile-name">{profile.name}</div>

      <div class="options">
        <label class="option-row">
          <input type="checkbox" bind:checked={includeProxy} disabled={!proxy} />
          <span class:muted={!proxy}>
            {$t('export_include_proxy')}
            {#if proxy}
              <span class="hint">({proxy.name})</span>
            {:else}
              <span class="hint">{$t('export_no_proxy')}</span>
            {/if}
          </span>
        </label>

        {#if includeProxy && proxy}
          <label class="option-row indent">
            <input type="checkbox" bind:checked={includeProxyPassword} />
            <span>{$t('export_include_password')}</span>
            <span class="badge-sensitive">{$t('export_sensitive')}</span>
          </label>
        {/if}

        <label class="option-row">
          <input type="checkbox" bind:checked={includeFiles} />
          <span>
            {$t('export_include_files')}
            <span class="hint">{$t('export_include_files_hint')}</span>
          </span>
        </label>

        {#if includeFiles}
          <p class="note">{$t('export_files_note')}</p>
        {/if}
      </div>

      {#if previewJson !== null}
        <div class="json-preview">
          <div class="json-preview-header">
            <span class="json-preview-label">JSON</span>
            <button class="copy-btn" onclick={copyJson}>
              <Icon name={copied ? 'check' : 'copy'} size={12} />
              {copied ? $t('export_btn_copied') : $t('export_btn_copy')}
            </button>
          </div>
          <textarea class="json-area mono" readonly value={previewJson} spellcheck="false"></textarea>
        </div>
      {/if}

      {#if error}
        <div class="error-msg">{error}</div>
      {/if}
      {#if successMsg}
        <div class="success-msg">{successMsg}</div>
      {/if}

  {#snippet footer()}
        <button class="btn btn-ghost btn-sm cancel-left" onclick={onclose}>{$t('profile_btn_cancel')}</button>
        <div class="footer-right">
          <button class="btn btn-ghost btn-sm" disabled={loading !== null} onclick={togglePreview}>
            <Icon name={loading === 'preview' ? 'loader' : previewJson !== null ? 'eye-off' : 'eye'} size={13} />
            {previewJson !== null ? 'JSON' : $t('export_btn_preview')}
          </button>
          <button class="btn btn-ghost btn-sm" disabled={loading !== null} onclick={saveJson}>
            <Icon name={loading === 'json' ? 'loader' : 'file-json'} size={13} />
            JSON
          </button>
          <button class="btn btn-primary btn-sm" disabled={loading !== null} onclick={saveZip}>
            <Icon name={loading === 'zip' ? 'loader' : 'archive'} size={13} />
            ZIP
          </button>
        </div>
  {/snippet}

</Dialog>

<style>
  .profile-name {
    font-size: var(--fs-base);
    font-weight: var(--fw-semibold);
    color: var(--text);
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.45rem 0.75rem;
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .option-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-base);
    color: var(--text);
    cursor: pointer;
  }

  .option-row.indent { padding-left: 1.4rem; }

  .option-row input[type="checkbox"] {
    flex-shrink: 0;
  }

  .hint { font-size: var(--fs-sm); color: var(--text-3); }
  .muted { color: var(--text-3); }

  .badge-sensitive {
    font-size: var(--fs-2xs);
    font-weight: var(--fw-semibold);
    background: var(--danger-bg);
    color: var(--danger-text);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-xs);
    padding: 1px 5px;
  }

  .note {
    font-size: var(--fs-sm);
    color: var(--text-3);
    margin: 0;
    padding-left: 1.4rem;
  }

  /* JSON preview — mono block on --surface-3, nested-card radius */
  .json-preview {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .json-preview-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.35rem 0.6rem;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }

  .json-preview-label {
    font-size: var(--fs-2xs);
    font-weight: var(--fw-bold);
    text-transform: uppercase;
    letter-spacing: 0.7px;
    color: var(--text-dim);
  }

  .copy-btn {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    background: none;
    border: none;
    font-size: var(--fs-sm);
    color: var(--accent-text-3);
    cursor: pointer;
    padding: 0.15rem 0.4rem;
    border-radius: var(--radius-xs);
    transition: all var(--dur-fast);
  }
  .copy-btn:hover { background: var(--accent-bg); }

  .json-area {
    width: 100%;
    height: 180px;
    background: var(--surface-3);
    border: none;
    color: var(--text);
    font-size: var(--fs-xs);
    padding: 0.6rem var(--sp-3);
    resize: none;
    box-sizing: border-box;
    display: block;
    outline: none;
    line-height: 1.6;
  }

  .success-msg {
    font-size: var(--fs-base);
    color: var(--success-text);
    background: var(--success-bg);
    border: 1px solid var(--success-border);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.6rem;
    word-break: break-all;
  }

  .cancel-left {
    margin-right: auto;
  }

  .footer-right {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
</style>

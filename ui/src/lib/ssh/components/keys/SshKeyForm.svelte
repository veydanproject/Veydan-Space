<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '$lib/ssh/api';
  import type { SshKey } from '$lib/ssh/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';

  interface Props {
    mode?: 'import' | 'generate';
    onSave: (key: SshKey) => void;
    onCancel: () => void;
  }

  let { mode = 'import', onSave, onCancel }: Props = $props();

  let activeMode = $state<'import' | 'generate'>(untrack(() => mode));
  let saving = $state(false);
  let error = $state('');

  let name = $state('');
  // import
  let privateKey = $state('');
  let importPassphrase = $state('');
  // generate — one combined select: algo or algo:bits
  let algoChoice = $state('ed25519');
  let comment = $state('');
  let genPassphrase = $state('');
  let genPassphraseConfirm = $state('');
  // after successful generation: show the public key before closing
  let createdKey = $state<SshKey | null>(null);
  let copied = $state(false);

  const ALGO_OPTIONS = [
    { value: 'ed25519', label: 'Ed25519' },
    { value: 'rsa:4096', label: 'RSA 4096' },
    { value: 'rsa:3072', label: 'RSA 3072' },
    { value: 'rsa:2048', label: 'RSA 2048' },
    { value: 'ecdsa:256', label: 'ECDSA P-256' },
    { value: 'ecdsa:384', label: 'ECDSA P-384' },
    { value: 'ecdsa:521', label: 'ECDSA P-521' },
  ];

  function mapError(e: unknown): string {
    const msg = formatError(e);
    if (msg.includes('key_encrypted')) return $t('ssh_key_error_encrypted');
    if (msg.includes('key_wrong_passphrase')) return $t('ssh_key_error_wrong_passphrase');
    return msg;
  }

  async function save() {
    if (!name.trim()) {
      error = $t('ssh_key_error_name_required');
      return;
    }
    if (activeMode === 'import' && !privateKey.trim()) {
      error = $t('ssh_key_error_key_required');
      return;
    }
    if (activeMode === 'generate' && genPassphrase !== genPassphraseConfirm) {
      error = $t('ssh_key_error_passphrase_mismatch');
      return;
    }
    saving = true;
    error = '';
    try {
      if (activeMode === 'import') {
        const key = await api.sshKeys.import({
          name: name.trim(),
          private_key: privateKey,
          passphrase: importPassphrase || null,
        });
        onSave(key);
      } else {
        const [algorithm, bits] = algoChoice.split(':');
        const key = await api.sshKeys.generate({
          name: name.trim(),
          algorithm: algorithm as 'ed25519' | 'rsa' | 'ecdsa',
          bits: bits ? Number(bits) : null,
          comment: comment.trim() || null,
          passphrase: genPassphrase || null,
        });
        // Show the public key right away — the user needs it for authorized_keys.
        createdKey = key;
      }
    } catch (e: unknown) {
      error = mapError(e);
    } finally {
      saving = false;
    }
  }

  async function copyPublic() {
    if (!createdKey) return;
    try {
      await navigator.clipboard.writeText(createdKey.public_key);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {}
  }
</script>

<div class="form">
  {#if createdKey}
    <div class="created-hint">
      <Icon name="check-circle" size={15} />
      {$t('ssh_key_created_hint')}
    </div>
    <div class="form-group">
      <span class="field-label">{$t('ssh_key_public_label')}</span>
      <textarea class="mono" readonly rows="4" value={createdKey.public_key}></textarea>
    </div>
    <div class="form-actions">
      <button class="btn btn-ghost" onclick={copyPublic}>
        <Icon name={copied ? 'check' : 'copy'} size={13} />
        {copied ? $t('ssh_key_copied') : $t('ssh_key_btn_copy_public')}
      </button>
      <button class="btn btn-primary" onclick={() => onSave(createdKey!)}>
        {$t('ssh_key_btn_done')}
      </button>
    </div>
  {:else}
    <div class="seg">
      <button
        type="button"
        class="seg-btn"
        class:active={activeMode === 'import'}
        onclick={() => { activeMode = 'import'; error = ''; }}
      >
        <Icon name="download" size={13} /> {$t('ssh_key_form_import')}
      </button>
      <button
        type="button"
        class="seg-btn"
        class:active={activeMode === 'generate'}
        onclick={() => { activeMode = 'generate'; error = ''; }}
      >
        <Icon name="zap" size={13} /> {$t('ssh_key_form_generate')}
      </button>
    </div>

    {#if error}
      <div class="error-msg">{error}</div>
    {/if}

    <div class="form-group">
      <label for="ssh-key-name">{$t('ssh_key_field_name')}</label>
      <input
        id="ssh-key-name"
        type="text"
        bind:value={name}
        placeholder={$t('ssh_key_field_name_placeholder')}
      />
    </div>

    {#if activeMode === 'import'}
      <div class="form-group">
        <label for="ssh-key-pem">{$t('ssh_key_field_private_key')}</label>
        <textarea
          class="mono"
          id="ssh-key-pem"
          bind:value={privateKey}
          rows="7"
          placeholder="-----BEGIN OPENSSH PRIVATE KEY-----&#10;..."
        ></textarea>
      </div>
      <div class="form-group">
        <label for="ssh-key-import-pass">{$t('ssh_key_field_passphrase')}</label>
        <input
          id="ssh-key-import-pass"
          type="password"
          bind:value={importPassphrase}
          placeholder="••••••••"
          autocomplete="new-password"
        />
        <span class="hint">{$t('ssh_key_field_passphrase_hint_import')}</span>
      </div>
    {:else}
      <div class="form-group">
        <label for="ssh-key-algo">{$t('ssh_key_field_algorithm')}</label>
        <select id="ssh-key-algo" bind:value={algoChoice}>
          {#each ALGO_OPTIONS as opt}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>
      <div class="form-group">
        <label for="ssh-key-comment">{$t('ssh_key_field_comment')}</label>
        <input
          id="ssh-key-comment"
          type="text"
          bind:value={comment}
          placeholder={$t('ssh_key_field_comment_placeholder')}
        />
      </div>
      <div class="form-row">
        <div class="form-group">
          <label for="ssh-key-gen-pass">{$t('ssh_key_field_passphrase')}</label>
          <input
            id="ssh-key-gen-pass"
            type="password"
            bind:value={genPassphrase}
            placeholder="••••••••"
            autocomplete="new-password"
          />
        </div>
        <div class="form-group">
          <label for="ssh-key-gen-pass2">{$t('ssh_key_field_passphrase_confirm')}</label>
          <input
            id="ssh-key-gen-pass2"
            type="password"
            bind:value={genPassphraseConfirm}
            placeholder="••••••••"
            autocomplete="new-password"
          />
        </div>
      </div>
      <span class="hint">{$t('ssh_key_field_passphrase_hint_generate')}</span>
    {/if}

    <div class="form-actions">
      <button class="btn btn-ghost" onclick={onCancel}>{$t('ssh_btn_cancel')}</button>
      <button class="btn btn-primary" onclick={save} disabled={saving}>
        {#if saving}<Icon name="loader" size={13} />{/if}
        {#if saving && activeMode === 'generate'}
          {$t('ssh_key_generating')}
        {:else}
          {activeMode === 'import' ? $t('ssh_key_btn_import') : $t('ssh_key_btn_generate')}
        {/if}
      </button>
    </div>
  {/if}
</div>

<style>
  .form { display: flex; flex-direction: column; gap: var(--sp-4); }
  .form-row { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .form-group { display: flex; flex-direction: column; gap: 8px; }
  label, .field-label {
    font-size: 0.72rem; text-transform: uppercase; color: var(--text-faint);
    font-weight: var(--fw-bold); letter-spacing: 0.5px;
  }
  input, select, textarea {
    /* background-color only: a select keeps the chevron of base.css */
    background-color: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    color: var(--text);
    padding: 0 14px;
    height: var(--control-h-lg);
    font-size: 0.9rem;
    font-family: inherit;
    resize: vertical;
  }
  select { padding-right: 2.2rem; }
  textarea { height: auto; min-height: 96px; padding: 12px 14px; font-size: 0.8rem; }
  input:focus, select:focus, textarea:focus {
    outline: none;
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--accent-bg);
  }
  .hint { font-size: var(--fs-xs); color: var(--text-faint); }
  .seg { align-self: flex-start; }
  .created-hint {
    display: flex; align-items: center; gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    background: var(--success-bg); color: var(--success-text);
    border: 1px solid color-mix(in srgb, var(--success) 30%, var(--border));
    border-radius: var(--radius-md);
    font-size: var(--fs-sm);
  }
  .form-actions { display: flex; gap: 10px; justify-content: flex-end; margin-top: var(--sp-2); }
  .form-actions .btn { height: 42px; border-radius: var(--radius-field); padding: 0 20px; }
</style>

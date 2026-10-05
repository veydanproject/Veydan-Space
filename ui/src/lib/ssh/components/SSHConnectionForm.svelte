<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '$lib/ssh/api';
  import type {
    SshConnection,
    SshConnectionCreateInput,
    SshConnectionUpdateInput,
    SshKey,
  } from '$lib/ssh/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { registry } from '$lib/core/registry';
  import { directory } from '$lib/core/directory';
  import { explainError } from '$lib/ssh/tor-error';

  interface Props {
    connection?: SshConnection | null;
    defaultWorkspaceId?: string | null;
    defaultProfileId?: string | null;
    onSave: (conn: SshConnection) => void;
    onCancel: () => void;
  }

  let {
    connection = null,
    defaultWorkspaceId = null,
    defaultProfileId = null,
    onSave,
    onCancel,
  }: Props = $props();

  // Proxies, workspaces and TOTP entries come through the catalog of entities (their owners are other modules).
  let proxies = $state<{ id: string; name: string; proxy_type: string; subtitle: string }[]>([]);
  let workspaces = $state<{ id: string; name: string }[]>([]);
  let totpEntries = $state<{ id: string; name: string; issuer: string }[]>([]);
  let sshKeys = $state<SshKey[]>([]);
  let sshKeysLoaded = $state(false);
  let saving = $state(false);
  let error = $state('');

  let name = $state(untrack(() => connection?.name ?? ''));
  let host = $state(untrack(() => connection?.host ?? ''));
  let port = $state(untrack(() => connection?.port ?? 22));
  let username = $state(untrack(() => connection?.username ?? ''));
  let authType = $state(untrack(() => connection?.auth_type ?? 'password'));
  // Secrets are never sent to the UI. Fields start empty; an untouched field
  // keeps the stored value, a touched one replaces or clears it.
  let password = $state('');
  let privateKey = $state('');
  let keyPassphrase = $state('');
  let passwordTouched = $state(false);
  let privateKeyTouched = $state(false);
  let keyPassphraseTouched = $state(false);
  let storedPassword = $state(untrack(() => connection?.has_password ?? false));
  let storedPrivateKey = $state(untrack(() => connection?.has_private_key ?? false));
  let storedKeyPassphrase = $state(untrack(() => connection?.has_key_passphrase ?? false));

  function secretPlaceholder(stored: boolean, touched: boolean, fallback: string): string {
    return stored && !touched ? $t('secret_stored_placeholder') : fallback;
  }

  /** Value to send: `undefined` keeps the stored secret; `''` clears it. */
  function secretValue(value: string, touched: boolean): string | undefined {
    if (!connection) return value;
    return touched ? value : undefined;
  }
  let keySource = $state<'saved' | 'inline'>(untrack(() => (connection?.ssh_key_id ? 'saved' : 'inline')));
  let sshKeyId = $state(untrack(() => connection?.ssh_key_id ?? ''));
  let requires2fa = $state(untrack(() => connection?.requires_2fa ?? false));
  let totpEntryId = $state(untrack(() => connection?.totp_entry_id ?? ''));
  let proxyId = $state(untrack(() => connection?.proxy_id ?? ''));
  /**
   * The saved proxy no longer exists (backend resolved no name for it). Derived
   * from the stored row rather than from `proxies`, so it does not flash while
   * that list loads, and it clears as soon as the user picks another proxy.
   */
  const danglingProxy = $derived(
    !!proxyId && proxyId === connection?.proxy_id && !connection?.proxy_name
  );
  let selectedWorkspaceIds = $state<string[]>(
    untrack(() => connection?.workspace_ids ?? (defaultWorkspaceId ? [defaultWorkspaceId] : []))
  );
  let selectedProfileIds = $state<string[]>(
    untrack(() => connection?.profile_ids ?? (defaultProfileId ? [defaultProfileId] : []))
  );
  let connectTimeout = $state(untrack(() => connection?.connect_timeout_sec ?? 15));
  let keepalive = $state(untrack(() => connection?.keepalive_sec ?? 30));
  let defaultCols = $state(untrack(() => connection?.default_cols ?? 120));
  let defaultRows = $state(untrack(() => connection?.default_rows ?? 32));

  $effect(() => {
    directory.ensureLoaded().then(() => {
      proxies = directory.list('proxy').map((p) => ({ id: p.id, name: p.name, proxy_type: directory.fields('proxy', p.id).type ?? '', subtitle: p.subtitle }));
      workspaces = directory.list('workspace').map((w) => ({ id: w.id, name: w.name }));
      totpEntries = directory.list('totp').map((e) => {
        const fields = directory.fields('totp', e.id);
        return { id: e.id, name: fields.name ?? e.name, issuer: fields.issuer ?? '' };
      });
    });
    api.sshKeys.list().then((list) => {
      sshKeys = list;
      sshKeysLoaded = true;
    });
  });

  let selectedKey = $derived(sshKeys.find((k) => k.id === sshKeyId));
  /** The connection references a key that no longer exists (deleted). */
  let savedKeyMissing = $derived(
    keySource === 'saved' && sshKeysLoaded && !!sshKeyId && !selectedKey
  );

  function keyOptionLabel(k: SshKey): string {
    return k.bits ? `${k.name} — ${k.algorithm} ${k.bits}` : `${k.name} — ${k.algorithm}`;
  }

  function toggleWorkspace(id: string) {
    if (selectedWorkspaceIds.includes(id)) {
      selectedWorkspaceIds = selectedWorkspaceIds.filter((x) => x !== id);
    } else {
      selectedWorkspaceIds = [...selectedWorkspaceIds, id];
    }
  }

  async function save() {
    if (!name.trim() || !host.trim() || !username.trim()) {
      error = $t('ssh_error_required');
      return;
    }
    const isKeyAuth = authType === 'key' || authType === 'key_password';
    const useSavedKey = isKeyAuth && keySource === 'saved';
    if (useSavedKey && (!sshKeyId || savedKeyMissing)) {
      error = $t('ssh_key_error_select_required');
      return;
    }
    saving = true;
    error = '';
    try {
      let conn: SshConnection;
      // Nullable fields are three-state on the backend: omitted = keep,
      // '' = clear, value = set. The form always sends the full desired
      // state, so '' expresses "cleared / detached" explicitly (create
      // normalizes '' to NULL).
      const base = {
        name: name.trim(),
        host: host.trim(),
        port,
        username: username.trim(),
        auth_type: authType,
        password: secretValue(password, passwordTouched),
        // Saved key: reference only — never persist stale inline material.
        private_key: useSavedKey ? '' : secretValue(privateKey, privateKeyTouched),
        key_passphrase: useSavedKey ? '' : secretValue(keyPassphrase, keyPassphraseTouched),
        ssh_key_id: useSavedKey ? sshKeyId : '',
        requires_2fa: requires2fa,
        totp_entry_id: totpEntryId,
        proxy_id: proxyId,
        workspace_ids: selectedWorkspaceIds,
        profile_ids: selectedProfileIds,
        connect_timeout_sec: connectTimeout,
        keepalive_sec: keepalive,
        default_cols: defaultCols,
        default_rows: defaultRows,
      };
      if (connection) {
        conn = await api.ssh.connectionUpdate(connection.id, base as SshConnectionUpdateInput);
      } else {
        conn = await api.ssh.connectionCreate(base as SshConnectionCreateInput);
      }
      onSave(conn);
    } catch (e: unknown) {
      error = explainError(e, $t);
    } finally {
      saving = false;
    }
  }
</script>

<div class="form">
  {#if error}
    <div class="error-msg">{error}</div>
  {/if}

  <!-- Name takes the whole width, like every field below it -->
  <div class="form-group">
    <label for="ssh-name">{$t('ssh_field_name')}</label>
    <input id="ssh-name" type="text" bind:value={name} placeholder={$t('ssh_field_name_placeholder')} />
  </div>

  <div class="form-row host-port">
    <div class="form-group">
      <label for="ssh-host">{$t('ssh_field_host')}</label>
      <input id="ssh-host" type="text" bind:value={host} placeholder={$t('ssh_field_host_placeholder')} />
    </div>
    <div class="form-group">
      <label for="ssh-port">{$t('ssh_field_port')}</label>
      <input id="ssh-port" type="number" bind:value={port} min="1" max="65535" />
    </div>
  </div>

  <div class="form-group">
    <label for="ssh-username">{$t('ssh_field_username')}</label>
    <input id="ssh-username" type="text" bind:value={username} placeholder={$t('ssh_field_username_placeholder')} />
  </div>

  <div class="form-group">
    <label for="ssh-auth-type">{$t('ssh_field_auth_type')}</label>
    <select id="ssh-auth-type" bind:value={authType}>
      <option value="password">{$t('ssh_auth_password')}</option>
      <option value="key">{$t('ssh_auth_key')}</option>
      <option value="key_password">{$t('ssh_auth_key_password')}</option>
    </select>
  </div>

  {#if authType === 'password'}
    <div class="form-group">
      <label for="ssh-password">{$t('ssh_field_password')}</label>
      <input
        id="ssh-password"
        type="password"
        bind:value={password}
        oninput={() => (passwordTouched = true)}
        placeholder={secretPlaceholder(storedPassword, passwordTouched, '••••••••')}
        autocomplete="new-password"
      />
      {#if storedPassword && !passwordTouched}
        <button type="button" class="link-btn" onclick={() => { password = ''; passwordTouched = true; storedPassword = false; }}>
          {$t('secret_clear')}
        </button>
      {/if}
    </div>
  {/if}

  {#if authType === 'key' || authType === 'key_password'}
    <div class="form-group">
      <span class="field-label">{$t('ssh_key_source_label')}</span>
      <div class="seg">
        <button
          type="button"
          class="seg-btn"
          class:active={keySource === 'saved'}
          onclick={() => (keySource = 'saved')}
        >
          <Icon name="key" size={12} /> {$t('ssh_key_source_saved')}
        </button>
        <button
          type="button"
          class="seg-btn"
          class:active={keySource === 'inline'}
          onclick={() => (keySource = 'inline')}
        >
          <Icon name="pencil" size={12} /> {$t('ssh_key_source_inline')}
        </button>
      </div>
    </div>

    {#if keySource === 'saved'}
      <div class="form-group">
        <label for="ssh-saved-key">{$t('ssh_key_field_saved_key')}</label>
        <select id="ssh-saved-key" bind:value={sshKeyId}>
          <option value="">{$t('ssh_key_select_placeholder')}</option>
          {#each sshKeys as k}
            <option value={k.id}>{keyOptionLabel(k)}</option>
          {/each}
        </select>
        {#if savedKeyMissing}
          <span class="hint warn">{$t('ssh_key_missing_warning')}</span>
        {:else if selectedKey?.has_passphrase}
          <span class="hint">{$t('ssh_key_selected_passphrase_hint')}</span>
        {:else if sshKeysLoaded && sshKeys.length === 0}
          <span class="hint">{$t('ssh_key_none_hint')}</span>
        {/if}
      </div>
    {:else}
      <div class="form-group">
        <label for="ssh-private-key">{$t('ssh_field_private_key')}</label>
        <textarea
          class="mono"
          id="ssh-private-key"
          bind:value={privateKey}
          oninput={() => (privateKeyTouched = true)}
          rows="5"
          placeholder={secretPlaceholder(storedPrivateKey, privateKeyTouched, '-----BEGIN OPENSSH PRIVATE KEY-----\n...')}
        ></textarea>
        {#if storedPrivateKey && !privateKeyTouched}
          <span class="hint">{$t('secret_stored_hint')}</span>
        {/if}
      </div>
      {#if authType === 'key_password'}
        <div class="form-group">
          <label for="ssh-key-passphrase">{$t('ssh_field_key_passphrase')}</label>
          <input
            id="ssh-key-passphrase"
            type="password"
            bind:value={keyPassphrase}
            oninput={() => (keyPassphraseTouched = true)}
            placeholder={secretPlaceholder(storedKeyPassphrase, keyPassphraseTouched, '••••••••')}
            autocomplete="new-password"
          />
        </div>
      {/if}
    {/if}
    <div class="form-group">
      <label for="ssh-password-ki">{$t('ssh_field_password_ki')}</label>
      <input
        id="ssh-password-ki"
        type="password"
        bind:value={password}
        oninput={() => (passwordTouched = true)}
        placeholder={secretPlaceholder(storedPassword, passwordTouched, $t('ssh_field_password_ki_placeholder'))}
        autocomplete="new-password"
      />
      {#if storedPassword && !passwordTouched}
        <button type="button" class="link-btn" onclick={() => { password = ''; passwordTouched = true; storedPassword = false; }}>
          {$t('secret_clear')}
        </button>
      {/if}
    </div>
  {/if}

  <div class="toggle-row">
    <div class="toggle-info">
      <span class="toggle-text">{$t('ssh_field_requires_2fa')}</span>
    </div>
    <button
      type="button"
      class="toggle-btn"
      class:active={requires2fa}
      onclick={() => (requires2fa = !requires2fa)}
      aria-pressed={requires2fa}
      aria-label={$t('ssh_field_requires_2fa')}
    >
      <span class="toggle-thumb"></span>
    </button>
  </div>

  {#if requires2fa}
    <div class="form-group">
      <label for="ssh-totp-entry">{$t('ssh_field_totp_entry')}</label>
      <select id="ssh-totp-entry" bind:value={totpEntryId}>
        <option value="">{$t('ssh_totp_entry_interactive')}</option>
        {#each totpEntries as entry}
          <option value={entry.id}>{entry.name}{entry.issuer ? ` (${entry.issuer})` : ''}</option>
        {/each}
      </select>
      <span class="hint">
        {#if totpEntryId}
          {$t('ssh_totp_entry_hint_auto')}
        {:else}
          {$t('ssh_totp_entry_hint_interactive')}
        {/if}
      </span>
    </div>
  {/if}

  <div class="form-group">
    <label for="ssh-proxy">{$t('ssh_field_proxy')}</label>
    <select id="ssh-proxy" bind:value={proxyId}>
      <option value="">{$t('ssh_proxy_none')}</option>
      <!-- The stored proxy was deleted, so it is absent from the list above.
           Without this option the select would render blank, which reads as
           "no proxy" while the connection actually refuses to connect. -->
      {#if danglingProxy}
        <option value={proxyId} disabled>{$t('ssh_proxy_missing')}</option>
      {/if}
      {#each proxies as p}
        <!-- A Tor row names no server: its subtitle (`Tor · DE, NL`) says which exit countries it uses. -->
        <option value={p.id}>[{p.proxy_type === 'tor' ? p.subtitle : p.proxy_type}] {p.name}</option>
      {/each}
    </select>
    {#if danglingProxy}
      <span class="hint warn">{$t('ssh_proxy_missing_hint')}</span>
    {/if}
  </div>

  <!-- Workspace assignments -->
  {#if workspaces.length > 0}
    <div class="form-group">
      <span class="field-label">{$t('ssh_field_workspaces')}</span>
      <div class="tag-grid">
        {#each workspaces as ws}
          <button
            type="button"
            class="chip"
            class:active={selectedWorkspaceIds.includes(ws.id)}
            onclick={() => toggleWorkspace(ws.id)}
          >
            {#if selectedWorkspaceIds.includes(ws.id)}<Icon name="check" size={11} />{/if}
            {ws.name}
          </button>
        {/each}
      </div>
      {#if selectedWorkspaceIds.length === 0}
        <span class="hint">{$t('ssh_hint_global')}</span>
      {/if}
    </div>
  {/if}

  <details class="advanced">
    <summary>{$t('ssh_advanced')}</summary>
    <div class="advanced-body">
      <div class="form-row">
        <div class="form-group">
          <label for="ssh-connect-timeout">{$t('ssh_field_connect_timeout')}</label>
          <input id="ssh-connect-timeout" type="number" bind:value={connectTimeout} min="1" max="120" />
        </div>
        <div class="form-group">
          <label for="ssh-keepalive">{$t('ssh_field_keepalive')}</label>
          <input id="ssh-keepalive" type="number" bind:value={keepalive} min="0" max="300" />
        </div>
      </div>
      <div class="form-row">
        <div class="form-group">
          <label for="ssh-default-cols">{$t('ssh_field_default_cols')}</label>
          <input id="ssh-default-cols" type="number" bind:value={defaultCols} min="40" max="500" />
        </div>
        <div class="form-group">
          <label for="ssh-default-rows">{$t('ssh_field_default_rows')}</label>
          <input id="ssh-default-rows" type="number" bind:value={defaultRows} min="10" max="200" />
        </div>
      </div>
    </div>
  </details>

  <div class="form-actions">
    <button class="btn btn-ghost" onclick={onCancel}>{$t('ssh_btn_cancel')}</button>
    <button class="btn btn-primary" onclick={save} disabled={saving}>
      {#if saving}<Icon name="loader" size={13} />{/if}
      {connection ? $t('ssh_btn_save') : $t('ssh_btn_create')}
    </button>
  </div>
  {#if connection}
    <!-- What other modules show about a connection: the notes about it. -->
    {#each registry.views('ssh', 'inline') as view (view.id)}
      <view.component id={connection.id} name={connection.name} />
    {/each}
  {/if}
</div>

<style>
  .form { display: flex; flex-direction: column; gap: var(--sp-4); }
  .form-row { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .form-row.host-port { grid-template-columns: 3fr 1fr; }
  .form-group { display: flex; flex-direction: column; gap: 8px; }
  /* Caps field labels per redesign drawer-form spec */
  label, .field-label {
    font-size: 0.72rem; text-transform: uppercase; color: var(--text-faint);
    font-weight: var(--fw-bold); letter-spacing: 0.5px;
  }
  /* Tall drawer form fields (46px / radius 11) */
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
  .toggle-row {
    display: flex; align-items: center; justify-content: space-between;
    border-top: 1px solid var(--border); border-bottom: 1px solid var(--border);
    padding: 1rem 0;
  }
  .toggle-info { display: flex; flex-direction: column; gap: 4px; }
  .toggle-text { font-size: 0.95rem; font-weight: var(--fw-semibold); color: var(--text); text-transform: none; letter-spacing: 0; }
  .toggle-btn {
    position: relative; flex-shrink: 0;
    width: 54px; height: 30px;
    background: var(--toggle-off); border: none; border-radius: 999px;
    cursor: pointer; padding: 0; transition: background 0.2s;
  }
  .toggle-btn.active { background: var(--accent-hover); }
  .toggle-thumb {
    position: absolute; top: 3px; left: 3px;
    width: 24px; height: 24px; border-radius: 50%;
    background: #fff; transition: transform 0.2s;
    box-shadow: var(--shadow-knob);
  }
  .toggle-btn.active .toggle-thumb { transform: translateX(24px); }

  /* Workspace tag picker */
  .tag-grid { display: flex; flex-wrap: wrap; gap: 0.4rem; }
  .hint { font-size: var(--fs-xs); color: var(--text-faint); text-transform: none; letter-spacing: 0; font-weight: var(--fw-normal); }
  .hint.warn { color: var(--warn-text); }
  .link-btn {
    align-self: flex-start; background: none; border: none; padding: 0; cursor: pointer;
    font-size: var(--fs-xs); color: var(--text-faint); text-decoration: underline;
  }
  .link-btn:hover { color: var(--text); }
  .seg { align-self: flex-start; }

  .advanced { border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2); }
  summary { padding: 0.7rem 1rem; cursor: pointer; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); user-select: none; }
  summary:hover { color: var(--text); }
  .advanced-body { padding: var(--sp-2) 1rem 1rem; display: flex; flex-direction: column; gap: var(--sp-3); }
  .form-actions { display: flex; gap: 10px; justify-content: flex-end; margin-top: var(--sp-2); }
  .form-actions .btn { height: 42px; border-radius: var(--radius-field); padding: 0 20px; }
</style>

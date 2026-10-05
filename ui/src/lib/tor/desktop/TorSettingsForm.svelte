<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The settings sections of the Tor page: bridges, the proxy for Tor itself,
  network, nodes, launch, the port for other programs and the extra torrc.
  One form with one Save button: a save restarts the idle instances at once,
  so it is not done at every keystroke, and some fields only make sense together.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { api } from '$lib/tor/api';
  import {
    formFromSettings, isChanged, settingsFromForm, torErrorText,
    type FormField, type FormProblem, type TorForm,
  } from '$lib/tor/settings';
  import type { BridgeMode } from '$lib/tor/types';

  let stored = $state<import('$lib/tor/types').TorSettings | null>(null);
  let form = $state<TorForm | null>(null);
  let builtin = $state<string[]>([]);
  let problems = $state<Partial<Record<FormField, FormProblem>>>({});
  let saving = $state(false);
  let error = $state('');
  let saved = $state(false);
  let loadFailed = $state(false);

  const changed = $derived(form !== null && stored !== null && isChanged(form, stored));
  const BRIDGE_MODES: { id: BridgeMode; label: 'tor_bridges_none' | 'tor_bridges_builtin' | 'tor_bridges_custom' }[] = [
    { id: 'none', label: 'tor_bridges_none' },
    { id: 'builtin', label: 'tor_bridges_builtin' },
    { id: 'custom', label: 'tor_bridges_custom' },
  ];

  onMount(async () => {
    try {
      stored = await api.tor.getSettings();
      form = formFromSettings(stored);
      builtin = await api.tor.builtinBridges().catch(() => []);
    } catch {
      loadFailed = true;
    }
  });

  function problemText(field: FormField): string {
    const p = problems[field];
    return p ? $t(p.key, p.vars) : '';
  }

  function touched() {
    saved = false;
  }

  async function save() {
    if (!form) return;
    error = '';
    saved = false;
    const result = settingsFromForm(form);
    if (!result.ok) {
      problems = result.problems;
      return;
    }
    problems = {};
    saving = true;
    try {
      stored = await api.tor.setSettings(result.settings);
      form = formFromSettings(stored);
      saved = true;
    } catch (e) {
      error = torErrorText(formatError(e), (k) => $t(k));
    } finally {
      saving = false;
    }
  }

  function reset() {
    if (!stored) return;
    form = formFromSettings(stored);
    problems = {};
    error = '';
    saved = false;
  }
</script>

{#if loadFailed}
  <div class="card"><div class="error-msg">{$t('tor_settings_load_failed')}</div></div>
{:else if form}
  <form class="tor-settings" oninput={touched} onchange={touched} onsubmit={(e) => { e.preventDefault(); save(); }}>
    <p class="muted small scope">{$t('tor_settings_scope')}</p>

    <!-- Bridges -->
    <div class="card" id="bridges">
      <div class="card-title">{$t('tor_section_bridges')}</div>
      <p class="muted small">{$t('tor_bridges_hint')}</p>
      <div class="form-group">
        <label for="tor-bridge-mode">{$t('tor_bridges_mode')}</label>
        <select id="tor-bridge-mode" bind:value={form.bridgeMode}>
          {#each BRIDGE_MODES as m (m.id)}
            <option value={m.id}>{$t(m.label)}</option>
          {/each}
        </select>
      </div>
      {#if form.bridgeMode === 'builtin'}
        <div class="form-group">
          <label for="tor-bridge-kind">{$t('tor_bridges_kind')}</label>
          {#if builtin.length}
            <select id="tor-bridge-kind" bind:value={form.bridgeBuiltin}>
              {#if !builtin.includes(form.bridgeBuiltin)}
                <option value={form.bridgeBuiltin}>{form.bridgeBuiltin}</option>
              {/if}
              {#each builtin as kind (kind)}
                <option value={kind}>{kind}</option>
              {/each}
            </select>
          {:else}
            <p class="warn-msg">{$t('tor_bridges_no_builtin')}</p>
          {/if}
        </div>
      {:else if form.bridgeMode === 'custom'}
        <div class="form-group">
          <label for="tor-bridge-lines">{$t('tor_bridges_lines')}</label>
          <textarea id="tor-bridge-lines" class="mono" rows="4" bind:value={form.bridgeLines} placeholder={'obfs4 192.0.2.1:443 FINGERPRINT cert=… iat-mode=0'} spellcheck="false"></textarea>
          <span class="muted small">{$t('tor_bridges_lines_hint')}</span>
          {#if problems.bridgeLines}<span class="error-msg">{problemText('bridgeLines')}</span>{/if}
        </div>
      {/if}
    </div>

    <!-- Proxy for Tor itself -->
    <div class="card" id="upstream">
      <div class="card-title">{$t('tor_section_upstream')}</div>
      <p class="muted small">{$t('tor_upstream_hint')}</p>
      <div class="form-group">
        <label for="tor-upstream-kind">{$t('tor_upstream_kind')}</label>
        <select id="tor-upstream-kind" bind:value={form.upstreamKind}>
          <option value="off">{$t('tor_upstream_off')}</option>
          <option value="socks5">SOCKS5</option>
          <option value="https">HTTPS</option>
        </select>
      </div>
      {#if form.upstreamKind !== 'off'}
        <div class="form-row">
          <div class="form-group">
            <label for="tor-upstream-host">{$t('tor_upstream_host')}</label>
            <input id="tor-upstream-host" type="text" bind:value={form.upstreamHost} autocomplete="off" spellcheck="false" />
            {#if problems.upstreamHost}<span class="error-msg">{problemText('upstreamHost')}</span>{/if}
          </div>
          <div class="form-group">
            <label for="tor-upstream-port">{$t('tor_upstream_port')}</label>
            <input id="tor-upstream-port" type="text" inputmode="numeric" bind:value={form.upstreamPort} autocomplete="off" />
            {#if problems.upstreamPort}<span class="error-msg">{problemText('upstreamPort')}</span>{/if}
          </div>
        </div>
        <div class="form-row">
          <div class="form-group">
            <label for="tor-upstream-user">{$t('tor_upstream_user')}</label>
            <input id="tor-upstream-user" type="text" bind:value={form.upstreamUser} autocomplete="off" spellcheck="false" />
          </div>
          <div class="form-group">
            <label for="tor-upstream-password">{$t('tor_upstream_password')}</label>
            <input id="tor-upstream-password" type="password" bind:value={form.upstreamPassword} autocomplete="off" />
          </div>
        </div>
        <p class="muted small">{$t('tor_upstream_password_hint')}</p>
      {/if}
    </div>

    <!-- Network -->
    <div class="card" id="network">
      <div class="card-title">{$t('tor_section_network')}</div>
      <div class="form-group">
        <label for="tor-ports">{$t('tor_ports_label')}</label>
        <input id="tor-ports" type="text" bind:value={form.ports} placeholder="80, 443" autocomplete="off" spellcheck="false" />
        <span class="muted small">{$t('tor_ports_hint')}</span>
        {#if problems.ports}<span class="error-msg">{problemText('ports')}</span>{/if}
      </div>
    </div>

    <!-- Nodes -->
    <div class="card" id="nodes">
      <div class="card-title">{$t('tor_section_nodes')}</div>
      <div class="form-group">
        <label for="tor-exclude">{$t('tor_exclude_label')}</label>
        <input id="tor-exclude" type="text" bind:value={form.exclude} placeholder={$t('tor_countries_placeholder')} autocomplete="off" spellcheck="false" />
        <span class="muted small">{$t('tor_exclude_hint')}</span>
        {#if problems.exclude}<span class="error-msg">{problemText('exclude')}</span>{/if}
      </div>
      <div class="toggle-row">
        <div class="toggle-info">
          <span>{$t('tor_strict_label')}</span>
          <span class="muted">{$t('tor_strict_hint')}</span>
        </div>
        <button
          type="button"
          class="toggle"
          class:on={form.strict}
          onclick={() => { if (form) { form.strict = !form.strict; touched(); } }}
          aria-pressed={form.strict}
          aria-label={$t('tor_strict_label')}
        ></button>
      </div>
    </div>

    <!-- Launch -->
    <div class="card" id="launch">
      <div class="card-title">{$t('tor_section_launch')}</div>
      <div class="toggle-row">
        <div class="toggle-info">
          <span>{$t('tor_start_with_app')}</span>
          <span class="muted">{$t('tor_start_with_app_hint')}</span>
        </div>
        <button
          type="button"
          class="toggle"
          class:on={form.startWithApp}
          onclick={() => { if (form) { form.startWithApp = !form.startWithApp; touched(); } }}
          aria-pressed={form.startWithApp}
          aria-label={$t('tor_start_with_app')}
        ></button>
      </div>
      <div class="form-group">
        <label for="tor-idle">{$t('tor_idle_label')}</label>
        <input id="tor-idle" class="narrow" type="text" inputmode="numeric" bind:value={form.idleMinutes} autocomplete="off" />
        <span class="muted small">{$t('tor_idle_hint')}</span>
        {#if problems.idleMinutes}<span class="error-msg">{problemText('idleMinutes')}</span>{/if}
      </div>
    </div>

    <!-- Port for other programs -->
    <div class="card" id="external">
      <div class="card-title">{$t('tor_section_external')}</div>
      <div class="toggle-row">
        <div class="toggle-info">
          <span>{$t('tor_external_label')}</span>
          <span class="muted">{$t('tor_external_hint')}</span>
        </div>
        <button
          type="button"
          class="toggle"
          class:on={form.externalOn}
          onclick={() => { if (form) { form.externalOn = !form.externalOn; touched(); } }}
          aria-pressed={form.externalOn}
          aria-label={$t('tor_external_label')}
        ></button>
      </div>
      {#if form.externalOn}
        <div class="form-group">
          <label for="tor-external-port">{$t('tor_external_port')}</label>
          <div class="addr-row">
            <span class="mono">127.0.0.1:</span>
            <input id="tor-external-port" class="narrow" type="text" inputmode="numeric" bind:value={form.externalPort} autocomplete="off" />
          </div>
          {#if problems.externalPort}<span class="error-msg">{problemText('externalPort')}</span>{/if}
        </div>
      {/if}
    </div>

    <!-- Advanced -->
    <div class="card" id="advanced">
      <div class="card-title">{$t('tor_section_advanced')}</div>
      <p class="warn-msg">{$t('tor_torrc_warning')}</p>
      <div class="form-group">
        <label for="tor-torrc">{$t('tor_torrc_label')}</label>
        <textarea id="tor-torrc" class="mono" rows="4" bind:value={form.extraTorrc} placeholder="AvoidDiskWrites 1" spellcheck="false"></textarea>
      </div>
    </div>

    <div class="save-bar">
      <button class="btn btn-primary btn-sm" type="submit" disabled={saving || !changed}>
        {saving ? $t('tor_btn_saving') : $t('tor_btn_save')}
      </button>
      <button class="btn btn-ghost btn-sm" type="button" disabled={saving || !changed} onclick={reset}>{$t('tor_btn_revert')}</button>
      {#if saved}<span class="ok-msg">{$t('tor_saved')}</span>{/if}
      {#if Object.keys(problems).length}<span class="error-msg">{$t('tor_fix_fields')}</span>{/if}
    </div>
    {#if error}<div class="error-msg">{error}</div>{/if}
  </form>
{/if}

<style>
  .tor-settings { display: flex; flex-direction: column; gap: var(--sp-4); }
  .card { display: flex; flex-direction: column; gap: var(--sp-4); padding: var(--sp-6); }
  .card-title {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-dim);
    text-transform: uppercase; letter-spacing: 1px;
  }
  .small { font-size: var(--fs-sm); }
  .mono { font-family: var(--font-mono); }
  .ok-msg { font-size: var(--fs-sm); color: var(--success-text); }
  .warn-msg { font-size: var(--fs-sm); color: var(--warn-text); }
  .error-msg { font-size: var(--fs-sm); color: var(--danger-text); }
  .toggle-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-4); }
  .toggle-info { display: flex; flex-direction: column; gap: 0.2rem; font-size: var(--fs-sm); color: var(--text); }
  .narrow { max-width: 9rem; }
  .addr-row { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); }
  textarea { resize: vertical; }
  .save-bar { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
</style>

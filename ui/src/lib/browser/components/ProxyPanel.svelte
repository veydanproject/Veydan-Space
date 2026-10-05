<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '$lib/browser/api';
  import { t } from '$lib/core/i18n';
  import type { CreateProxyRequest, Proxy } from '$lib/browser/types';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import { registry } from '$lib/core/registry';
  import { explainError } from '$lib/browser/tor-error';

  interface Props {
    proxy?: Proxy | null;
    workspaceId?: string | null;
    onclose: () => void;
    onsaved: (proxy: Proxy) => void;
  }

  let { proxy = null, workspaceId = null, onclose, onsaved }: Props = $props();

  const isEdit = $derived(proxy !== null);

  let saving = $state(false);
  let error = $state('');

  // SSH TOFU: shown when a new (unknown) server fingerprint is received
  let pendingFingerprint = $state<{ fingerprint: string; ip: string } | null>(null);
  let fingerprintError = $state('');

  let form = $state<CreateProxyRequest>(untrack(() => ({
    name: proxy?.name ?? '',
    proxy_type: proxy?.proxy_type ?? 'socks5',
    host: proxy?.host ?? '',
    port: proxy?.port ?? 1080,
    username: proxy?.username ?? null,
    // Secrets are not sent to the UI: null = keep stored, '' = clear.
    password: null,
    country: proxy?.country ?? null,
    city: proxy?.city ?? null,
    private_key: null,
    tags: proxy ? proxy.tags : (workspaceId ? [`workspace:${workspaceId}`] : []),
  })));

  // A Tor row names no server: the backend stores this placeholder and no credentials.
  const TOR_HOST = '127.0.0.1';
  const TOR_PORT = 9050;
  const isTorType = $derived(form.proxy_type === 'tor');

  /** Exit countries: empty (any country) or two ASCII letters each, as the backend checks them. */
  const countriesValid = $derived(
    !(form.country ?? '').trim() || (form.country ?? '').split(',').every((c) => /^[A-Za-z]{2}$/.test(c.trim())),
  );

  function setType(next: string) {
    const prev = form.proxy_type;
    if (next === prev) return;
    form.proxy_type = next;
    if (next === 'tor') {
      form.host = TOR_HOST;
      form.port = TOR_PORT;
      form.username = null;
      form.password = null;
      form.private_key = null;
      form.country = null;
      form.city = null;
    } else if (prev === 'tor') {
      form.host = '';
      form.port = next === 'ssh' ? 22 : 1080;
      form.country = null;
    } else {
      if (next === 'ssh' && (form.port === 1080 || form.port === 8080)) form.port = 22;
      if (next !== 'ssh' && form.port === 22) form.port = 1080;
    }
  }

  async function submit() {
    if (!form.name.trim() || (!isTorType && !form.host.trim())) {
      error = $t('proxy_error_fields');
      return;
    }
    if (isTorType && !countriesValid) {
      error = $t('proxy_tor_countries_invalid');
      return;
    }
    const body: CreateProxyRequest = isTorType
      ? {
          ...form,
          host: TOR_HOST,
          port: TOR_PORT,
          username: null,
          // '' clears a secret the row had before it became a Tor row.
          password: proxy?.has_password ? '' : null,
          private_key: proxy?.has_private_key ? '' : null,
          country: (form.country ?? '').trim() || null,
          city: null,
        }
      : form;
    saving = true;
    error = '';
    try {
      const result = isEdit
        ? await api.proxies.update(proxy!.id, body)
        : await api.proxies.create(body);
      onsaved(result);
    } catch (e) {
      error = explainError(e, $t);
    } finally {
      saving = false;
    }
  }
</script>

<Drawer open title={isEdit ? $t('proxies_form_edit') : $t('proxies_form_new')} {onclose}>
      <form onsubmit={(e) => { e.preventDefault(); submit(); }}>
        {#if error}
          <div class="error-msg" style="margin-bottom:0.75rem">{error}</div>
        {/if}

        <div class="section">
          <div class="section-label">{$t('proxy_field_type')}</div>
          <div class="type-row">
            {#each ['http', 'https', 'socks5', 'ssh', 'tor'] as t_}
              <button
                type="button"
                class="type-btn"
                class:active={form.proxy_type === t_}
                onclick={() => setType(t_)}
              >{t_ === 'tor' ? 'Tor' : t_.toUpperCase()}</button>
            {/each}
          </div>
        </div>

        <div class="divider"></div>

        <div class="section">
          <div class="section-label">{$t('proxy_field_name')} *</div>
          <div class="form-group">
            <!-- svelte-ignore a11y_autofocus -->
            <input
              id="pp-name"
              type="text"
              bind:value={form.name}
              placeholder={$t('proxy_field_name_placeholder')}
              autofocus
            />
          </div>

          {#if !isTorType}
          <div class="form-row">
            <div class="form-group host-group">
              <label for="pp-host">{$t('proxy_field_host')} *</label>
              <input id="pp-host" type="text" bind:value={form.host} placeholder={$t('proxy_field_host_placeholder')} />
            </div>
            <div class="form-group port-group">
              <label for="pp-port">{$t('proxy_field_port')}</label>
              <input id="pp-port" type="number" bind:value={form.port} min="1" max="65535" />
            </div>
          </div>
          {/if}
        </div>

        {#if isTorType}
        <div class="divider"></div>
        <div class="section">
          <div class="section-label">{$t('proxy_tor_countries')}</div>
          <div class="form-group">
            <input
              id="pp-tor-countries"
              type="text"
              bind:value={form.country}
              placeholder={$t('proxy_tor_countries_placeholder')}
              aria-invalid={!countriesValid}
              autocomplete="off"
              spellcheck="false"
            />
          </div>
          {#if countriesValid}
            <div class="field-hint">{$t('proxy_tor_countries_hint')}</div>
          {:else}
            <div class="field-hint field-hint--error">{$t('proxy_tor_countries_invalid')}</div>
          {/if}
          <div class="field-hint">{$t('proxy_tor_about')}</div>
          <div class="field-hint">{$t('proxy_tor_note')}</div>
        </div>
        {:else}
        <div class="divider"></div>

        <div class="section">
          <div class="section-label">{$t('proxy_field_username')} / {$t('proxy_field_password')}</div>
          <div class="form-group">
            <input id="pp-user" type="text" bind:value={form.username} placeholder={$t('proxy_field_username')} />
          </div>
          <div class="form-group">
            <input
              id="pp-pass"
              type="password"
              bind:value={form.password}
              placeholder={proxy?.has_password && form.password === null ? $t('secret_stored_placeholder') : $t('proxy_field_password')}
            />
            {#if proxy?.has_password && form.password === null}
              <button type="button" class="link-btn" onclick={() => (form.password = '')}>{$t('secret_clear')}</button>
            {/if}
          </div>
        </div>

        {#if form.proxy_type === 'ssh'}
        <div class="divider"></div>
        <div class="section">
          <div class="section-label">{$t('proxy_section_ssh_key')}</div>
          <div class="form-group">
            <textarea
              id="pp-pkey"
              class="mono"
              rows="5"
              bind:value={form.private_key}
              placeholder={proxy?.has_private_key && form.private_key === null ? $t('secret_stored_placeholder') : '-----BEGIN OPENSSH PRIVATE KEY-----\n...\n-----END OPENSSH PRIVATE KEY-----'}
              style="font-size: 0.72rem; resize: vertical;"
            ></textarea>
            {#if proxy?.has_private_key && form.private_key === null}
              <button type="button" class="link-btn" onclick={() => (form.private_key = '')}>{$t('secret_clear')}</button>
            {/if}
          </div>
          <div class="field-hint">{$t('proxy_ssh_key_hint')}</div>
        </div>
        {/if}

        <div class="divider"></div>

        <div class="section">
          <div class="section-label">{$t('proxy_section_geo')}</div>
          <div class="form-row">
            <div class="form-group">
              <label for="pp-country">{$t('proxy_col_country')}</label>
              <input id="pp-country" type="text" bind:value={form.country} placeholder="US" />
            </div>
            <div class="form-group">
              <label for="pp-city">{$t('proxy_field_city')}</label>
              <input id="pp-city" type="text" bind:value={form.city} placeholder={$t('proxy_city_placeholder')} />
            </div>
          </div>
        </div>
        {/if}

        <div class="form-actions">
          <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('proxy_btn_cancel')}</button>
          <button type="submit" class="btn btn-primary" disabled={saving}>
            {saving ? '…' : isEdit ? $t('proxy_btn_save') : $t('proxy_btn_add')}
          </button>
        </div>
      </form>
      {#if proxy}
        <!-- What other modules show about a proxy: the notes about it. -->
        {#each registry.views('proxy', 'inline') as view (view.id)}
          <view.component id={proxy.id} name={proxy.name} />
        {/each}
      {/if}
</Drawer>

<style>
  .section { display: flex; flex-direction: column; gap: var(--sp-2); margin-bottom: var(--sp-1); }

  .section-label {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-dim);
    text-transform: uppercase; letter-spacing: 0.9px;
  }

  /* Proxy type — standalone segment buttons (46px / radius 11) */
  .type-row { display: flex; gap: var(--sp-2); }

  .type-btn {
    flex: 1; height: var(--control-h-lg); padding: 0 0.5rem;
    font-size: var(--fs-sm); font-weight: var(--fw-semibold);
    background: var(--surface-3); border: 1px solid var(--border);
    color: var(--text-2); border-radius: var(--radius-field); cursor: pointer;
    transition: all var(--dur-fast);
  }
  .type-btn:hover:not(.active) { border-color: var(--border-2); color: var(--text); }
  .type-btn.active { background: var(--accent-bg); border-color: var(--accent-border); color: var(--accent-text); }

  .divider { height: 1px; background: var(--border); margin: var(--sp-3) 0; }

  .field-hint {
    font-size: var(--fs-2xs); color: var(--text-faint); margin-top: -0.25rem;
  }
  .field-hint--error { color: var(--danger-text); }
  .link-btn {
    align-self: flex-start; background: none; border: none; padding: 0; margin-top: 0.25rem; cursor: pointer;
    font-size: var(--fs-2xs); color: var(--text-faint); text-decoration: underline;
  }
  .link-btn:hover { color: var(--text); }

  /* Tall drawer form fields per design (46px / radius 11) */
  form input[type='text'],
  form input[type='number'],
  form input[type='password'] {
    height: var(--control-h-lg);
    border-radius: var(--radius-field);
    background: var(--surface-3);
    font-size: 0.9rem;
  }

  textarea {
    width: 100%; background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius-field); color: var(--text);
    padding: 0.6rem 0.75rem; box-sizing: border-box;
  }
  textarea:focus { outline: none; border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); }

  .form-row { display: grid; grid-template-columns: 1fr auto; gap: var(--sp-2); }
  .port-group { width: 100px; }
  .host-group { flex: 1; }

  .form-actions {
    display: flex; gap: 10px; justify-content: flex-end;
    padding-top: var(--sp-3);
  }
  .form-actions .btn { height: 42px; border-radius: var(--radius-field); padding: 0 20px; }
</style>

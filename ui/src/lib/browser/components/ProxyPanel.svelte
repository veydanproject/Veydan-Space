<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '$lib/browser/api';
  import { t } from '$lib/core/i18n';
  import type { CreateProxyRequest, Proxy } from '$lib/browser/types';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
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
  // A check of the fields shows under them (the countries field shows its own); a refusal of the save is the form's plate.
  const fieldsErr = $derived(!!error && error === $t('proxy_error_fields'));
  const countriesErr = $derived(!!error && error === $t('proxy_tor_countries_invalid'));

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

<Drawer open title={isEdit ? $t('proxies_form_edit') : $t('proxies_form_new')} width="var(--drawer-w-md)" {onclose}>
      <form id="px-form" onsubmit={(e) => { e.preventDefault(); submit(); }}>
        {#if error && !fieldsErr && !countriesErr}
          <div style="margin-bottom:0.75rem"><Notice tone="danger">{error}</Notice></div>
        {/if}

        <!-- Every caption of the form is the one field caption (`.form-group label`); the type is a segment. -->
        <div class="section form-group">
          <!-- svelte-ignore a11y_label_has_associated_control -->
          <label id="pp-type-label">{$t('proxy_field_type')}</label>
          <div class="seg seg-fill" role="group" aria-labelledby="pp-type-label">
            {#each ['http', 'https', 'socks5', 'ssh', 'tor'] as t_}
              <button
                type="button"
                class="seg-btn"
                class:active={form.proxy_type === t_}
                aria-pressed={form.proxy_type === t_}
                onclick={() => setType(t_)}
              >{t_ === 'tor' ? 'Tor' : t_.toUpperCase()}</button>
            {/each}
          </div>
        </div>

        <div class="divider"></div>

        <div class="section form-group">
          <label for="pp-name">{$t('proxy_field_name')} *</label>
          <div class="form-group">
            <!-- svelte-ignore a11y_autofocus -->
            <input
              id="pp-name"
              type="text"
              bind:value={form.name}
              placeholder={$t('proxy_field_name_placeholder')}
              aria-invalid={fieldsErr && !form.name.trim()}
              autofocus
            />
          </div>

          {#if !isTorType}
          <div class="form-row">
            <div class="form-group host-group">
              <label for="pp-host">{$t('proxy_field_host')} *</label>
              <input id="pp-host" type="text" bind:value={form.host} placeholder={$t('proxy_field_host_placeholder')} aria-invalid={fieldsErr && !form.host.trim()} />
            </div>
            <div class="form-group port-group">
              <label for="pp-port">{$t('proxy_field_port')}</label>
              <input id="pp-port" type="number" bind:value={form.port} min="1" max="65535" />
            </div>
          </div>
          {/if}
          {#if fieldsErr}<div class="error-msg">{error}</div>{/if}
        </div>

        {#if isTorType}
        <div class="divider"></div>
        <div class="section form-group">
          <label for="pp-tor-countries">{$t('proxy_tor_countries')}</label>
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

        <div class="section form-group">
          <label for="pp-user">{$t('proxy_field_username')} / {$t('proxy_field_password')}</label>
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
        <div class="section form-group">
          <label for="pp-pkey">{$t('proxy_section_ssh_key')}</label>
          <div class="form-group">
            <textarea
              id="pp-pkey"
              class="mono"
              rows="5"
              bind:value={form.private_key}
              placeholder={proxy?.has_private_key && form.private_key === null ? $t('secret_stored_placeholder') : '-----BEGIN OPENSSH PRIVATE KEY-----\n...\n-----END OPENSSH PRIVATE KEY-----'}
              style="font-size: var(--fs-xs);"
            ></textarea>
            {#if proxy?.has_private_key && form.private_key === null}
              <button type="button" class="link-btn" onclick={() => (form.private_key = '')}>{$t('secret_clear')}</button>
            {/if}
          </div>
          <div class="field-hint">{$t('proxy_ssh_key_hint')}</div>
        </div>
        {/if}

        <div class="divider"></div>

        <div class="section form-group">
          <!-- svelte-ignore a11y_label_has_associated_control -->
          <label>{$t('proxy_section_geo')}</label>
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
      </form>
      {#if proxy}
        <!-- What other modules show about a proxy: the notes about it. -->
        {#each registry.views('proxy', 'inline') as view (view.id)}
          <view.component id={proxy.id} name={proxy.name} />
        {/each}
      {/if}
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('common_cancel')}</button>
    <button type="submit" form="px-form" class="btn btn-primary" disabled={saving}>
      {saving ? '…' : isEdit ? $t('common_save') : $t('proxy_btn_add')}
    </button>
  {/snippet}
</Drawer>

<style>
  .section { display: flex; flex-direction: column; gap: var(--sp-2); margin-bottom: var(--sp-1); }

  .divider { height: 1px; background: var(--border); margin: var(--sp-3) 0; }

  .field-hint {
    font-size: var(--fs-xs); color: var(--text-faint); margin-top: -0.25rem;
  }
  .field-hint--error { color: var(--danger-text); }
  .link-btn {
    align-self: flex-start; background: none; border: none; padding: 0; margin-top: 0.25rem; cursor: pointer;
    font-size: var(--fs-xs); color: var(--text-faint); text-decoration: underline;
  }
  .link-btn:hover { color: var(--text); }

  /* The fields: the common ones on the panel's glass, 40 high (--control-h). */
  form input[type='text'],
  form input[type='number'],
  form input[type='password'] { height: var(--control-h); }

  textarea { resize: vertical; }

  .form-row { display: grid; grid-template-columns: 1fr auto; gap: var(--sp-2); }
  .port-group { width: 100px; }
  .host-group { flex: 1; }
</style>

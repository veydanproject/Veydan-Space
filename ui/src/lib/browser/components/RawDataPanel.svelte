<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api } from '$lib/browser/api';
  import type { Profile, ProfileRawData, CookieEntry } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import { formatError, formatEpochDate } from '$lib/core/utils';
  import { locale, t } from '$lib/core/i18n';

  interface Props {
    profile: Profile;
    onclose: () => void;
  }

  let { profile, onclose }: Props = $props();

  type Tab = 'fingerprint' | 'config' | 'cookies';
  let tab = $state<Tab>('fingerprint');
  let data = $state<ProfileRawData | null>(null);
  let loading = $state(true);
  let error = $state('');
  let exportingCookies = $state(false);
  let cookieExportMsg = $state('');
  let cookieExportFailed = $state(false);

  $effect(() => {
    loading = true;
    error = '';
    api.profiles.rawData(profile.id)
      .then((d) => { data = d; })
      .catch((e) => { error = formatError(e); })
      .finally(() => { loading = false; });
  });

  function formatExpiry(ts: number | null): string {
    if (ts === null || ts === 0) return $t('raw_cookie_session');
    return formatEpochDate(ts, $locale);
  }

  function copyToClipboard(text: string) {
    navigator.clipboard.writeText(text).catch(() => {});
  }

  async function exportCookies() {
    exportingCookies = true;
    cookieExportMsg = '';
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const safeName = profile.name.replace(/[^a-z0-9_-]/gi, '_');
      const path = await save({
        defaultPath: `${safeName}_cookies.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!path) return;
      await api.profiles.exportCookiesToFile(profile.id, path);
      cookieExportMsg = $t('raw_cookies_saved', { path });
      cookieExportFailed = false;
    } catch (e) {
      cookieExportMsg = formatError(e);
      cookieExportFailed = true;
    } finally {
      exportingCookies = false;
    }
  }

  const webrtcLabels = $derived<Record<string, string>>({
    disable: $t('raw_webrtc_disable'),
    real_ip: $t('raw_webrtc_real_ip'),
    proxy_ip: $t('raw_webrtc_proxy_ip'),
  });
</script>

<Drawer open title={$t('raw_title', { name: profile.name })} width="var(--drawer-w-lg)" {onclose}>
  {#snippet subheader()}
    <div class="tab-bar">
      <button class="tab" class:active={tab === 'fingerprint'} onclick={() => (tab = 'fingerprint')}>
        {$t('raw_tab_fingerprint')}
      </button>
      <button class="tab" class:active={tab === 'config'} onclick={() => (tab = 'config')}>
        {$t('raw_tab_config')}
      </button>
      <button class="tab" class:active={tab === 'cookies'} onclick={() => (tab = 'cookies')}>
        {$t('raw_tab_cookies')}
        {#if data}
          <span class="tab-count">{data.cookies.length}</span>
        {/if}
      </button>
    </div>
  {/snippet}
  <div class="rd-body">
      {#if loading}
        <div class="centered">{$t('loading')}</div>
      {:else if error}
        <div class="error-msg">{error}</div>
      {:else if data}

        {#if tab === 'fingerprint'}
          <div class="section">
            <div class="section-title">Navigator</div>
            <div class="rows">
              <div class="row"><span class="label">userAgent</span><span class="value ua">{data.user_agent}</span></div>
              <div class="row"><span class="label">platform</span><span class="value">{data.platform}</span></div>
              <div class="row"><span class="label">language</span><span class="value">{data.locale}</span></div>
              <div class="row"><span class="label">languages</span><span class="value">{data.languages}</span></div>
            </div>
          </div>

          <div class="section">
            <div class="section-title">{$t('raw_section_screen')}</div>
            <div class="rows">
              <div class="row"><span class="label">width</span><span class="value">{data.screen_width}</span></div>
              <div class="row"><span class="label">height</span><span class="value">{data.screen_height}</span></div>
            </div>
          </div>

          <div class="section">
            <div class="section-title">WebGL</div>
            <div class="rows">
              <div class="row">
                <span class="label">vendor</span>
                <span class="value" class:muted={!data.webgl_vendor}>{data.webgl_vendor ?? $t('raw_auto_by_os')}</span>
              </div>
              <div class="row">
                <span class="label">renderer</span>
                <span class="value" class:muted={!data.webgl_renderer}>{data.webgl_renderer ?? $t('raw_auto_by_os')}</span>
              </div>
            </div>
          </div>

          <div class="section">
            <div class="section-title">{$t('raw_section_geo')}</div>
            <div class="rows">
              <div class="row">
                <span class="label">timezone</span>
                <span class="value" class:muted={!data.timezone}>{data.timezone ?? $t('raw_system')}</span>
              </div>
              {#if data.geolocation_enabled}
                <div class="row"><span class="label">latitude</span><span class="value">{data.latitude ?? '—'}</span></div>
                <div class="row"><span class="label">longitude</span><span class="value">{data.longitude ?? '—'}</span></div>
              {:else}
                <div class="row"><span class="label">geolocation</span><span class="value muted">{$t('raw_disabled')}</span></div>
              {/if}
            </div>
          </div>

          <div class="section">
            <div class="section-title">{$t('raw_section_privacy')}</div>
            <div class="rows">
              <div class="row">
                <span class="label">WebRTC</span>
                <span class="value">{webrtcLabels[data.webrtc_mode] ?? data.webrtc_mode}</span>
              </div>
            </div>
          </div>

          <div class="section">
            <div class="section-title">{$t('raw_section_seeds')}</div>
            <div class="rows">
              <div class="row"><span class="label">canvas</span><span class="value mono">{data.canvas_seed}</span></div>
              <div class="row"><span class="label">audio</span><span class="value mono">{data.audio_seed}</span></div>
              <div class="row"><span class="label">fonts</span><span class="value mono">{data.fonts_seed}</span></div>
            </div>
          </div>

        {:else if tab === 'config'}
          <div class="code-block-wrap">
            <div class="code-block-header">
              <span>CAMOU_CONFIG_1</span>
              <button class="copy-btn" onclick={() => copyToClipboard(data!.camoufox_config)}>
                <Icon name="copy" size={12} />{$t('raw_copy')}
              </button>
            </div>
            <pre class="code-block">{data.camoufox_config}</pre>
          </div>

          <div class="code-block-wrap">
            <div class="code-block-header">
              <span>user.js</span>
              <button class="copy-btn" onclick={() => copyToClipboard(data!.user_js)}>
                <Icon name="copy" size={12} />{$t('raw_copy')}
              </button>
            </div>
            <pre class="code-block">{data.user_js}</pre>
          </div>

        {:else if tab === 'cookies'}
          <div class="cookies-toolbar">
            <span class="cookies-count">{$t('raw_cookies_count', { n: String(data.cookies.length) })}</span>
            <button
              class="btn btn-ghost btn-sm"
              disabled={exportingCookies || data.cookies.length === 0}
              onclick={exportCookies}
            >
              <Icon name="download" size={12} />{exportingCookies ? '…' : $t('panel_btn_export_cookies')}
            </button>
          </div>
          {#if cookieExportMsg}
            <div class="cookie-export-msg" class:error={cookieExportFailed}>{cookieExportMsg}</div>
          {/if}
          {#if data.cookies.length === 0}
            <div class="empty-state">
              <Icon name="cookie" size={28} />
              <span>{$t('raw_cookies_empty')}</span>
              <span class="empty-hint">{$t('raw_cookies_empty_hint')}</span>
            </div>
          {:else}
            <div class="cookies-table-wrap">
              <table class="cookies-table">
                <thead>
                  <tr>
                    <th>{$t('raw_col_host')}</th>
                    <th>{$t('raw_col_name')}</th>
                    <th>{$t('raw_col_value')}</th>
                    <th>{$t('raw_col_expiry')}</th>
                    <th>{$t('raw_col_flags')}</th>
                  </tr>
                </thead>
                <tbody>
                  {#each data.cookies as cookie, i (i)}
                    <tr>
                      <td class="host-cell">{cookie.host}</td>
                      <td class="name-cell">{cookie.name}</td>
                      <td class="value-cell" title={cookie.value}>{cookie.value.length > 40 ? cookie.value.slice(0, 40) + '…' : cookie.value}</td>
                      <td class="expiry-cell">{formatExpiry(cookie.expiry)}</td>
                      <td class="flags-cell">
                        {#if cookie.secure}<span class="flag s">S</span>{/if}
                        {#if cookie.http_only}<span class="flag h">H</span>{/if}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {/if}

      {/if}
  </div>
</Drawer>

<style>

  /* .tab-bar / .tab / .tab-count styling is global (base.css); only local deltas kept below */
  .tab-bar { padding-top: 0.3rem; }
  .tab { justify-content: center; }

  .tab-count {
    font-weight: 600;
    border: 1px solid var(--border);
    line-height: 1.4;
  }
  .tab.active .tab-count {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .rd-body {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  .centered { text-align: center; color: var(--text-2); padding: var(--sp-12); }

  /* Fingerprint rows */
  .section { display: flex; flex-direction: column; gap: 0.3rem; }
  .section-title {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); text-transform: uppercase;
    letter-spacing: 0.7px; color: var(--text-dim); margin-bottom: 0.2rem;
  }
  .rows { display: flex; flex-direction: column; gap: var(--sp-1); }
  .row {
    display: flex; gap: var(--sp-2); font-size: var(--fs-base);
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: var(--radius-sm); padding: 0.3rem 0.6rem;
    align-items: flex-start;
  }
  .label { color: var(--text-3); min-width: 90px; flex-shrink: 0; font-size: var(--fs-sm); padding-top: 1px; }
  .value { color: var(--text); word-break: break-all; flex: 1; }
  .value.ua { font-size: var(--fs-xs); word-break: break-word; }
  .value.muted { color: var(--text-3); font-style: italic; }
  .value.mono { font-family: var(--font-mono); }

  /* Code blocks — mono on --surface-3, nested-card radius */
  .code-block-wrap { display: flex; flex-direction: column; gap: 0; }
  .code-block-header {
    display: flex; align-items: center; justify-content: space-between;
    background: var(--surface-2); border: 1px solid var(--border);
    border-bottom: none; border-radius: var(--radius-md) var(--radius-md) 0 0;
    padding: 0.35rem 0.6rem; font-size: var(--fs-sm); color: var(--text-2); font-weight: var(--fw-semibold);
  }
  .copy-btn {
    display: flex; align-items: center; gap: var(--sp-1);
    font-size: var(--fs-xs); color: var(--text-2); background: none; border: none;
    cursor: pointer; padding: 0.1rem 0.3rem; border-radius: var(--radius-sm);
    transition: all var(--dur-fast);
  }
  .copy-btn:hover { color: var(--accent-text-3); background: var(--surface-3); }

  .code-block {
    background: var(--surface-3); border: 1px solid var(--border);
    border-radius: 0 0 var(--radius-md) var(--radius-md);
    padding: var(--sp-3); font-size: var(--fs-xs); font-family: var(--font-mono);
    overflow-x: auto; white-space: pre; color: var(--text);
    max-height: 300px; overflow-y: auto; margin: 0;
  }

  /* Cookies toolbar */
  .cookies-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
  }
  .cookies-count {
    font-size: var(--fs-sm);
    color: var(--text-3);
    font-weight: 500;
  }
  .cookie-export-msg {
    font-size: var(--fs-sm);
    padding: 0.3rem 0.6rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--success-border);
    background: var(--success-bg);
    color: var(--success-text);
    word-break: break-all;
  }
  .cookie-export-msg.error {
    border-color: var(--danger-border);
    background: var(--danger-bg);
    color: var(--danger-text);
  }

  /* Cookies table */
  .cookies-table-wrap { overflow-x: auto; }
  .cookies-table {
    width: 100%; border-collapse: collapse; font-size: var(--fs-sm);
  }
  .cookies-table th {
    text-align: left; padding: 0.4rem var(--sp-2);
    border-bottom: 1px solid var(--border);
    color: var(--text-3); font-weight: var(--fw-bold); font-size: var(--fs-2xs);
    text-transform: uppercase; letter-spacing: 0.7px;
    background: var(--surface-2); white-space: nowrap;
  }
  .cookies-table td {
    padding: 0.35rem var(--sp-2);
    border-bottom: 1px solid var(--surface-2);
    color: var(--text); vertical-align: top;
  }
  .cookies-table tr:hover td { background: var(--surface-row-hover); }
  .host-cell { color: var(--text-2); white-space: nowrap; max-width: 130px; overflow: hidden; text-overflow: ellipsis; }
  .name-cell { font-weight: 500; white-space: nowrap; max-width: 100px; overflow: hidden; text-overflow: ellipsis; }
  .value-cell { color: var(--text-3); max-width: 160px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .expiry-cell { color: var(--text-2); white-space: nowrap; }
  .flags-cell { display: flex; gap: 0.2rem; }
  .flag {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); border-radius: var(--radius-xs);
    padding: 0.05rem 0.3rem;
  }
  .flag.s { background: var(--success-bg); color: var(--success-text); }
  .flag.h { background: var(--surface-2); color: var(--text-2); border: 1px solid var(--border); }

  /* Empty state */
  .empty-state {
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: var(--sp-2); padding: var(--sp-12); color: var(--text-3); text-align: center;
  }
  .empty-hint { font-size: var(--fs-sm); color: var(--text-3); max-width: 260px; }
</style>

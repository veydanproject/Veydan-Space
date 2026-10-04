<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api } from '$lib/browser/api';
  import { t } from '$lib/core/i18n';
  import type { CreateProfileRequest, PresetInfo, Proxy } from '$lib/browser/types';
  import { LOCALES } from '$lib/core/locales';
  import { GPU_PRESETS, presetToGpuOs, defaultGpuForPreset } from '$lib/browser/gpu-presets';
  import { formatError } from '$lib/core/utils';
  import { proxyOptionLabel } from '$lib/browser/proxy-label';
  import { TIMEZONES, getExpectedRegion, getTimezoneRegion } from '$lib/core/timezones';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';

  interface Props {
    workspaceId: string;
    /** All proxies (workspace-tagged shown first) */
    proxies: Proxy[];
    onclose: () => void;
    oncreated: () => void;
  }

  let { workspaceId, proxies, onclose, oncreated }: Props = $props();

  const wsTag = $derived(`workspace:${workspaceId}`);
  const proxyOptions = $derived.by(() => {
    const ws = proxies.filter(p => p.tags.includes(wsTag));
    const other = proxies.filter(p => !p.tags.includes(wsTag));
    const toOpt = (p: Proxy) => ({ label: proxyOptionLabel(p), value: p.id });
    return [
      { label: $t('profile_proxy_none'), value: null as string | null },
      ...ws.map(toOpt),
      ...(other.length > 0 && ws.length > 0 ? [{ label: '— ' + $t('proxies_other') + ' —', value: null, disabled: true }] : []),
      ...other.map(toOpt),
    ];
  });

  let presets = $state<PresetInfo[]>([]);
  let saving = $state(false);
  let error = $state('');

  let form = $state<CreateProfileRequest>(untrack(() => ({
    name: '',
    workspace_id: workspaceId,
    browser_type: 'camoufox',
    proxy_id: null,
    fingerprint_preset: 'win10',
    webrtc_mode: 'disable',
    geolocation_enabled: false,
    locale: 'en-US',
    languages: 'en-US,en',
    timezone: null,
    webgl_vendor: null,
    webgl_renderer: null,
    notes: '',
    default_search_engine: 'ddg',
    history_enabled: true,
  })));

  let gpuOptions = $derived(
    GPU_PRESETS.filter(g => g.os === presetToGpuOs(form.fingerprint_preset ?? 'win10'))
  );

  let warnings = $derived(() => {
    const result: string[] = [];
    const gpuOs = presetToGpuOs(form.fingerprint_preset ?? 'win10');

    if (form.webgl_vendor && form.webgl_renderer) {
      // Явно выбранный GPU не совпадает с ОС профиля
      const gpu = GPU_PRESETS.find(g => g.vendor === form.webgl_vendor && g.renderer === form.webgl_renderer);
      if (gpu && gpu.os !== gpuOs) {
        result.push($t('profile_warning_gpu_os'));
      }
    } else if (gpuOs !== 'linux') {
      // GPU не выбран на Windows/macOS — будет использован дефолт
      result.push($t('profile_warning_no_gpu_windows'));
    }

    if (form.proxy_id && form.timezone) {
      const proxy = proxies.find(p => p.id === form.proxy_id);
      const proxyRegion = getExpectedRegion(proxy?.country);
      const tzRegion = getTimezoneRegion(form.timezone);
      if (proxyRegion && tzRegion && proxyRegion !== tzRegion) {
        result.push($t('profile_warning_tz_proxy'));
      }
    }

    return result;
  });

  onMount(async () => {
    presets = await api.fingerprintPresets();
  });

  async function submit() {
    if (!form.name.trim()) { error = $t('profile_error_name'); return; }
    saving = true; error = '';
    try {
      await api.profiles.create({ ...form, workspace_id: workspaceId });
      oncreated();
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }
</script>

<Drawer open title={$t('profile_create_title')} width="var(--drawer-w-md)" {onclose}>
      <form onsubmit={(e) => { e.preventDefault(); submit(); }}>
        {#if error}
          <div class="error-msg" style="margin-bottom:0.75rem">{error}</div>
        {/if}

        <!-- General -->
        <div class="section">
          <div class="section-label">{$t('profile_section_general')}</div>

          <div class="form-group">
            <label for="cp-name">{$t('profile_field_name')} *</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input
              id="cp-name"
              type="text"
              bind:value={form.name}
              placeholder={$t('profile_field_name_placeholder')}
              autofocus
            />
          </div>

          <div class="form-group">
            <label for="cp-browser">{$t('profile_field_browser')}</label>
            <CustomSelect
              id="cp-browser"
              bind:value={form.browser_type}
              options={[
                { label: 'Camoufox', value: 'camoufox' },
                { label: 'Firefox',  value: 'firefox' },
              ]}
            />
          </div>

          <div class="form-group">
            <label for="cp-proxy">{$t('profile_field_proxy')}</label>
            <CustomSelect
              id="cp-proxy"
              bind:value={form.proxy_id}
              options={proxyOptions}
            />
          </div>
        </div>

        <div class="divider"></div>

        <!-- Fingerprint -->
        <div class="section">
          <div class="section-label">{$t('profile_section_fingerprint')}</div>

          <div class="form-group">
            <label for="cp-preset">{$t('profile_field_preset')}</label>
            <CustomSelect
              id="cp-preset"
              bind:value={form.fingerprint_preset}
              options={presets.map(p => ({ label: p.label, value: p.id }))}
              onchange={(preset) => {
                const def = defaultGpuForPreset(preset ?? 'win10');
                if (def) {
                  form.webgl_vendor = def.vendor;
                  form.webgl_renderer = def.renderer;
                } else {
                  form.webgl_vendor = null;
                  form.webgl_renderer = null;
                }
              }}
            />
          </div>

          <div class="form-group">
            <label for="cp-gpu">{$t('profile_field_gpu')}</label>
            <CustomSelect
              id="cp-gpu"
              value={form.webgl_vendor && form.webgl_renderer ? `${form.webgl_vendor}|${form.webgl_renderer}` : ''}
              options={[
                { label: $t('profile_gpu_none'), value: '' },
                ...gpuOptions.map(g => ({ label: g.label, value: `${g.vendor}|${g.renderer}` })),
              ]}
              onchange={(v) => {
                const gpu = GPU_PRESETS.find(g => `${g.vendor}|${g.renderer}` === v);
                form.webgl_vendor = gpu?.vendor ?? null;
                form.webgl_renderer = gpu?.renderer ?? null;
              }}
            />
          </div>

          <div class="form-group">
            <label for="cp-locale">{$t('profile_field_locale')}</label>
            <CustomSelect
              id="cp-locale"
              bind:value={form.locale}
              options={LOCALES.map(l => ({ label: `${l.label} — ${l.locale}`, value: l.locale }))}
              onchange={(v) => {
                const opt = LOCALES.find(l => l.locale === v);
                if (opt) form.languages = opt.languages;
              }}
            />
          </div>

          <div class="form-group">
            <label for="cp-tz">{$t('profile_field_timezone')}</label>
            <CustomSelect
              id="cp-tz"
              bind:value={form.timezone}
              placeholder={$t('profile_timezone_none')}
              options={[
                { label: $t('profile_timezone_none'), value: null },
                ...TIMEZONES.map(tz => ({ label: tz.label, value: tz.value })),
              ]}
            />
          </div>

          <div class="form-group">
            <label for="cp-webrtc">{$t('profile_field_webrtc')}</label>
            <CustomSelect
              id="cp-webrtc"
              bind:value={form.webrtc_mode}
              options={[
                { label: $t('profile_webrtc_disable'), value: 'disable' },
                { label: $t('profile_webrtc_proxy'),   value: 'proxy_only' },
                { label: $t('profile_webrtc_real_ip'), value: 'real_ip' },
              ]}
            />
          </div>

          <div class="form-group">
            <label for="cp-search">{$t('profile_field_search_engine')}</label>
            <CustomSelect
              id="cp-search"
              bind:value={form.default_search_engine}
              options={[
                { label: 'DuckDuckGo', value: 'ddg' },
                { label: 'Google',     value: 'google' },
                { label: 'Brave Search', value: 'brave' },
                { label: 'Startpage', value: 'startpage' },
              ]}
            />
          </div>

          <div class="toggle-row">
            <div class="toggle-info">
              <span class="toggle-label">{$t('profile_field_history')}</span>
              <span class="toggle-hint">{$t('profile_field_history_hint')}</span>
            </div>
            <button
              type="button"
              class="toggle-btn"
              class:active={form.history_enabled}
              onclick={() => (form.history_enabled = !form.history_enabled)}
              aria-pressed={form.history_enabled}
              aria-label={$t('profile_field_history')}
            >
              <span class="toggle-thumb"></span>
            </button>
          </div>

        </div>

        {#if warnings().length > 0}
          <div class="warnings">
            <div class="warning-title">⚠ {$t('profile_warning_title')}</div>
            {#each warnings() as w}
              <div class="warning-item">• {w}</div>
            {/each}
          </div>
        {/if}

        <div class="divider"></div>

        <div class="form-group">
          <label for="cp-notes">{$t('profile_field_notes')}</label>
          <textarea id="cp-notes" bind:value={form.notes} rows="2" placeholder={$t('profile_field_notes_placeholder')}></textarea>
        </div>

        <div class="form-actions">
          <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('profile_btn_cancel')}</button>
          <button type="submit" class="btn btn-primary" disabled={saving}>
            {saving ? $t('profile_btn_creating') : $t('profile_btn_create')}
          </button>
        </div>
      </form>
</Drawer>

<style>
  .section { display: flex; flex-direction: column; gap: 1rem; margin-bottom: var(--sp-4); }

  .section-label {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-dim);
    text-transform: uppercase; letter-spacing: 0.9px;
    margin-bottom: 0.25rem;
  }

  .divider { height: 1px; background: var(--border); margin: var(--sp-4) 0; }

  /* Tall drawer form fields per design (46px / radius 11) */
  form :global(input[type='text']),
  form :global(input[type='number']) {
    height: var(--control-h-lg);
    border-radius: var(--radius-field);
    font-size: 0.9rem;
  }
  form textarea { border-radius: var(--radius-field); font-size: 0.9rem; }

  .warnings {
    background: var(--warn-bg);
    border: 1px solid var(--warn-border);
    border-radius: var(--radius);
    padding: 0.6rem var(--sp-3);
    margin-bottom: var(--sp-3);
    font-size: var(--fs-sm);
  }
  .warning-title { font-weight: 700; margin-bottom: var(--sp-1); color: var(--warn-text); }
  .warning-item { color: var(--text-2); }

  .form-actions {
    display: flex; gap: 10px; justify-content: flex-end;
    padding-top: var(--sp-3);
  }
  .form-actions .btn { height: 42px; border-radius: var(--radius-field); padding: 0 20px; }

  .toggle-row {
    display: flex; align-items: center; justify-content: space-between;
    padding: 1rem 0; gap: var(--sp-3);
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .toggle-info { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .toggle-label { font-size: 0.95rem; font-weight: var(--fw-semibold); color: var(--text); }
  .toggle-hint { font-size: 0.8rem; color: var(--text-faint); line-height: 1.5; max-width: 290px; }

  .toggle-btn {
    position: relative; flex-shrink: 0;
    width: 54px; height: 30px;
    background: var(--toggle-off); border: none; border-radius: var(--radius-pill);
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
</style>

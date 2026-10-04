<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { hasHome } from '$lib/core/mobile/nav';
  import Icon from '$lib/core/Icon.svelte';
  import PickerSheet from '$lib/core/mobile/PickerSheet.svelte';
  import { formatDateTime } from '$lib/core/utils';
  import {
    pwSettings,
    generatePassword,
    HISTORY_LIMITS,
    historyLimitId,
    historyLimitFromId,
  } from '$lib/pass/password-gen';
  import { pwgenHistory } from '$lib/pass/store/pwgen-history.svelte';
  import { t, locale } from '$lib/core/mobile/i18n';

  let password = $state('');
  let error = $state('');
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout>;
  let limitOpen = $state(false);
  /** History rows shown in the clear; the rest are dots, as on the desktop. */
  let revealed = $state<Set<string>>(new Set());

  const history = $derived(pwgenHistory.entries);
  const limitOptions = $derived(HISTORY_LIMITS.map((o) => ({ id: o.id, label: $t(o.label) })));
  const limitLabel = $derived(
    limitOptions.find((o) => o.id === historyLimitId($pwSettings.historyLimit))?.label ?? '',
  );

  // The history is read while the screen is up, also with the switch off: the
  // passwords saved before stay in the database, and the screen says so and
  // offers to clear them. The lock unmounts the screen, and the passwords
  // leave memory with it.
  onMount(() => {
    const release = pwgenHistory.hold();
    void pwgenHistory.load();
    return release;
  });

  function generate() {
    error = '';
    try {
      password = generatePassword($pwSettings);
      if ($pwSettings.historyEnabled) void pwgenHistory.record(password, $pwSettings.historyLimit);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      error = msg === 'pwgen_error_no_charset' ? $t('pwgen_error_no_charset') : msg;
    }
  }

  async function copyText(text: string) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      return; // the clipboard refused: no "Copied"
    }
    clearTimeout(toastTimer);
    toast = $t('pwgen_copied');
    toastTimer = setTimeout(() => (toast = ''), 1600);
  }

  const flags = [
    { key: 'uppercase', label: 'pwgen_upper' },
    { key: 'lowercase', label: 'pwgen_lower' },
    { key: 'numbers', label: 'pwgen_digits' },
    { key: 'symbols', label: 'pwgen_symbols' },
    { key: 'excludeSimilar', label: 'pwgen_exclude_similar' },
  ] as const;

  function toggle(key: (typeof flags)[number]['key'] | 'historyEnabled') {
    pwSettings.update((s) => ({ ...s, [key]: !s[key] }));
  }

  function toggleHistory() {
    toggle('historyEnabled');
    // The list comes back hidden, whichever way the switch went.
    revealed = new Set();
    void pwgenHistory.load();
  }

  function toggleReveal(id: string) {
    const next = new Set(revealed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    revealed = next;
  }

  async function clearHistory() {
    if (await pwgenHistory.confirmClear($t)) revealed = new Set();
  }
</script>

<div class="m-page">
  <div class="m-header">
    {#if hasHome()}
      <a class="m-ibtn" href="/" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    {/if}
    <h1 class="m-title">{$t('tools_generator')}</h1>
  </div>

  <div class="m-body">
  <div class="output m-card" class:empty={!password}>
    <span class="pw mono">{password || $t('pwgen_placeholder')}</span>
    <button class="m-ibtn" disabled={!password} onclick={() => copyText(password)} aria-label={$t('pwgen_btn_copy')}>
      <Icon name="copy" size={20} />
    </button>
  </div>

  {#if error}
    <div class="m-error">{error}</div>
  {/if}

  <div class="m-list">
    <div class="m-row length">
      <span class="m-row-label">{$t('pwgen_length_label', { n: String($pwSettings.length) })}</span>
      <input type="range" min="8" max="64" bind:value={$pwSettings.length} />
    </div>
    {#each flags as f (f.key)}
      <button class="m-row" onclick={() => toggle(f.key)}>
        <span class="m-row-label">{$t(f.label)}</span>
        <span class="toggle" class:on={$pwSettings[f.key]}></span>
      </button>
    {/each}
  </div>

  <button class="m-btn-grad generate" onclick={generate}>
    <Icon name="refresh-cw" size={18} />
    {$t('pwgen_btn_generate')}
  </button>

  <div class="m-list history-settings">
    <button class="m-row" role="switch" aria-checked={$pwSettings.historyEnabled} onclick={toggleHistory}>
      <span class="m-row-label">{$t('pwgen_save_history')}</span>
      <span class="toggle" class:on={$pwSettings.historyEnabled}></span>
    </button>
    {#if $pwSettings.historyEnabled}
      <button class="m-row" onclick={() => (limitOpen = true)}>
        <span class="m-row-label">{$t('pwgen_limit_label')}</span>
        <span class="m-row-value">{limitLabel}</span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
    {/if}
  </div>
  {#if $pwSettings.historyEnabled}
    <p class="m-hint under-list">{$t('pwgen_history_local_note')}</p>
  {:else}
    <p class="m-hint under-list">
      {$t('pwgen_history_off_hint')}
      {#if pwgenHistory.loaded && history.length > 0}
        {$t('pwgen_history_off_kept', { n: String(history.length) })}
      {/if}
    </p>
    {#if pwgenHistory.loaded && history.length > 0}
      <button class="m-link danger off-clear" onclick={clearHistory}>
        <Icon name="trash" size={16} />
        {$t('pwgen_btn_clear')}
      </button>
    {/if}
  {/if}

  {#if $pwSettings.historyEnabled && pwgenHistory.loaded}
    <div class="history-head">
      <span class="m-section">{$t('pwgen_history_title', { n: String(history.length) })}</span>
      {#if history.length > 0}
        <button class="m-link danger" onclick={clearHistory}>
          <Icon name="trash" size={16} />
          {$t('pwgen_btn_clear')}
        </button>
      {/if}
    </div>
    {#if history.length === 0}
      <div class="m-card">
        <div class="m-empty">
          <Icon name="clock" size={40} />
          <p>{$t('pwgen_history_empty')}</p>
          <span>{$t('pwgen_history_empty_hint')}</span>
        </div>
      </div>
    {:else}
      <div class="m-list">
        {#each history as entry (entry.id)}
          {@const shown = revealed.has(entry.id)}
          <div class="m-row entry">
            <button class="entry-copy" onclick={() => copyText(entry.password)} aria-label={$t('pwgen_btn_copy')}>
              <span class="entry-pw mono" class:masked={!shown}>
                {shown ? entry.password : '•'.repeat(Math.min(entry.password.length, 20))}
              </span>
              <span class="entry-date">{formatDateTime(entry.created_at, $locale)}</span>
            </button>
            <button class="m-ibtn" onclick={() => toggleReveal(entry.id)} aria-label={$t('pwgen_btn_show_hide')}>
              <Icon name={shown ? 'eye-off' : 'eye'} size={20} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  {#if toast}
    <div class="toast">{toast}</div>
  {/if}
  </div>
</div>

<PickerSheet
  open={limitOpen}
  title={$t('pwgen_limit_label')}
  options={limitOptions}
  value={historyLimitId($pwSettings.historyLimit)}
  onclose={() => (limitOpen = false)}
  onpick={(id) => pwSettings.update((s) => ({ ...s, historyLimit: historyLimitFromId(id) }))}
/>

<style>
  .output {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-height: 60px;
    padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-4);
    margin-bottom: var(--sp-4);
  }
  .output.empty .pw { color: var(--text-3); }
  .pw {
    flex: 1;
    font-size: 15px;
    word-break: break-all;
    user-select: text;
    -webkit-user-select: text;
  }
  .m-ibtn:disabled { opacity: 0.35; }
  .generate { margin-top: var(--sp-5); }
  .length {
    flex-direction: column;
    align-items: stretch;
    gap: var(--sp-2);
    padding-top: var(--sp-3);
    padding-bottom: var(--sp-3);
  }
  .length .m-row-label { font-weight: 600; }
  .length input[type='range'] {
    width: 100%;
    min-height: 0;
    padding: 0;
    border: 0;
    background: transparent;
    accent-color: var(--accent);
  }
  .toggle { width: 46px; height: 26px; }
  .toggle::after { width: 20px; height: 20px; }
  .toggle.on::after { left: 23px; }

  /* The history */
  .history-settings { margin-top: var(--sp-5); }
  .under-list { margin-top: var(--sp-2); padding: 0 var(--sp-1); }
  /* Right under the hint it belongs to, its icon in line with the text. */
  .under-list:has(+ .off-clear) { margin-bottom: 0; }
  .off-clear { margin-left: calc(-1 * var(--sp-1)); }
  .history-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }
  .history-head .m-section { padding-top: var(--sp-2); }
  .m-link.danger { color: var(--danger-text); }
  .entry { padding: 0 var(--sp-2) 0 0; gap: 0; }
  .entry-copy {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-4);
    min-height: 52px;
    justify-content: center;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
  }
  /* A long password wraps inside the row: never wider than the screen. */
  .entry-pw {
    max-width: 100%;
    overflow-wrap: anywhere;
    word-break: break-all;
  }
  .entry-pw.masked { color: var(--text-3); letter-spacing: 0.05em; }
  .entry-date { font-size: var(--fs-xs); color: var(--text-3); }
</style>

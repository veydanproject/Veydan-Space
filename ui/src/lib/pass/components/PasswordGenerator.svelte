<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { formatDateTime } from '$lib/core/utils';
  import { theme, toggleTheme } from '$lib/core/theme';
  import {
    pwSettings,
    generatePassword,
    HISTORY_LIMITS,
    historyLimitId,
    historyLimitFromId,
  } from '$lib/pass/password-gen';
  import { pwgenHistory } from '$lib/pass/store/pwgen-history.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import Pane from '$lib/core/ui/Pane.svelte';
  import { passUi } from '$lib/pass/store/ui.svelte';

  /** `as="pane"`: the body of the desktop Pass page instead of a drawer (the shell keeps the theme toggle). */
  let { open = $bindable(false), as = 'drawer' }: { open: boolean; as?: 'drawer' | 'pane' } = $props();
  const Frame = $derived(as === 'pane' ? Pane : Drawer);

  let errorMsg = $state('');
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout>;
  const history = $derived(pwgenHistory.entries);
  let revealedIds = $state<Set<string>>(new Set());

  // The lock unmounts the generator: its passwords leave memory (the store).
  onMount(() => pwgenHistory.hold());

  $effect(() => {
    if (open && $pwSettings.historyEnabled) {
      void pwgenHistory.load();
    }
  });

  function generate() {
    errorMsg = '';
    try {
      passUi.generated = generatePassword($pwSettings);
      passUi.generatedShown = true;
      if ($pwSettings.historyEnabled) {
        void pwgenHistory.record(passUi.generated, $pwSettings.historyLimit);
      }
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      errorMsg = msg === 'pwgen_error_no_charset' ? $t('pwgen_error_no_charset') : msg;
    }
  }

  function copyText(text: string) {
    navigator.clipboard.writeText(text);
    clearTimeout(toastTimer);
    toast = $t('pwgen_copied');
    toastTimer = setTimeout(() => (toast = ''), 1800);
  }

  function toggleReveal(id: string) {
    const next = new Set(revealedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    revealedIds = next;
  }

  async function clearHistory() {
    if (await pwgenHistory.confirmClear($t)) revealedIds = new Set();
  }

  const formatDate = (iso: string) => formatDateTime(iso, $locale);
</script>

{#snippet themeToggle()}
  <button class="icon-btn" onclick={toggleTheme} title={$t('theme_toggle')}>
    {#if $theme === 'dark'}
      <Icon name="sun" size={14} />
    {:else}
      <Icon name="moon" size={14} />
    {/if}
  </button>
{/snippet}

<Frame bind:open title={$t('pwgen_title')} actions={as === 'pane' ? undefined : themeToggle}>

  <div class="pw-content">
      <!-- Password output -->
      <div class="pw-output">
        <div class="pw-field-wrap">
          {#if passUi.generated}
            <!-- The whole password, wrapped in a narrow pane: never cut with an ellipsis -->
            <div class="pw-field pw-value filled mono" class:masked={!passUi.generatedShown}>
              {passUi.generatedShown ? passUi.generated : '•'.repeat(passUi.generated.length)}
            </div>
          {:else}
            <input type="text" class="pw-field" readonly value="" placeholder={$t('pwgen_placeholder')} />
          {/if}
          {#if passUi.generated}
            <button class="icon-btn pw-eye" onclick={() => (passUi.generatedShown = !passUi.generatedShown)} title={$t('pwgen_btn_show_hide')}>
              <Icon name={passUi.generatedShown ? 'eye-off' : 'eye'} size={14} />
            </button>
            <button class="icon-btn pw-copy" onclick={() => copyText(passUi.generated)} title={$t('pwgen_btn_copy')}>
              <Icon name="copy" size={14} />
            </button>
          {/if}
        </div>

        {#if errorMsg}
          <div class="error-msg">{errorMsg}</div>
        {/if}

        <button class="btn btn-primary btn-generate" onclick={generate}>
          <Icon name="refresh-cw" size={14} />
          {$t('pwgen_btn_generate')}
        </button>
      </div>

      <!-- Settings -->
      <div class="section">
        <button class="section-toggle" onclick={() => (passUi.generatorSettingsOpen = !passUi.generatorSettingsOpen)}>
          <Icon name="settings" size={13} />
          {$t('pwgen_btn_settings')}
          <Icon name={passUi.generatorSettingsOpen ? 'chevron-right' : 'chevron-right'} size={13} class={passUi.generatorSettingsOpen ? 'rot90' : ''} />
        </button>

        {#if passUi.generatorSettingsOpen}
          <div class="settings-body">
            <div class="setting-row">
              <label for="pwgen-length">{$t('pwgen_length')} <strong>{$pwSettings.length}</strong></label>
              <input
                id="pwgen-length"
                type="range" min="8" max="64"
                bind:value={$pwSettings.length}
              />
            </div>

            <div class="checkboxes">
              <label class="cb-label">
                <input type="checkbox" bind:checked={$pwSettings.uppercase} />
                {$t('pwgen_upper')}
              </label>
              <label class="cb-label">
                <input type="checkbox" bind:checked={$pwSettings.lowercase} />
                {$t('pwgen_lower')}
              </label>
              <label class="cb-label">
                <input type="checkbox" bind:checked={$pwSettings.numbers} />
                {$t('pwgen_digits')}
              </label>
              <label class="cb-label">
                <input type="checkbox" bind:checked={$pwSettings.symbols} />
                {$t('pwgen_symbols')}
              </label>
              <label class="cb-label">
                <input type="checkbox" bind:checked={$pwSettings.excludeSimilar} />
                {$t('pwgen_exclude_similar')}
              </label>
            </div>

            <div class="divider"></div>

            <label class="cb-label">
              <input type="checkbox" bind:checked={$pwSettings.historyEnabled} />
              {$t('pwgen_save_history')}
            </label>

            {#if $pwSettings.historyEnabled}
              <div class="setting-row warn-note">
                <Icon name="info" size={13} />
                {$t('pwgen_history_local_note')}
              </div>
              <div class="setting-row">
                <label for="pwgen-hlimit">{$t('pwgen_limit_label')}</label>
                <CustomSelect
                  id="pwgen-hlimit"
                  options={HISTORY_LIMITS.map((o) => ({ label: $t(o.label), value: o.id }))}
                  value={historyLimitId($pwSettings.historyLimit)}
                  onchange={(v) => {
                    pwSettings.update(s => ({ ...s, historyLimit: historyLimitFromId(v) }));
                  }}
                />
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <!-- History -->
      {#if $pwSettings.historyEnabled}
        <div class="section">
          <div class="section-header-row">
            <span class="section-label">
              <Icon name="clock" size={13} />
              {$t('pwgen_history_title', { n: String(history.length) })}
            </span>
            {#if history.length > 0}
              <button class="btn btn-ghost btn-sm" onclick={clearHistory}>
                <Icon name="trash" size={13} />
                {$t('pwgen_btn_clear')}
              </button>
            {/if}
          </div>

          {#if history.length === 0}
            <p class="empty-note">{$t('pwgen_history_empty')}</p>
          {:else}
            <ul class="history-list">
              {#each history as entry (entry.id)}
                <li class="history-item">
                  <div class="history-pw">
                    {#if revealedIds.has(entry.id)}
                      <span class="pw-text mono">{entry.password}</span>
                    {:else}
                      <span class="pw-text mono masked">{'•'.repeat(Math.min(entry.password.length, 20))}</span>
                    {/if}
                    <span class="history-date">{formatDate(entry.created_at)}</span>
                  </div>
                  <div class="history-actions">
                    <button class="icon-btn" onclick={() => toggleReveal(entry.id)} title={$t('pwgen_btn_show_hide')}>
                      <Icon name={revealedIds.has(entry.id) ? 'eye-off' : 'eye'} size={13} />
                    </button>
                    <button class="icon-btn" onclick={() => copyText(entry.password)} title={$t('pwgen_btn_copy')}>
                      <Icon name="copy" size={13} />
                    </button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
  </div>

  {#if toast}
    <div class="toast">{toast}</div>
  {/if}
</Frame>

<style>
  .pw-content {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  /* Password output */
  .pw-output {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .pw-field-wrap {
    position: relative;
    display: flex;
    align-items: flex-start;
  }

  /* Generated password display — large, on --surface-3; the value is `.mono` */
  .pw-field {
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
    letter-spacing: 0.04em;
    height: var(--control-h-lg);
    background: var(--surface-3);
    border-color: var(--border);
    border-radius: var(--radius-md);
    color: var(--text);
    text-overflow: ellipsis;
  }
  .pw-value {
    width: 100%;
    height: auto;
    min-height: var(--control-h-lg);
    display: flex;
    align-items: center;
    padding: 0.55rem 0.75rem;
    border: 1px solid var(--border);
    line-height: 1.45;
    overflow-wrap: anywhere;
    word-break: break-all;
    user-select: text;
    -webkit-user-select: text;
  }
  .pw-value.masked { letter-spacing: 0.08em; user-select: none; -webkit-user-select: none; }
  /* Room for the reveal and copy buttons, which show once there is a password. */
  .pw-field.filled { padding-right: 4.5rem; }
  /* The hint is a sentence, not a password: the UI font (the empty field is not `.mono`), so it fits a narrow pane in either language. */
  .pw-field::placeholder {
    font-size: var(--fs-base);
    font-weight: var(--fw-medium);
    letter-spacing: 0;
  }

  .pw-eye {
    position: absolute;
    top: calc((var(--control-h-lg) - 32px) / 2);
    right: 2.25rem;
    background: transparent;
    border: none;
  }

  .pw-copy {
    position: absolute;
    top: calc((var(--control-h-lg) - 32px) / 2);
    right: var(--sp-1);
    background: transparent;
    border: none;
  }

  .btn-generate {
    width: 100%;
    justify-content: center;
    gap: var(--sp-2);
    height: 42px;
    border-radius: var(--radius-field);
  }

  /* Sections — nested cards (--surface-2 + radius-md) */
  .section {
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    overflow: hidden;
  }

  .section-toggle {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0.55rem var(--sp-3);
    background: transparent;
    border: none;
    border-radius: 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    color: var(--text-2);
    text-align: left;
    cursor: pointer;
    transition: background var(--dur-fast);
  }

  .section-toggle:hover { background: var(--surface-hover); }

  :global(.rot90) { transform: rotate(90deg); }

  .settings-body {
    padding: var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }

  .setting-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-sm);
    color: var(--text-2);
  }

  .setting-row label {
    min-width: 80px;
    flex-shrink: 0;
    font-weight: 500;
    color: var(--text-2);
  }

  .setting-row input[type="range"] {
    flex: 1;
    padding: 0;
    background: transparent;
    border: none;
    accent-color: var(--accent);
  }

  .checkboxes {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .cb-label {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-sm);
    color: var(--text);
    cursor: pointer;
  }


  .divider {
    height: 1px;
    background: var(--border);
    margin: 0.1rem 0;
  }

  .warn-note {
    background: var(--warn-bg);
    color: var(--warn-text);
    padding: 0.4rem 0.6rem;
    border-radius: var(--radius-sm);
    font-size: var(--fs-xs);
  }

  /* Section header */
  .section-header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-2) var(--sp-3);
    background: transparent;
    border-bottom: 1px solid var(--border);
  }

  .section-label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--fs-2xs);
    font-weight: var(--fw-bold);
    text-transform: uppercase;
    letter-spacing: 0.7px;
    color: var(--text-dim);
  }

  .empty-note {
    padding: var(--sp-3);
    font-size: var(--fs-sm);
    color: var(--text-3);
    text-align: center;
  }

  /* History */
  .history-list {
    list-style: none;
    display: flex;
    flex-direction: column;
  }

  .history-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.45rem var(--sp-3);
    border-top: 1px solid var(--border);
    gap: var(--sp-2);
  }

  .history-item:first-child { border-top: none; }

  .history-pw {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .pw-text {
    font-size: var(--fs-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 180px;
  }

  .masked { color: var(--text-3); letter-spacing: 0.05em; }

  .history-date {
    font-size: var(--fs-2xs);
    color: var(--text-3);
  }

  .history-actions {
    display: flex;
    gap: 0.3rem;
    flex-shrink: 0;
  }

  /* Toast */
  .toast {
    position: absolute;
    bottom: var(--sp-4);
    left: 50%;
    transform: translateX(-50%);
    background: var(--success);
    color: #fff;
    font-size: var(--fs-sm);
    font-weight: 600;
    padding: 0.4rem var(--sp-4);
    border-radius: var(--radius-pill);
    box-shadow: var(--shadow);
    pointer-events: none;
    animation: fade-in 0.15s ease;
    white-space: nowrap;
  }

  @keyframes fade-in {
    from { opacity: 0; transform: translateX(-50%) translateY(4px); }
    to   { opacity: 1; transform: translateX(-50%) translateY(0); }
  }
</style>

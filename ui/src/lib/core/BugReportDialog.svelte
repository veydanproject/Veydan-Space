<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { get } from 'svelte/store';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { api } from '$lib/core/api';
  import { registry } from '$lib/core/registry';
  import { locale, t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { product } from '$lib/core/product';
  import type { HostInfo } from '$lib/core/types';
  import { formatError } from '$lib/core/utils';

  const BUG_EMAIL = 'bug@veydan.net';

  interface Props {
    open: boolean;
  }

  let { open = $bindable(false) }: Props = $props();

  let text = $state('');
  let copied = $state(false);
  let mailError = $state('');
  let wasOpen = false;

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  function localDate(): string {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
  }

  /** Plain-text report with version facts, the modules' lines and empty fields for the user. */
  function buildReport(info: HostInfo | null, modules: string[]): string {
    const tr = get(t);
    const lines = [
      product.name,
      '',
      `${tr('settings_about_version')}: ${info?.version || '—'}`,
      `${tr('settings_bug_os')}: ${info ? `${info.os}/${info.arch}` : '—'}`,
      ...modules,
    ];
    lines.push(
      `${tr('settings_bug_language')}: ${get(locale)}`,
      `${tr('settings_bug_date')}: ${localDate()}`,
      '',
      tr('settings_bug_what'),
      '',
      '',
      tr('settings_bug_steps'),
      '',
      '',
      tr('settings_bug_expected'),
      '',
      '',
    );
    return lines.join('\n');
  }

  /** What the modules know about themselves (the browser's Camoufox version); a failing module adds nothing. */
  async function moduleLines(): Promise<string[]> {
    const parts = await Promise.all(registry.active.map((m) => m.report?.().catch(() => []) ?? []));
    return parts.flat();
  }

  function fill() {
    copied = false;
    mailError = '';
    const draft = buildReport(null, []);
    text = draft;
    void Promise.all([api.system.hostInfo(), moduleLines()]).then(([info, modules]) => {
      if (!info?.os || !info.arch) return;
      if (text === draft) text = buildReport(info, modules);
    }).catch(() => {});
  }

  $effect(() => {
    if (open && !wasOpen) fill();
    wasOpen = open;
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      mailError = '';
    } catch (e) {
      mailError = formatError(e);
    }
  }

  function mailtoUrl(): string {
    const subject = encodeURIComponent(get(t)('settings_bug_subject'));
    const body = encodeURIComponent(text);
    return `mailto:${BUG_EMAIL}?subject=${subject}&body=${body}`;
  }

  async function openMail() {
    const url = mailtoUrl();
    mailError = '';
    if (isTauri && !isMobile) {
      try {
        await api.system.openUrl(url);
      } catch (e) {
        mailError = formatError(e);
      }
      return;
    }
    const a = document.createElement('a');
    a.href = url;
    a.target = '_blank';
    a.rel = 'noopener';
    a.click();
  }
</script>

<Dialog bind:open title={$t('settings_bug_report')} width="520px">
  <p class="hint">{$t('settings_bug_hint')}</p>
  <p class="email">{BUG_EMAIL}</p>
  <textarea rows="14" bind:value={text} spellcheck="false"></textarea>
  {#if mailError}<p class="err">{mailError}</p>{/if}
  {#snippet footer()}
    <button type="button" class="btn btn-primary btn-sm" onclick={copy}>
      {copied ? $t('settings_bug_copied') : $t('settings_bug_copy')}
    </button>
    <button type="button" class="btn btn-ghost btn-sm" onclick={openMail}>
      {$t('settings_bug_mail')}
    </button>
  {/snippet}
</Dialog>

<style>
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  .email {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--text);
    user-select: all;
  }
  textarea {
    min-height: 220px;
    resize: vertical;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
    white-space: pre;
  }
  .err { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import { get } from 'svelte/store';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import type { BugSender } from '$lib/core/module';
  import { api } from '$lib/core/api';
  import { registry } from '$lib/core/registry';
  import { locale, t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { product } from '$lib/core/product';
  import type { HostInfo } from '$lib/core/types';
  import { formatError } from '$lib/core/utils';

  const BUG_EMAIL = 'bug@veydan.net';
  /** The longest text the server takes. */
  const TEXT_MAX = 8000;

  interface Props {
    open: boolean;
  }

  let { open = $bindable(false) }: Props = $props();

  let text = $state('');
  let mailError = $state('');
  let wasOpen = false;
  /** The text as the dialog filled it: a report that says nothing more is not sent. */
  let draft = $state('');
  let info: HostInfo | null = null;
  let modules: string[] = [];
  /**
   * The module a report goes through (the messenger, with an account on the
   * project's servers); without one the text is copied or mailed, as before.
   */
  let sender = $state<BugSender | null>(null);
  let attachNet = $state(true);
  let phase = $state<'idle' | 'checking' | 'sending'>('idle');
  const busy = $derived(phase !== 'idle');
  const written = $derived(text.trim() !== '' && text !== draft);

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

  /** The first active module that can send a report now. */
  async function findSender(): Promise<BugSender | null> {
    for (const m of registry.active) {
      if (m.bug && (await m.bug.available().catch(() => false))) return m.bug;
    }
    return null;
  }

  function fill() {
    mailError = '';
    phase = 'idle';
    attachNet = true;
    info = null;
    modules = [];
    sender = null;
    draft = buildReport(null, []);
    text = draft;
    const first = draft;
    void findSender().then((found) => { sender = found; });
    void Promise.all([api.system.hostInfo(), moduleLines()]).then(([host, lines]) => {
      if (!host?.os || !host.arch) return;
      info = host;
      modules = lines;
      const filled = buildReport(host, lines);
      if (text === first) text = filled;
      draft = filled;
    }).catch(() => {});
  }

  /** The word of a refusal, said in the user's language. */
  function refusal(e: unknown): string {
    const tr = get(t);
    const word = formatError(e);
    if (word.includes('rate_limited')) return tr('settings_bug_err_rate');
    if (word.includes('bug_text') || word.includes('payload_too_large')) return tr('settings_bug_err_long');
    return tr('settings_bug_err_failed');
  }

  async function send() {
    const via = sender;
    if (!via || busy || !written) return;
    const tr = get(t);
    mailError = '';
    try {
      let net: unknown;
      if (attachNet) {
        phase = 'checking';
        // A check that failed does not hold the report back: it goes without.
        net = await via.diagnose().catch(() => undefined);
      }
      phase = 'sending';
      const status = await via.send({
        text: text.trim(),
        product: product.id,
        version: info?.version ?? '',
        os: info ? `${info.os}/${info.arch}` : '',
        locale: get(locale),
        // The limits of the server: a line past them would refuse the whole report.
        modules: modules.slice(0, 16).map((line) => line.slice(0, 200)),
        net,
      });
      if (status === 'queued') toast.info(tr('settings_bug_queued'));
      else toast.success(tr('settings_bug_sent'));
      open = false;
    } catch (e) {
      // The dialog stays with what was written: a refusal is no reason to write it again.
      mailError = refusal(e);
    } finally {
      phase = 'idle';
    }
  }

  $effect(() => {
    if (open && !wasOpen) fill();
    wasOpen = open;
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      copiedToast();
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
  {#if sender}
    <p class="hint">{$t('settings_bug_hint_send')}</p>
  {:else}
    <p class="hint">{$t('settings_bug_hint')}</p>
    <p class="email">{BUG_EMAIL}</p>
  {/if}
  <textarea rows="14" bind:value={text} spellcheck="false" maxlength={TEXT_MAX} disabled={busy}></textarea>
  {#if sender}
    <ToggleRow title={$t('settings_bug_net')} hint={$t('settings_bug_net_hint')} bind:on={attachNet} disabled={busy} />
  {/if}
  {#if phase === 'checking'}<p class="hint" role="status">{$t('settings_bug_checking')}</p>{/if}
  {#if mailError}<Notice tone="danger">{mailError}</Notice>{/if}
  {#snippet footer()}
    {#if sender}
      <button type="button" class="btn btn-ghost" onclick={copy} disabled={busy}>
        {$t('settings_bug_copy')}
      </button>
      <button type="button" class="btn btn-primary" onclick={send} disabled={busy || !written}>
        {$t(phase === 'idle' ? 'settings_bug_send' : phase === 'checking' ? 'settings_bug_checking_short' : 'settings_bug_sending')}
      </button>
    {:else}
      <button type="button" class="btn btn-ghost" onclick={openMail}>
        {$t('settings_bug_mail')}
      </button>
      <button type="button" class="btn btn-primary" onclick={copy}>
        {$t('settings_bug_copy')}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .hint { margin: 0; line-height: var(--lh-snug); }
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
    line-height: var(--lh-snug);
    white-space: pre;
  }
</style>

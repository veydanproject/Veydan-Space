<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Where the transfer of one file is, and what can be done with it. Two
  looks: `compact`, a ring with its percent and its main action (a picture,
  a circle, a voice message), and `full`, two lines of words, the buttons
  and a thin bar (a file, the list of transfers). `p` is `null` while the
  file is elsewhere and nothing fetches it.
-->
<script lang="ts">
  import { t, locale, localeTag } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerApi, type MessengerTransferProgress } from '../api';
  import { confirmStore } from '../shared/confirm.svelte';
  import { bytes, percent } from '../shared/format';
  import { mediaErrorText } from './errors';
  import { transferStore } from './transferStore.svelte';
  import {
    remoteLines, shortLine, transferActions, transferLines, type TransferAction, type TransferKey, type Translate, type Words,
  } from './transferLabel';

  interface Props {
    p: MessengerTransferProgress | null;
    look: 'compact' | 'full';
    /** Compact over a picture or a circle: white on dark, laid over it. */
    overlay?: boolean;
    /** The file: its name (asked about before an upload is cancelled), its size and parts while it is elsewhere. */
    name: string;
    size: number;
    chunks?: number;
    messageId?: string | null;
    /** Fetches the file. A bubble passes its own, to show what arrives; without it the runtime is asked here. */
    ondownload?: () => void;
    /** A fetch was asked and has not answered yet. */
    busy?: boolean;
    /** Called before a question is asked about the transfer: a sheet it lies in steps aside for it. */
    onask?: () => void;
  }
  let { p, look, overlay = false, name, size, chunks, messageId = null, ondownload, busy = false, onask }: Props = $props();

  /** An upload with more than this sent asks before it is cancelled: its message goes too. */
  const ASK_ABOVE = 20 * 1024 * 1024;

  const LABEL: Record<TransferAction, TransferKey | 'msg_media_pause' | 'msg_media_resume' | 'msg_media_download' | 'msg_media_cancel'> = {
    pause: 'msg_media_pause',
    resume: 'msg_media_resume',
    retry_now: 'msg_xfer_retry_now',
    retry: 'msg_xfer_retry',
    retry_download: 'msg_xfer_retry_download',
    download: 'msg_media_download',
    cancel: 'msg_media_cancel',
  };
  const ICON: Record<TransferAction, string> = {
    pause: 'pause', resume: 'play', retry_now: 'refresh-cw', retry: 'refresh-cw', retry_download: 'refresh-cw', download: 'download', cancel: 'x',
  };
  /** Actions said in words on their button, not only drawn. */
  const WORDED: TransferAction[] = ['retry_now', 'retry', 'retry_download', 'download'];
  const RING = 2 * Math.PI * 17;

  // The countdown to the next attempt moves every second.
  let now = $state(Date.now());
  $effect(() => {
    if (p?.status !== 'waiting_retry' || !p.retry_at_ms) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const words = $derived<Words>({ t: $t as unknown as Translate, locale: $locale, now });
  const lines = $derived(p ? transferLines(p, words) : remoteLines(size, chunks, words));
  const pct = $derived(p ? percent(p.done_bytes, p.total_bytes) : 0);
  const actions = $derived(transferActions(p));
  const main = $derived(actions[0] ?? null);
  const failed = $derived(p?.status === 'failed');
  const pill = $derived(shortLine(p, size, words));

  let error = $state('');
  let acting = $state(false);

  const label = (a: TransferAction) => $t(LABEL[a]);
  const fetches = (a: TransferAction) => a === 'download' || a === 'retry_download';
  /** A transfer told only by its message has no id yet: only a fetch works then. */
  const usable = (a: TransferAction) => !acting && (fetches(a) ? !busy : !!p?.transfer_id);

  /**
   * A fetch lasts as long as its file: nothing waits for it, so the pause
   * and the cancel of the transfer stay usable meanwhile. Its events tell
   * how it goes; a refusal is shown here.
   */
  function background(call: Promise<unknown>) {
    call.catch((e) => (error = mediaErrorText(e, (key) => $t(key))));
  }

  async function run(a: TransferAction) {
    if (!usable(a)) return;
    error = '';
    acting = true;
    try {
      if (fetches(a)) {
        if (ondownload) ondownload();
        else if (messageId) background(messengerApi.media.download(messageId, true));
        return;
      }
      const id = p?.transfer_id;
      if (!id) return;
      if (a === 'pause') await messengerApi.media.pause(id);
      else if (a === 'cancel') {
        if (p?.direction === 'up' && p.done_bytes > ASK_ABOVE) {
          const text = $t('msg_xfer_cancel_confirm', { name, size: bytes(p.done_bytes, localeTag($locale)) });
          onask?.();
          if (!(await confirmStore.ask(text, $t('msg_xfer_cancel_upload'), true))) return;
        }
        await messengerApi.media.cancel(id);
        transferStore.cancelled(id);
      } else if (p?.direction === 'down') background(messengerApi.media.resume(id));
      else await messengerApi.media.resume(id);
    } catch (e) {
      error = mediaErrorText(e, (key) => $t(key));
    } finally {
      acting = false;
    }
  }

  function press(e: MouseEvent, a: TransferAction) {
    // A bubble's own press (open, look) is under the buttons.
    e.stopPropagation();
    run(a);
  }
</script>

{#if look === 'compact'}
  <div class="compact" class:overlay class:failed>
    <span class="ring-wrap">
      <span class="ring" role="progressbar" aria-label={name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-valuetext={lines.line1}>
        <svg viewBox="0 0 40 40" aria-hidden="true">
          <circle cx="20" cy="20" r="17" />
          <circle class="done" cx="20" cy="20" r="17" style="stroke-dasharray:{RING};stroke-dashoffset:{RING * (1 - pct / 100)}" />
        </svg>
      </span>
      <button class="ring-btn" disabled={!main || !usable(main)} onclick={(e) => main && press(e, main)}
        aria-label={main ? label(main) : lines.line1} title={[lines.line1, lines.line2, error].filter(Boolean).join(' · ')}>
        <span class:spin={!main || (busy && fetches(main))}><Icon name={main && !(busy && fetches(main)) ? ICON[main] : 'loader'} size={16} /></span>
      </button>
      {#if failed || error}<span class="bad" aria-hidden="true"><Icon name="alert-triangle" size={11} /></span>{/if}
    </span>
    {#if actions.includes('cancel')}
      <button class="mini" disabled={!usable('cancel')} onclick={(e) => press(e, 'cancel')} aria-label={label('cancel')} title={label('cancel')}>
        <Icon name="x" size={12} />
      </button>
    {/if}
    {#if pill}<span class="pill">{pill}</span>{/if}
  </div>
{:else}
  <div class="full" class:failed>
    <div class="row">
      <span class="lines" aria-live="polite">
        {#if lines.line1}<span class="l1">{lines.line1}</span>{/if}
        {#if lines.line2}<span class="l2">{lines.line2}</span>{/if}
        {#if error}<span class="err">{error}</span>{/if}
      </span>
      {#if actions.length}
        <span class="btns">
          {#each actions as a (a)}
            <button class="act" class:worded={WORDED.includes(a)} disabled={!usable(a)} onclick={(e) => press(e, a)} aria-label={label(a)} title={label(a)}>
              <span class:spin={busy && fetches(a)}><Icon name={busy && fetches(a) ? 'loader' : ICON[a]} size={13} /></span>
              {#if WORDED.includes(a)}<span>{label(a)}</span>{/if}
            </button>
          {/each}
        </span>
      {/if}
    </div>
    {#if p && p.status !== 'done' && p.status !== 'cancelled'}
      <span class="bar" role="progressbar" aria-label={name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-valuetext={lines.line1}>
        <span style="width:{pct}%"></span>
      </span>
    {/if}
  </div>
{/if}

<style>
  /* Compact: inline beside a voice message, or laid over a picture. */
  .compact { position: relative; display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .compact.overlay { position: absolute; inset: 0; flex-direction: column; justify-content: center; gap: 5px; pointer-events: none; }
  .compact.overlay > * { pointer-events: auto; }
  .ring-wrap { position: relative; width: 40px; height: 40px; flex-shrink: 0; }
  .overlay .ring-wrap { width: 44px; height: 44px; }
  .ring { position: absolute; inset: 0; pointer-events: none; }
  .ring svg { width: 100%; height: 100%; transform: rotate(-90deg); }
  .ring circle { fill: none; stroke: color-mix(in srgb, var(--accent) 22%, transparent); stroke-width: 2.5; }
  .ring circle.done { stroke: var(--accent); transition: stroke-dashoffset 0.25s var(--ease); }
  .overlay .ring circle { stroke: rgba(255, 255, 255, 0.25); }
  .overlay .ring circle.done { stroke: #fff; }
  .failed .ring circle.done { stroke: var(--danger-text); }
  .ring-btn {
    position: absolute; inset: 3px; border: none; border-radius: 50%; padding: 0; cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--accent) 12%, transparent); color: var(--accent-text-2);
  }
  .overlay .ring-btn { background: rgba(0, 0, 0, 0.45); color: #fff; }
  .ring-btn:disabled { cursor: default; }
  .ring-btn > span, .act > span:first-child { display: inline-flex; }
  .bad {
    position: absolute; top: -3px; right: -3px; width: 17px; height: 17px; border-radius: 50%; display: flex; align-items: center;
    justify-content: center; background: var(--danger-bg); color: var(--danger-text); pointer-events: none;
  }
  .mini {
    width: 24px; height: 24px; border: none; border-radius: 50%; padding: 0; cursor: pointer; display: inline-flex;
    align-items: center; justify-content: center; background: var(--surface-3); color: var(--text-2);
  }
  .overlay .mini { position: absolute; top: 6px; left: 6px; background: rgba(0, 0, 0, 0.5); color: #fff; }
  .pill { font-size: 10px; color: var(--text-3); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .overlay .pill { color: #fff; background: rgba(0, 0, 0, 0.45); padding: 1px 6px; border-radius: var(--radius-pill); }

  /* Full: words, buttons, a thin bar. The buttons go under the words when the place is narrow. */
  .full { display: flex; flex-direction: column; gap: 4px; min-width: 0; width: 100%; }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: 2px var(--sp-2); min-width: 0; }
  .lines { flex: 1 1 140px; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .l1 { font-size: var(--fs-2xs); color: var(--text-2); line-height: 1.3; overflow-wrap: anywhere; }
  .failed .l1 { color: var(--danger-text); }
  .l2 { font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.3; font-variant-numeric: tabular-nums; }
  .err { font-size: var(--fs-2xs); color: var(--danger-text); }
  .btns { display: inline-flex; flex-wrap: wrap; gap: 2px; margin-left: auto; }
  .act {
    border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; align-items: center; gap: 4px;
    padding: 6px; border-radius: var(--radius-sm); font: inherit; font-size: var(--fs-2xs); font-weight: var(--fw-semibold); white-space: nowrap;
  }
  .act.worded { color: var(--accent-text-2); padding: 5px 8px; background: color-mix(in srgb, var(--accent) 10%, transparent); }
  .act:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .act:disabled, .mini:disabled { opacity: 0.45; cursor: default; }
  .bar { display: block; height: 3px; border-radius: 2px; background: var(--surface-3); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width 0.25s var(--ease); }
  .failed .bar span { background: var(--danger-text); }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (pointer: coarse) {
    .act { padding: 9px; }
    .act.worded { padding: 8px 10px; }
    .mini { width: 30px; height: 30px; }
  }
</style>

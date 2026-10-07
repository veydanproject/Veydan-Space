<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Every transfer under way, paused or failed, newest first: the file, its
  chat, where it is and its buttons. A row opens its chat at its message.
  A sheet on the phone, a panel under the header's chip on a desk. It
  closes by itself when nothing is left in it.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { messengerApi, type MessengerTransferProgress } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { openChat } from '../content/actions';
  import { fileIcon } from '../shared/format';
  import { mediaErrorText } from './errors';
  import { transferStore } from './transferStore.svelte';
  import TransferStatus from './TransferStatus.svelte';

  interface Props {
    open: boolean;
    /** Where the chip is: the desk's panel hangs under it. */
    anchor?: DOMRect | null;
    /** A sheet from the bottom instead of a panel. */
    sheet?: boolean;
  }
  let { open = $bindable(), anchor = null, sheet = false }: Props = $props();

  const list = $derived(transferStore.list);
  /** "Retry all" has something to take up: failed, waiting, or paused by the closing of the app. */
  const retryable = $derived(transferStore.counts.retriable > 0);
  let error = $state('');
  let retrying = $state(false);

  /** The panel hangs under the chip, its right edge at the chip's, never off the screen. */
  const place = $derived.by(() => {
    const screen = typeof window === 'undefined' ? 1024 : window.innerWidth;
    const width = Math.min(400, screen - 16);
    const right = anchor?.right ?? screen - 8;
    return { width, left: Math.max(8, Math.min(right - width, screen - width - 8)) };
  });

  // Nothing left to show: the list goes.
  $effect(() => { if (open && list.length === 0) open = false; });
  $effect(() => { if (!open) error = ''; });

  const chatTitle = (chatId: string | null) => chatStore.chats.find((c) => c.id === chatId)?.title ?? '';

  /** Opens the chat of a transfer at its message. */
  async function show(p: MessengerTransferProgress) {
    if (!p.chat_id) return;
    open = false;
    if (p.message_id) chatStore.jumpTo = { chatId: p.chat_id, messageId: p.message_id };
    if (chatStore.activeId !== p.chat_id) await openChat(p.chat_id).catch(() => {});
  }

  async function retryAll() {
    error = '';
    retrying = true;
    try { await messengerApi.media.retryFailed(); }
    catch (e) { error = mediaErrorText(e, (key) => $t(key)); }
    finally { retrying = false; }
  }

  /**
   * A question about a row (the cancel of a large upload) is asked over the
   * screen. The phone's sheet lies in the body above it: it steps aside first.
   */
  const onask = $derived(sheet ? () => (open = false) : undefined);

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open && !sheet) { e.stopPropagation(); open = false; }
  }

  // A fixed layer inside a glass header is laid out against the header: the list lives in the body.
  function toBody(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  /** A press that starts and ends outside the panel closes it; a drag out of it does not. */
  let downOutside = false;
  function outside(e: MouseEvent) {
    if (downOutside && e.target === e.currentTarget) open = false;
    downOutside = false;
  }
</script>

<svelte:window {onkeydown} />

{#snippet rows()}
  <ul class="list">
    {#each list as p (p.transfer_id)}
      <li class="row" class:failed={p.status === 'failed'}>
        <button class="go" onclick={() => show(p)} disabled={!p.chat_id} title={$t('msg_xfer_show')} aria-label={`${$t('msg_xfer_show')}: ${p.file_name}`}>
          <span class="kind" aria-hidden="true">
            <Icon name={fileIcon(p.file_name, p.mime)} size={18} />
            <span class="dir"><Icon name={p.direction === 'up' ? 'arrow-up' : 'arrow-down'} size={10} /></span>
          </span>
          <span class="who">
            <span class="name">{p.file_name}</span>
            {#if chatTitle(p.chat_id)}<span class="chat">{chatTitle(p.chat_id)}</span>{/if}
          </span>
          <Icon name="chevron-right" size={14} />
        </button>
        <TransferStatus {p} look="full" name={p.file_name} size={p.total_bytes} messageId={p.message_id} {onask} />
      </li>
    {/each}
  </ul>
  {#if error}<div class="error">{error}</div>{/if}
{/snippet}

{#snippet footer()}
  {#if retryable}
    <button class="retry-all" onclick={retryAll} disabled={retrying}>
      <Icon name="refresh-cw" size={14} />{$t('msg_xfer_retry_all')}
    </button>
  {/if}
{/snippet}

{#if sheet}
  <div use:toBody>
    <Dialog bind:open title={$t('msg_xfer_title')} footer={retryable ? footer : undefined}>
      {@render rows()}
    </Dialog>
  </div>
{:else if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="layer" use:toBody role="presentation" onpointerdown={(e) => (downOutside = e.target === e.currentTarget)} onclick={outside}>
    <div class="panel" role="dialog" aria-label={$t('msg_xfer_title')}
      style:top={`${(anchor?.bottom ?? 58) + 6}px`} style:left={`${place.left}px`} style:width={`${place.width}px`}>
      <div class="head">
        <span class="title">{$t('msg_xfer_title')}</span>
        <button class="close" onclick={() => (open = false)} aria-label={$t('common_close')} title={$t('common_close')}><Icon name="x" size={14} /></button>
      </div>
      <div class="body">{@render rows()}</div>
      {#if retryable}<div class="foot">{@render footer()}</div>{/if}
    </div>
  </div>
{/if}

<style>
  /* Catches a press outside the panel; inside the window frame, below its title bar. */
  /* Below dialogs: the question before a cancel opens over the panel. */
  .layer { position: fixed; inset: var(--overlay-inset, 0); z-index: calc(var(--z-modal) - 1); }
  .panel {
    position: fixed; max-height: min(70vh, 560px); display: flex; flex-direction: column;
    background: var(--surface-drawer, var(--surface)); border: 1px solid var(--border); border-radius: var(--radius-lg); box-shadow: var(--shadow-lg, var(--shadow));
  }
  .head { display: flex; align-items: center; justify-content: space-between; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); border-bottom: 1px solid var(--border); }
  .title { font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .close { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .close:hover { color: var(--text); background: var(--surface-3); }
  .body { overflow-y: auto; padding: var(--sp-1) var(--sp-2); }
  .foot { padding: var(--sp-2); border-top: 1px solid var(--border); display: flex; justify-content: flex-end; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .row { display: flex; flex-direction: column; gap: 4px; padding: var(--sp-2) 2px; min-width: 0; }
  .row + .row { border-top: 1px solid var(--border); }
  .go {
    display: flex; align-items: center; gap: var(--sp-2); width: 100%; min-width: 0; border: none; background: none; padding: 2px;
    border-radius: var(--radius-sm); color: var(--text-3); cursor: pointer; font: inherit; text-align: left;
  }
  .go:hover:not(:disabled) { background: var(--surface-3); }
  .go:disabled { cursor: default; }
  .kind {
    position: relative; width: 34px; height: 34px; flex-shrink: 0; border-radius: 9px; display: inline-flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent-text-2);
  }
  .failed .kind { background: var(--danger-bg); color: var(--danger-text); }
  .dir {
    position: absolute; right: -3px; bottom: -3px; width: 15px; height: 15px; border-radius: 50%; display: flex; align-items: center; justify-content: center;
    background: var(--surface); color: var(--text-2); box-shadow: 0 0 0 1px var(--border);
  }
  .who { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chat { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error { font-size: var(--fs-2xs); color: var(--danger-text); padding: 4px 2px; }
  .retry-all {
    display: inline-flex; align-items: center; justify-content: center; gap: 6px; border: none; cursor: pointer; font: inherit;
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); padding: 8px 12px; border-radius: var(--radius-md);
    background: var(--accent-tint); color: var(--accent-text-2);
  }
  .retry-all:disabled { opacity: 0.5; cursor: default; }
  @media (pointer: coarse) { .retry-all { width: 100%; padding: 12px; } }
</style>

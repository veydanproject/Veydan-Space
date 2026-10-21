<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Someone else's QR read: on the phone by the camera, on the desktop from a
  picture (a photo or a screenshot of their code), since a desktop has no
  camera of ours. Their link with its secret is a request their device
  answers at once; a plain link of a person only opens the chat with them.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import QrScanner from '$lib/core/ui/QrScanner.svelte';
  import { readQrImage } from '$lib/core/ui/qrImage';
  import { messengerError } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { onPhone } from '../shared/phone';
  import { inviteStore, type JoinOutcome } from './inviteStore.svelte';
  import { webLoginHref } from './invites';

  interface Props {
    /** A scanned code let me in (or named a person): their chat is to be opened. */
    onchat?: (chatId: string) => void;
    /** The scanner's way by hand: the key and the code typed in a new chat. */
    onmanual?: () => void;
    /** The camera starts at once (the phone only: a file picker waits for a tap). */
    autostart?: boolean;
    /** The camera that started at once was closed without a code. */
    onclosed?: () => void;
  }
  let { onchat, onmanual, autostart = false, onclosed }: Props = $props();

  const phone = onPhone();

  let scanning = $state(false);
  let started = false;
  let reading = $state(false);
  let joined = $state<{ name: string } | null>(null);
  let scanError = $state('');
  let fileInput = $state<HTMLInputElement | null>(null);
  let destroyed = false;

  const JOIN_ERRORS = { not_contact: 'msg_inv_err_not_contact', is_me: 'msg_inv_err_is_me', blocked: 'msg_inv_err_blocked' } as const;

  function begin() {
    scanError = '';
    joined = null;
    if (phone) scanning = true;
    else fileInput?.click();
  }

  /** A code was read: a person's link with a secret goes as a request at once, without a question. */
  async function scanned(content: string) {
    scanning = false;
    started = false;
    scanError = '';
    joined = null;
    let out: JoinOutcome;
    try {
      out = await inviteStore.join(content);
    } catch (e) {
      scanError = messengerError(e);
      return;
    }
    if (!out.ok) {
      // The QR code of the sign-in page of the web cabinet: approved on the screen of the cloud, after a question.
      if (out.reason === 'web') {
        void goto(webLoginHref(out.id));
        return;
      }
      // «Добавить сервер?» answered no: nothing to say.
      if (out.reason !== 'declined') scanError = $t(JOIN_ERRORS[out.reason]);
      return;
    }
    const peer = out.pubkey;
    try {
      // The home chat, or the private one on the server of the link.
      const chat = await chatStore.openPeer(out.address);
      // A plain link of a person carries no secret: nothing went, their chat simply opens.
      if (!out.invited) {
        onchat?.(chat.id);
        return;
      }
      joined = { name: out.name || `${peer.slice(0, 8)}…` };
      // The words stay a moment before the chat takes the screen.
      setTimeout(() => { if (!destroyed) onchat?.(chat.id); }, 900);
    } catch (e) {
      scanError = messengerError(e);
    }
  }

  /** The desktop's way: the code read from a picture. */
  async function picked(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    reading = true;
    try {
      const found = await readQrImage(file);
      if (found === null) scanError = $t('msg_inv_scan_not_found');
      else await scanned(found);
    } catch (err) {
      scanError = messengerError(err);
    } finally {
      reading = false;
      if (fileInput) fileInput.value = '';
    }
  }

  onMount(() => {
    if (autostart && phone) {
      started = true;
      scanning = true;
    }
    return () => { destroyed = true; };
  });
</script>

{#if scanning}
  <QrScanner
    title={$t('msg_inv_scan_title')}
    hint={$t('msg_inv_scan_hint')}
    manual={$t('msg_inv_scan_manual')}
    words={{
      starting: $t('msg_inv_scan_starting'),
      denied: $t('msg_inv_scan_denied'),
      failed: $t('msg_inv_scan_failed'),
      retry: $t('msg_inv_scan_retry'),
      openSettings: $t('msg_inv_scan_open_settings'),
    }}
    onresult={scanned}
    onclose={() => { scanning = false; if (started) { started = false; onclosed?.(); } }}
    onmanual={onmanual ? () => { scanning = false; started = false; onmanual(); } : undefined}
  />
{/if}

<div class="scan">
  {#if !phone}
    <input bind:this={fileInput} type="file" accept="image/*" hidden onchange={picked} />
  {/if}
  <button class="btn btn-primary btn-sm" disabled={reading} onclick={begin}>
    <Icon name={reading ? 'loader' : phone ? 'scan' : 'image'} size={13} />{phone ? $t('msg_inv_scan') : $t('msg_inv_scan_image')}
  </button>

  {#if joined}
    <div class="joined" role="status">
      <span class="mark"><Icon name="check" size={22} /></span>
      <div class="joined-text"><strong>{$t('msg_inv_joined')}</strong><span>{joined.name}</span></div>
    </div>
  {/if}

  {#if scanError}<div class="error-msg">{scanError}{#if onmanual}<button class="link-btn" onclick={onmanual}>{$t('msg_inv_scan_manual')}</button>{/if}</div>{/if}
</div>

<style>
  .scan { display: flex; flex-direction: column; gap: var(--sp-3); min-width: 0; }
  .scan > .btn { align-self: flex-start; }
  .scan > .btn :global(svg) { margin-right: 2px; }
  .joined { display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-3); border-radius: var(--radius-md); background: var(--success-bg); border: 1px solid var(--success-border); color: var(--success-text); }
  .joined .mark { display: inline-flex; align-items: center; justify-content: center; width: 40px; height: 40px; border-radius: 50%; background: var(--bg); }
  .joined-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; color: var(--text); }
  .joined-text span { color: var(--text-2); font-size: var(--fs-sm); overflow-wrap: anywhere; }
  .link-btn { margin-left: 6px; padding: 0; border: 0; background: none; color: var(--accent-text-3); font: inherit; text-decoration: underline; cursor: pointer; }
  @media (pointer: coarse) {
    .scan > .btn { align-self: stretch; min-height: 40px; justify-content: center; }
  }
</style>

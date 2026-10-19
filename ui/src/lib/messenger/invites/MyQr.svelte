<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «QR»: a code to show, so a person in front of me gets into a chat
  with me without waiting in my requests. The code on the screen lives 15
  minutes and is drawn anew while it is shown, so a photo of it soon lets
  nobody in; «Постоянный» keeps one until it is revoked, for a card or a
  sign. A link to send goes for one person and a week; a code to say aloud
  for one person and a day, maybe for someone named. The same screen reads
  someone else's code (ScanQr): by the camera on the phone, from a picture
  on the desktop.
-->
<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import { contactLabel, messengerApi, messengerError, type InviteView } from '../api';
  import { messengerStore } from '../store.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { clock, stamp } from '../shared/time';
  import { inviteStore } from './inviteStore.svelte';
  import { countdown, formatCode, isPermanentQr, QR_REFRESH_AHEAD_SECS, QR_TTL_SECS, reusableQr } from './invites';
  import ScanQr from './ScanQr.svelte';
  import ServerBox from '../servers/ServerBox.svelte';
  import ServerPicker from '../servers/ServerPicker.svelte';
  import { serverStore } from '../servers/serverStore.svelte';
  import { privateLink } from '../servers/privateInvite.svelte';

  interface Props {
    /** A scanned code let me in (or named a person): their chat is to be opened. */
    onchat?: (chatId: string) => void;
    /** The scanner's way by hand: the key and the code typed in a new chat. */
    onmanual?: () => void;
    /** The camera for someone else's code starts at once (the phone only). */
    scan?: boolean;
    /** The camera that started at once was closed without a code. */
    onscanclosed?: () => void;
  }
  let { onchat, onmanual, scan = false, onscanclosed }: Props = $props();

  const nowSec = () => Math.floor(Date.now() / 1000);

  let now = $state(nowSec());
  let permanent = $state(false);
  let current = $state<InviteView | null>(null);
  let qrSrc = $state<string | null>(null);
  let drawing = $state(false);
  let error = $state('');
  /** After a failed draw the next try waits this long: a runtime that refuses is not asked every second. */
  const DRAW_RETRY_SECS = 15;
  let retryAt = 0;
  let linkNote = $state(false);
  let codeOpen = $state(false);
  let codeFor = $state<string | null>('');
  let code = $state<InviteView | null>(null);
  let codeBusy = $state(false);
  let codeCopied = $state(false);
  let destroyed = false;
  let timer: ReturnType<typeof setInterval> | null = null;

  const left = $derived(current?.expires_at != null ? current.expires_at - now : null);
  const paused = $derived(inviteStore.pausedUntil(now));
  const forOptions = $derived([
    { label: $t('msg_inv_code_for_anyone'), value: '' },
    ...messengerStore.contacts.map((c) => ({ label: contactLabel(c), value: c.pubkey })),
  ]);
  const myKey = $derived(messengerStore.identity?.npub ?? '');

  /**
   * The code of a private chat on a server of mine instead of the home one:
   * my link there with its invitation (5 people, 30 days), drawn once.
   * Whoever reads it is asked to add the server, then let straight into
   * the private chat (ScanQr → inviteStore.join).
   */
  let qrServer = $state<string | null>(null);
  let serverQr = $state<string | null>(null);
  const homeLabel = $derived(messengerStore.manifest?.mode === 'own' ? $t('msg_srv_mode') : $t('msg_srv_veydan_title'));
  const qrServerView = $derived(serverStore.byId(qrServer));
  $effect(() => {
    const s = qrServer;
    serverQr = null;
    if (!s) return;
    let live = true;
    messengerApi.servers.privateLinkQr(s)
      .then((svg) => { if (live) serverQr = `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`; })
      .catch((e) => { if (live) error = messengerError(e); });
    return () => { live = false; };
  });

  /** The QR to show: one still live of the kind chosen, else a new one. */
  async function draw() {
    if (drawing || destroyed) return;
    drawing = true;
    error = '';
    try {
      if (!inviteStore.list) await inviteStore.load();
      let v = reusableQr(inviteStore.list ?? [], permanent, nowSec());
      v ??= await inviteStore.create(permanent ? { kind: 'qr', label: '' } : { kind: 'qr', label: '', ttl_secs: QR_TTL_SECS });
      const svg = await messengerApi.links.contactQr(v.id);
      if (destroyed) return;
      current = v;
      // Shown as an image: nothing inside the picture can run.
      qrSrc = `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
    } catch (e) {
      error = messengerError(e);
      retryAt = nowSec() + DRAW_RETRY_SECS;
    } finally {
      drawing = false;
    }
  }

  function tick() {
    now = nowSec();
    // The next code is drawn before this one ends; a failed draw is tried again after a pause.
    if (now < retryAt) return;
    if (!permanent && current?.expires_at != null && current.expires_at - now <= QR_REFRESH_AHEAD_SECS) void draw();
  }

  /**
   * «Постоянный» on draws the lasting code; off revokes it, asked first,
   * and a no leaves the switch on: a permanent code that is not revoked
   * keeps admitting whatever the screen shows, so the switch says whether
   * one exists.
   */
  async function setPermanent(on: boolean) {
    if (on === permanent || drawing) return;
    if (!on) {
      const lasting = (inviteStore.list ?? []).filter((v) => isPermanentQr(v) && !v.revoked);
      if (lasting.length && !(await confirmStore.ask($t('msg_inv_revoke_confirm'), $t('msg_inv_revoke'), true))) return;
      for (const v of lasting) {
        await inviteStore.revoke(v.id);
        if (inviteStore.error) { error = inviteStore.error; return; }
      }
    }
    permanent = on;
    current = null;
    qrSrc = null;
    void draw();
  }

  /** A link for one person and a week: a new one each time, so a forwarded link lets in one more at most. */
  async function copyLink() {
    error = '';
    linkNote = false;
    try {
      if (qrServer) {
        await navigator.clipboard.writeText(await privateLink(qrServer));
        copiedToast();
        linkNote = true;
        return;
      }
      const v = await inviteStore.create({ kind: 'link', label: '' });
      if (!v.link) throw new Error('no link');
      await navigator.clipboard.writeText(v.link);
      copiedToast();
      linkNote = true;
    } catch (e) {
      error = messengerError(e);
    }
  }

  async function makeCode() {
    error = '';
    codeBusy = true;
    try {
      code = await inviteStore.create({ kind: 'code', label: '', ...(codeFor ? { for_pubkey: codeFor } : {}) });
    } catch (e) {
      error = messengerError(e);
    } finally {
      codeBusy = false;
    }
  }

  async function copyCode() {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code.secret);
      copiedToast();
      codeCopied = true;
      setTimeout(() => (codeCopied = false), 1600);
    } catch (e) {
      error = messengerError(e);
    }
  }

  onMount(() => {
    void (async () => {
      if (!inviteStore.list) await inviteStore.load();
      // A permanent code made once is the one shown, until it is revoked.
      permanent = (inviteStore.list ?? []).some((v) => isPermanentQr(v) && !v.revoked);
      await draw();
    })();
    timer = setInterval(tick, 1000);
  });
  onDestroy(() => {
    destroyed = true;
    if (timer) clearInterval(timer);
  });
</script>

<div class="myqr">
  {#if paused}
    <div class="strip warn" role="status"><Icon name="alert-triangle" size={14} />{$t('msg_inv_paused', { time: clock(paused) })}</div>
  {/if}

  <p class="hint">{$t('msg_inv_my_qr_hint')}</p>

  {#if serverStore.list?.length}
    <div class="for">
      <span class="muted small">{$t('msg_srv_qr_for')}</span>
      <ServerPicker bind:value={qrServer} none={homeLabel} label={$t('msg_srv_qr_for')} />
    </div>
  {/if}

  <div class="qrbox" aria-busy={drawing}>
    {#if qrServer}
      {#if serverQr}<img class="qr" src={serverQr} alt={$t('msg_inv_qr_alt')} />
      {:else}<div class="qr placeholder"><Icon name={error ? 'alert-triangle' : 'loader'} size={22} /></div>{/if}
    {:else if qrSrc}
      <img class="qr" src={qrSrc} alt={$t('msg_inv_qr_alt')} />
    {:else}
      <div class="qr placeholder"><Icon name={error ? 'alert-triangle' : 'loader'} size={22} /></div>
    {/if}
  </div>

  {#if qrServer}
    <p class="muted center">{$t('msg_srv_qr_hint')}</p>
    {#if qrServerView}<ServerBox server={{ ...qrServerView, is_veydan: false }} />{/if}
  {:else}
  <div class="under">
    {#if permanent}
      <span class="muted">{$t('msg_inv_qr_permanent_hint')}</span>
      {#if current && isPermanentQr(current)}
        <button class="btn btn-ghost btn-sm danger" disabled={!!inviteStore.revoking} onclick={() => void setPermanent(false)}>{$t('msg_inv_revoke')}</button>
      {/if}
    {:else if left !== null}
      <span class="muted" aria-live="off">{$t('msg_inv_qr_left', { time: countdown(left) })}</span>
    {/if}
  </div>

  <div class="line">
    <span class="line-text">
      <span>{$t('msg_inv_qr_permanent')}</span>
      {#if !permanent}<span class="muted small">{$t('msg_inv_qr_timed_hint')}</span>{/if}
    </span>
    <button class="toggle" class:on={permanent} disabled={drawing || !!inviteStore.revoking} role="switch" aria-checked={permanent} aria-label={$t('msg_inv_qr_permanent')}
      onclick={() => void setPermanent(!permanent)}></button>
  </div>
  {/if}

  {#if myKey}
    <div class="key">
      <span class="muted small">{$t('msg_inv_my_key')}</span>
      <code>{myKey}</code>
    </div>
  {/if}

  <div class="actions">
    <button class="btn btn-ghost btn-sm" onclick={copyLink}><Icon name={linkNote ? 'check' : 'link'} size={13} />{$t('msg_inv_copy_link')}</button>
    {#if !qrServer}
    <button class="btn btn-ghost btn-sm" class:active={codeOpen} aria-expanded={codeOpen} onclick={() => (codeOpen = !codeOpen)}>
      <Icon name="key" size={13} />{$t('msg_inv_code_24h')}
    </button>
    {/if}
  </div>
  {#if linkNote}<p class="muted small" role="status">{$t(qrServer ? 'msg_srv_invite_copied' : 'msg_inv_link_copied')}</p>{/if}

  {#if codeOpen && !qrServer}
    <div class="code-box">
      {#if code}
        <div class="code-row">
          <span class="code mono">{formatCode(code.secret)}</span>
          <button class="btn btn-ghost btn-sm" onclick={copyCode}><Icon name={codeCopied ? 'check' : 'copy'} size={13} />{$t('msg_copy')}</button>
        </div>
        {#if code.expires_at != null}<p class="muted small">{$t('msg_inv_code_until', { time: stamp(code.expires_at) })}</p>{/if}
        <p class="muted small">{$t('msg_inv_code_hint')}</p>
        <button class="btn btn-ghost btn-sm self-start" onclick={() => (code = null)}>{$t('msg_inv_code_another')}</button>
      {:else}
        <span class="label">{$t('msg_inv_code_for')}</span>
        <CustomSelect options={forOptions} bind:value={codeFor} />
        <button class="btn btn-primary btn-sm self-start" disabled={codeBusy} onclick={makeCode}>{$t('msg_inv_create_code')}</button>
      {/if}
    </div>
  {/if}

  {#if error}<div class="error-msg">{error}</div>{/if}

  <ScanQr {onchat} {onmanual} autostart={scan} onclosed={onscanclosed} />
</div>

<style>
  .myqr { display: flex; flex-direction: column; gap: var(--sp-3); min-width: 0; }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .muted { color: var(--text-2); font-size: var(--fs-xs); line-height: 1.4; }
  .small { font-size: var(--fs-xs); }
  p.muted { margin: 0; }
  .strip {
    display: flex; align-items: center; gap: 6px; padding: 8px var(--sp-3); border-radius: var(--radius-sm); font-size: var(--fs-xs);
    color: var(--warn-text); background: var(--warn-bg); border: 1px solid var(--warn-border); line-height: 1.4;
  }
  .for { display: flex; flex-direction: column; gap: 6px; }
  .center { text-align: center; }
  .qrbox { display: flex; justify-content: center; }
  /* A code is read by a camera: always dark on light, whatever the theme. */
  .qr { width: min(260px, 100%); aspect-ratio: 1; height: auto; background: #fff; border-radius: var(--radius-md); padding: 8px; image-rendering: pixelated; }
  .placeholder { display: flex; align-items: center; justify-content: center; color: var(--text-3); background: var(--surface-2); border: 1px solid var(--border); }
  .under { display: flex; align-items: center; justify-content: center; gap: var(--sp-2); flex-wrap: wrap; text-align: center; min-height: 28px; font-variant-numeric: tabular-nums; }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); }
  .line-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .key { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .key code {
    font-size: var(--fs-2xs); color: var(--text-2); background: var(--surface-2); border: 1px solid var(--border);
    border-radius: var(--radius-sm); padding: 6px 8px; overflow-wrap: anywhere; user-select: all;
  }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; }
  .actions .btn :global(svg) { margin-right: 2px; }
  .btn.active { background: var(--surface-3); }
  .danger { color: var(--danger-text); }
  .code-box { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2); }
  .code-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); flex-wrap: wrap; }
  .code { font-size: var(--fs-2xl); font-weight: var(--fw-bold); letter-spacing: 2px; user-select: all; font-variant-numeric: tabular-nums; }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .self-start { align-self: flex-start; }
  @media (pointer: coarse) {
    .actions .btn { flex: 1 1 auto; min-height: 40px; justify-content: center; }
  }
</style>

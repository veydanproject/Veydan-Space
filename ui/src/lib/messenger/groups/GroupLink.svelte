<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The link of a group: copy it, show it as a QR code, replace it. A
  public link lets anyone in; a private one only says whom to ask.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerApi, messengerError, type MessengerGroup } from '../api';
  import { groupStore } from './groupStore.svelte';
  import { isManager } from './permissions';
  import { confirmStore } from '../shared/confirm.svelte';
  import ShareDialog from '../content/ShareDialog.svelte';

  interface Props { group: MessengerGroup }
  let { group }: Props = $props();

  let copied = $state(false);
  let sharing = $state(false);
  let qr = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');

  // A code drawn for one link says nothing about the next one.
  let drawnFor = '';
  $effect(() => {
    if ((group.link ?? '') !== drawnFor) { drawnFor = group.link ?? ''; qr = null; }
  });

  async function copy() {
    if (!group.link) return;
    try {
      await navigator.clipboard.writeText(group.link);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch (e) { error = messengerError(e); }
  }

  async function toggleQr() {
    if (qr) { qr = null; return; }
    error = ''; busy = true;
    try {
      const svg = await messengerApi.groups.linkQr(group.id);
      // Shown as an image: nothing inside the picture can run.
      qr = `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  async function rotate() {
    if (!(await confirmStore.ask($t('msg_group_link_rotate_confirm'), $t('msg_group_link_rotate'), true))) return;
    error = ''; busy = true;
    try { await groupStore.rotateLink(group.id); }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }
</script>

{#if group.link}
  <section class="block">
    <div class="label">{$t('msg_group_link')}</div>
    <p class="hint">{$t(group.kind === 'public' ? 'msg_group_link_hint_public' : 'msg_group_link_hint_private')}</p>
    <code class="link">{group.link}</code>
    <div class="row">
      <button class="btn btn-ghost btn-sm" onclick={copy}><Icon name={copied ? 'check' : 'copy'} size={13} />{copied ? $t('msg_group_link_copied') : $t('msg_copy')}</button>
      <button class="btn btn-ghost btn-sm" onclick={() => (sharing = true)}><Icon name="send" size={13} />{$t('msg_share')}</button>
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={toggleQr}><Icon name="qr-code" size={13} />{qr ? $t('msg_group_qr_hide') : $t('msg_group_qr_show')}</button>
      {#if group.kind === 'public' && isManager(group)}
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={rotate}><Icon name="refresh-cw" size={13} />{$t('msg_group_link_rotate')}</button>
      {/if}
    </div>
    {#if qr}<img class="qr" src={qr} alt={$t('msg_group_qr_alt', { name: group.name })} />{/if}
    {#if error}<div class="error-msg">{error}</div>{/if}
  </section>
{/if}

{#if group.link}<ShareDialog bind:open={sharing} link={group.link} exclude={group.chat_id} />{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.45; }
  .link {
    font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-2); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 6px 8px; overflow-wrap: anywhere;
    max-height: 66px; overflow-y: auto; user-select: all;
  }
  .row { display: flex; flex-wrap: wrap; gap: 6px; }
  .danger { color: var(--danger-text); }
  /* A code is read by a camera: always dark on light, whatever the theme. */
  .qr { align-self: center; width: min(240px, 100%); height: auto; background: #fff; border-radius: var(--radius-md); padding: 6px; image-rendering: pixelated; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The link of a group: copy it, show it as a QR code, replace it. A
  public link lets anyone in; a private one only says whom to ask.
-->
<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerApi, messengerError, type MessengerGroup } from '../api';
  import { groupStore } from './groupStore.svelte';
  import { isManager } from './permissions';
  import { confirmStore } from '../shared/confirm.svelte';
  import ShareDialog from '../content/ShareDialog.svelte';
  import ShareOutSheet from '../content/ShareOutSheet.svelte';
  import { groupShareItem } from '../content/shareOut';
  import { qrSrc, saveQr, saveQrError } from '../content/saveQr';

  interface Props {
    group: MessengerGroup;
    /** Its title above it; none on a page of its own, whose head says it. */
    heading?: boolean;
  }
  let { group, heading = true }: Props = $props();

  let copied = $state(false);
  let sharing = $state(false);
  let sharingOut = $state(false);
  let qr = $state<string | null>(null);
  let qrSvg = '';
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
      copiedToast();
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch (e) { error = messengerError(e); }
  }

  async function toggleQr() {
    if (qr) { qr = null; return; }
    error = ''; busy = true;
    try {
      qrSvg = await messengerApi.groups.linkQr(group.id);
      // Shown as an image: nothing inside the picture can run.
      qr = qrSrc(qrSvg);
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  /** The code as a picture, after the warning: whoever sees it gets in, as with the link. */
  async function save() {
    if (!qr || !(await confirmStore.ask($t('msg_qr_save_key_warn'), $t('msg_qr_save')))) return;
    error = ''; busy = true;
    try { await saveQr(qrSvg, 'veydan-group-qr'); }
    catch (e) { error = saveQrError(e, messengerError); }
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
    {#if heading}<div class="label">{$t('msg_group_link')}</div>{/if}
    <div class="link-box">
      <p class="hint">{$t(group.kind === 'public' ? 'msg_group_link_hint_public' : 'msg_group_link_hint_private')}</p>
      <code class="link">{group.link}</code>
      <div class="grid">
        <button class="btn btn-ghost btn-sm" onclick={copy}><Icon name={copied ? 'check' : 'copy'} size={18} />{copied ? $t('msg_group_link_copied') : $t('msg_copy')}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (sharing = true)}><Icon name="send" size={18} />{$t('msg_share')}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (sharingOut = true)}><Icon name="upload" size={18} />{$t('msg_share_out_open')}</button>
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={toggleQr}><Icon name="qr-code" size={18} />{qr ? $t('msg_group_qr_hide') : $t('msg_group_qr_show')}</button>
        {#if group.kind === 'public' && isManager(group)}
          <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={rotate}><Icon name="refresh-cw" size={18} />{$t('msg_group_link_rotate')}</button>
        {/if}
      </div>
      {#if qr}
        <img class="qr" src={qr} alt={$t('msg_group_qr_alt', { name: group.name })} />
        <button class="btn btn-ghost btn-sm save" disabled={busy} onclick={save}><Icon name="download" size={18} />{$t('msg_qr_save')}</button>
      {/if}
      {#if error}<div class="error-msg">{error}</div>{/if}
    </div>
  </section>
{/if}

{#if group.link}
  <ShareDialog bind:open={sharing} link={group.link} exclude={group.chat_id} />
  <ShareOutSheet bind:open={sharingOut} item={groupShareItem(group.link)} qr={() => messengerApi.groups.linkQr(group.id)} />
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-3); }
  .label { display: flex; align-items: center; min-height: 24px; font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .link-box { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-4); border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.5; }
  .link {
    font-size: var(--fs-xs); line-height: 1.5; color: var(--text-2); background: var(--surface-3);
    border-radius: var(--radius); padding: var(--sp-2) var(--sp-3); overflow-wrap: anywhere; word-break: break-all;
    max-height: 96px; overflow-y: auto; user-select: all;
  }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-2); }
  .grid .btn:last-child:nth-child(odd) { grid-column: 1 / -1; }
  .grid .btn { min-height: var(--control-h-sm); min-width: 0; padding-inline: var(--sp-2); white-space: normal; text-wrap: balance; text-align: center; line-height: 1.25; }
  .danger { color: var(--danger-text); }
  /* A code is read by a camera: always dark on light, whatever the theme. */
  .qr { align-self: center; width: min(240px, 100%); height: auto; background: #fff; border-radius: var(--radius-md); padding: var(--sp-2); image-rendering: pixelated; }
  .save { align-self: center; }
  @media (pointer: coarse) { .grid .btn, .save { min-height: var(--control-h-lg); } }
</style>

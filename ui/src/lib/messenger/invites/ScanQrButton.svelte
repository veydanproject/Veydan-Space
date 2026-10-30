<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The way to someone else's QR, beside «QR»: the camera at once on the phone, a dialog that reads a picture on the desktop. -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { openChat } from '../content/actions';
  import { onPhone } from '../shared/phone';
  import { qrHref } from '../mobile/routes';
  import ScanQr from './ScanQr.svelte';

  interface Props {
    /** Only the icon (a header's action), else the icon and the words. */
    icon?: boolean;
    disabled?: boolean;
  }
  let { icon = false, disabled = false }: Props = $props();

  const phone = onPhone();
  let open = $state(false);

  function go() {
    if (phone) void goto(qrHref(true));
    else open = true;
  }

  function opened(chatId: string) {
    open = false;
    openChat(chatId).catch(() => {});
  }
</script>

{#if icon}
  <button class="ibtn" {disabled} onclick={go} aria-label={$t('msg_inv_scan_qr')} title={$t('msg_inv_scan_qr')}><Icon name="scan" size={phone ? 21 : 16} /></button>
{:else}
  <button class="btn btn-ghost btn-sm" {disabled} onclick={go}><Icon name="scan" size={13} />{$t('msg_inv_scan_qr')}</button>
{/if}

{#if !phone}
  <Dialog bind:open title={$t('msg_inv_scan_title')} width="min(420px, calc(100vw - 24px))">
    {#if open}<p class="hint">{$t('msg_inv_scan_image_hint')}</p>{/if}
    <!-- «Выбрать картинку» (and what it read) at the window's foot (the owner's rule). -->
    {#snippet footer()}
      {#if open}<div class="foot"><ScanQr onchat={opened} /></div>{/if}
    {/snippet}
  </Dialog>
{/if}

<style>
  .btn :global(svg) { margin-right: 2px; }
  .foot { flex: 1; min-width: 0; }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
</style>

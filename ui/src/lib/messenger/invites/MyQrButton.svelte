<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The way to «Мой QR»: a screen of its own on the phone (where it also scans), a dialog on the desktop (it only shows). -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { onPhone } from '../shared/phone';
  import { qrHref } from '../mobile/routes';
  import MyQr from './MyQr.svelte';

  interface Props {
    /** Only the icon (a header's action), else the icon and the words. */
    icon?: boolean;
    disabled?: boolean;
  }
  let { icon = false, disabled = false }: Props = $props();

  const phone = onPhone();
  let open = $state(false);

  function go() {
    if (phone) void goto(qrHref());
    else open = true;
  }
</script>

{#if icon}
  <button class="ibtn" {disabled} onclick={go} aria-label={$t('msg_inv_my_qr')} title={$t('msg_inv_my_qr')}><Icon name="qr-code" size={phone ? 21 : 16} /></button>
{:else}
  <button class="btn btn-ghost btn-sm" {disabled} onclick={go}><Icon name="qr-code" size={13} />{$t('msg_inv_my_qr')}</button>
{/if}

{#if !phone}
  <Dialog bind:open title={$t('msg_inv_my_qr')} width="min(420px, calc(100vw - 24px))">
    {#if open}<MyQr />{/if}
  </Dialog>
{/if}

<style>
  .btn :global(svg) { margin-right: 2px; }
</style>

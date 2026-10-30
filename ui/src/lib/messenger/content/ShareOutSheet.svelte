<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Поделиться» outside Veydan (ROO-173): the ways a text with a link goes
  into another app (shareOut.ts), the link copied, or the link as a QR on
  the screen. Knows nothing of what is shared: the caller gives the item and,
  for «Показать QR», how its code is drawn. Sending inside Veydan is
  ShareDialog.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { messengerError } from '../api';
  import { shareOut, shareOutWays, type ShareOutItem, type ShareOutWay } from './shareOut';

  interface Props {
    open: boolean;
    /** What is shared; the sheet shows nothing while it is null. */
    item: ShareOutItem | null;
    title?: string;
    /** The QR of the link as SVG; «Показать QR» is not offered without it. */
    qr?: () => Promise<string>;
  }
  let { open = $bindable(), item, title, qr }: Props = $props();

  const ICONS: Record<ShareOutWay, string> = { send: 'share', sms: 'smartphone', mail: 'mail', copy: 'copy', qr: 'qr-code' };
  const LABELS = {
    send: 'msg_share_out_send',
    sms: 'msg_share_out_sms',
    mail: 'msg_share_out_mail',
    copy: 'msg_share_out_copy',
    qr: 'msg_share_out_qr',
  } as const satisfies Record<ShareOutWay, string>;
  const ways = $derived(shareOutWays().filter((w) => w !== 'qr' || !!qr));

  let busy = $state(false);
  let error = $state('');
  let qrSrc = $state<string | null>(null);
  let showQr = $state(false);

  // One opening of one item; an answer that comes after the sheet closed or
  // its item changed is of the one before (a QR of another link) and is dropped.
  let opening = 0;

  // Each opening starts at the list: the QR and the error of an item before are gone.
  $effect(() => {
    void open;
    void item;
    opening++;
    error = '';
    showQr = false;
    qrSrc = null;
    busy = false;
  });

  async function pick(way: ShareOutWay) {
    if (!item || busy) return;
    const mine = opening;
    error = '';
    busy = true;
    try {
      if (way === 'qr') {
        const svg = await qr!();
        if (mine !== opening) return;
        // Shown as an image: nothing inside the picture can run.
        qrSrc = `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
        showQr = true;
        return;
      }
      await shareOut(item, way);
      // The other app has it now; the copied link stays in the sheet, for another way.
      if (way !== 'copy' && mine === opening) open = false;
    } catch (e) {
      if (mine !== opening) return;
      const m = messengerError(e);
      error = m.includes('share_no_app') ? $t('msg_share_out_no_app') : m;
    } finally {
      if (mine === opening) busy = false;
    }
  }
</script>

<Dialog bind:open title={title ?? $t('msg_share_out_title')} width="min(380px, calc(100vw - 24px))">
  {#if item}
    {#if showQr && qrSrc}
      <div class="qr">
        <img src={qrSrc} alt={$t('msg_share_out_qr')} />
        <button class="btn btn-ghost btn-sm" onclick={() => (showQr = false)}>
          <Icon name="arrow-left" size={13} />{$t('msg_back')}
        </button>
      </div>
    {:else}
      <ul class="ways">
        {#each ways as way (way)}
          <li>
            <button type="button" class="way" disabled={busy} onclick={() => pick(way)}>
              <span class="way-icon"><Icon name={ICONS[way]} size={18} /></span>
              <span>{$t(LABELS[way])}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
  {/if}
</Dialog>

<style>
  .ways { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .way {
    width: 100%; display: flex; align-items: center; gap: var(--sp-3); padding: 8px 6px; min-height: 44px;
    border: none; border-radius: var(--radius-sm); background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-sm); color: var(--text); text-align: left;
  }
  .way:hover:not(:disabled) { background: var(--surface-3); }
  .way:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .way:disabled { opacity: 0.6; cursor: default; }
  .way-icon {
    width: 32px; height: 32px; flex-shrink: 0; border-radius: var(--radius-sm); background: var(--surface-3); color: var(--text-2);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .qr { display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); }
  .qr img { width: min(260px, 100%); aspect-ratio: 1; height: auto; background: #fff; border-radius: var(--radius-md); padding: 8px; image-rendering: pixelated; }
  .btn :global(svg) { margin-right: 2px; }
  @media (pointer: coarse) {
    .way { min-height: 48px; }
  }
</style>

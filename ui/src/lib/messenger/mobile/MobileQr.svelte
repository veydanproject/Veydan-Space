<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's «QR»: my code to show, the camera for someone else's, and
  the key and code by hand when the camera cannot. «Сканировать QR» comes
  here with `?scan=1`: the camera is on at once, and closed without a code
  it goes back where it was asked from.
-->
<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import NewChatDialog from '../chats/NewChatDialog.svelte';
  import MyQr from '../invites/MyQr.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, chatHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  let manual = $state(false);
  /** MyQr's actions, kept in the bottom bar of the page. */
  let foot = $state<Snippet>();
  const scan = page.url.searchParams.get('scan') === '1';

  onMount(() => {
    messengerStore.ensureLoaded().catch(() => {});
    messengerStore.startListeners().catch(() => {});
  });
</script>

<!-- The middle scrolls; the actions stay in a bar at the bottom of the screen (the owner's rule). -->
<MobileFrame title={$t('msg_inv_my_qr')} onback={() => void leave(BASE)} scroll={false}>
  <div class="mid">
    <div class="pad">
      <MyQr bind:foot onchat={(id) => goto(chatHref(id))} onmanual={() => (manual = true)} {scan} onscanclosed={() => void leave(BASE)} />
    </div>
  </div>
  {#if foot}<div class="foot">{@render foot()}</div>{/if}
</MobileFrame>

<NewChatDialog bind:open={manual} onopened={() => { if (chatStore.activeId) goto(chatHref(chatStore.activeId)); }} />

<style>
  /* The frame's own scroll and padding, now of the middle alone. */
  .mid { flex: 1; min-height: 0; overflow-y: auto; -webkit-overflow-scrolling: touch; padding: var(--sp-3); }
  .pad { padding: var(--sp-4); }
  /* Under the scroll, the content's edges kept (--sp-3 + --sp-4); the home indicator's inset under it. */
  .foot {
    flex-shrink: 0; display: flex;
    padding: var(--sp-3) calc(var(--sp-3) + var(--sp-4)) calc(var(--sp-3) + var(--sab, 0px));
    background: var(--m-nav, var(--glass-bar)); border-top: 1px solid var(--border);
    -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
  }
</style>

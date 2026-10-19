<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Частный сервер» (serverInfo.show): what «ⓘ» beside a private chat, a
  group or a card of a server opens. A private server is run by a company
  or a person, not by Veydan, which has nothing to do with it: whoever
  talks there trusts its owner. Then every server a chat there goes
  through (ServerRows). Mount once per page, beside ServerAcceptDialog.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import ServerRows from './ServerRows.svelte';
  import { hostOf, serverInfo } from './serverAccept.svelte';

  const s = $derived(serverInfo.current);
  const veydan = $derived(!!s?.is_veydan);
  const name = $derived(s ? s.name?.trim() || hostOf(s.relay) : '');
</script>

<Dialog open={s !== null} title={$t(veydan ? 'msg_srv_info_title_veydan' : 'msg_srv_info_title')} width="min(400px, calc(100vw - 24px))" onclose={() => serverInfo.close()}>
  {#if s}
    <div class="body">
      <div class="head">
        <Icon name={veydan ? 'shield-check' : 'server-2'} size={18} />
        <span class="name">{veydan ? $t('msg_srv_veydan_name') : name}</span>
      </div>
      <p class="text">{$t(veydan ? 'msg_srv_info_text_veydan' : 'msg_srv_info_text')}</p>
      <div class="box"><ServerRows server={s} /></div>
    </div>
  {/if}
  {#snippet footer()}
    <button type="button" class="btn btn-primary" onclick={() => serverInfo.close()}>{$t('msg_srv_info_ok')}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .head { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; color: var(--text-2); }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .text { margin: 0; font-size: var(--fs-sm); color: var(--text-body); line-height: 1.5; }
  .box { padding: var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--surface-2); }
</style>

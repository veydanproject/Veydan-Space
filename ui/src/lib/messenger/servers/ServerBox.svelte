<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The server a card is on, as a box of lines: «Сервер — Частный · S1 ⓘ»
  first, then what a chat there goes through (ServerRows). The card of a
  person on a server, of a group on one, of a server itself, «Новый чат»
  and «Новая группа» show the same box. «ⓘ» opens «Частный сервер»
  (ServerInfoDialog): who runs it.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ServerRows from './ServerRows.svelte';
  import { hostOf, serverInfo, type ServerParts } from './serverAccept.svelte';

  interface Props {
    server: ServerParts;
    /** Its name after «Частный»; a card that names it above says it once. */
    named?: boolean;
  }
  let { server, named = true }: Props = $props();

  const veydan = $derived(!!server.is_veydan);
  const name = $derived(server.name?.trim() || hostOf(server.relay));
  const what = $derived(veydan ? $t('msg_srv_veydan_name') : named ? `${$t('msg_srv_label')} · ${name}` : $t('msg_srv_label'));
</script>

<div class="srv-box">
  <div class="head">
    <span class="label">{$t('msg_srv_row_server')}</span>
    <button type="button" class="what" title={$t('msg_srv_info_more')} aria-label={$t('msg_srv_info_more')}
      onclick={(e) => { e.stopPropagation(); serverInfo.show(server); }}>
      <Icon name={veydan ? 'shield-check' : 'server-2'} size={12} /><span class="text">{what}</span><Icon name="info" size={12} />
    </button>
  </div>
  <ServerRows {server} />
</div>

<style>
  .srv-box {
    display: flex; flex-direction: column; gap: 3px;
    padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-sm); background: var(--surface-2);
  }
  .head { display: flex; align-items: baseline; gap: var(--sp-2); min-width: 0; font-size: var(--fs-xs); line-height: 1.4; }
  .label { flex-shrink: 0; color: var(--text-3); }
  .what {
    display: inline-flex; align-items: center; gap: 4px; min-width: 0; margin-inline-start: auto;
    padding: 0; border: 0; background: none; cursor: pointer;
    font: inherit; font-weight: var(--fw-semibold); color: var(--text);
  }
  .what :global(svg) { flex-shrink: 0; color: var(--text-3); }
  .what .text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .what:hover :global(svg) { color: var(--text); }
  .what:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; border-radius: 4px; }
</style>

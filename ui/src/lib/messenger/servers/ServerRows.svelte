<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What a server is made of, a line each: «Сообщения», «Файлы», «Звонки»,
  «Уведомления», the name on the left and where on the right. The card of
  a server, «Добавить сервер?» and «Частный сервер» show the same lines.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { serverRows, type ServerParts } from './serverAccept.svelte';

  interface Props { server: ServerParts }
  let { server }: Props = $props();

  const rows = $derived(serverRows(server, (key, params) => $t(key as 'msg_you', params)));
</script>

<dl class="rows">
  {#each rows as row (row.label)}
    <div class="row">
      <dt>{row.label}</dt>
      <dd class:none={row.none} title={row.value}>{row.value}</dd>
    </div>
  {/each}
</dl>

<style>
  .rows { margin: 0; display: flex; flex-direction: column; gap: 3px; font-size: var(--fs-xs); line-height: 1.4; }
  .row { display: flex; align-items: baseline; gap: var(--sp-2); min-width: 0; }
  dt { flex-shrink: 0; color: var(--text-3); }
  dd {
    margin: 0 0 0 auto; min-width: 0; color: var(--text-2); text-align: end;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  dd.none { color: var(--text-3); }
</style>

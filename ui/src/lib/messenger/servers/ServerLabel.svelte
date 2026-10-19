<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Частный»: the label of a chat or a group on a private server of mine,
  beside its name in the list, in its header and on the page of the
  person. A short neutral chip of the kit, the same width whatever the
  server is called: it says the chat goes through a private server and no
  more. With `info` (the header, the page of the person) it is a button:
  «Частный сервер» says who runs it and every server the chat goes
  through (ServerInfoDialog).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { serverStore } from './serverStore.svelte';
  import { serverInfo } from './serverAccept.svelte';

  interface Props {
    /** The private server (`server_id` of the chat). */
    server: string;
    size?: 'sm' | 'md';
    /** A button that opens «Частный сервер». */
    info?: boolean;
  }
  let { server, size = 'sm', info = false }: Props = $props();

  const known = $derived(serverStore.byId(server));
  const name = $derived(serverStore.nameOf(server));
  const hint = $derived($t('msg_srv_label_hint', { name: name || server }));

  function open(e: MouseEvent) {
    // In a header the label sits on the face that opens the panel.
    e.stopPropagation();
    if (known) serverInfo.show({ ...known, is_veydan: false });
  }
</script>

{#if info && known}
  <button type="button" class="srv-label btn-like" class:md={size === 'md'} title={hint} aria-label={hint} onclick={open}>
    <Icon name="server-2" size={size === 'md' ? 12 : 11} /><span class="text">{$t('msg_srv_label')}</span><Icon name="info" size={size === 'md' ? 12 : 11} />
  </button>
{:else}
  <span class="srv-label" class:md={size === 'md'} title={hint}>
    <Icon name="server-2" size={size === 'md' ? 12 : 11} /><span class="text">{$t('msg_srv_label')}</span>
  </span>
{/if}

<style>
  .srv-label {
    display: inline-flex; align-items: center; gap: 3px; flex-shrink: 0;
    padding: 0 6px; border-radius: var(--radius-pill); border: 1px solid var(--border);
    background: var(--surface-2); color: var(--text-2);
    font: inherit; font-size: var(--fs-2xs); font-weight: var(--fw-semibold); line-height: 16px; white-space: nowrap;
  }
  .srv-label :global(svg) { flex-shrink: 0; }
  .srv-label.md { padding: 1px 8px; font-size: var(--fs-xs); line-height: 18px; }
  .btn-like { cursor: pointer; }
  .btn-like:hover { border-color: var(--border-strong); color: var(--text); }
  .btn-like:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 1px; }
</style>

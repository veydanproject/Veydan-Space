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
  `icon` (a row of the list, the head on a phone): the server icon alone,
  like the kind marks of a row, «Частный сервер …» in its title and aria-label, so
  the name keeps the whole line.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { serverStore } from './serverStore.svelte';
  import { serverInfo } from './serverAccept.svelte';

  interface Props {
    /** The private server (`server_id` of the chat). */
    server: string;
    size?: 'icon' | 'sm' | 'md';
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

{#if size === 'icon'}
  {#if info && known}
    <button type="button" class="srv-mark btn-like" title={hint} aria-label={hint} onclick={open}><Icon name="server-2" size={16} /></button>
  {:else}
    <span class="srv-mark" role="img" title={hint} aria-label={hint}><Icon name="server-2" size={16} /></span>
  {/if}
{:else if info && known}
  <button type="button" class="srv-label btn-like" class:md={size === 'md'} title={hint} aria-label={hint} onclick={open}>
    <Icon name="server-2" size={12} /><span class="text">{$t('msg_srv_label')}</span><Icon name="info" size={12} />
  </button>
{:else}
  <span class="srv-label" class:md={size === 'md'} title={hint}>
    <Icon name="server-2" size={12} /><span class="text">{$t('msg_srv_label')}</span>
  </span>
{/if}

<style>
  /* The small one is the tag of the pills (a row of the list, a head): 20 high, surface-2, fs-xs. */
  .srv-label {
    display: inline-flex; align-items: center; gap: var(--sp-1); flex-shrink: 0;
    height: 20px; padding: 0 var(--sp-2); border-radius: var(--radius-pill); border: 1px solid var(--border);
    background: var(--surface-2); color: var(--text-2);
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); line-height: 18px; white-space: nowrap;
  }
  .srv-label :global(svg) { flex-shrink: 0; }
  .srv-label.md { height: auto; gap: 3px; padding: 1px 8px; line-height: 18px; }
  /* The icon one: a mark of the row (16, the colour of the chip), a 20 square when it is a button. */
  .srv-mark { flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; color: var(--text-2); }
  /* A 20 square that answers a touch over 40: the hit area is a ring of 10 around it. */
  button.srv-mark { position: relative; width: var(--sp-5); height: var(--sp-5); padding: 0; border: none; border-radius: var(--radius-sm); background: none; font: inherit; }
  button.srv-mark::before { content: ''; position: absolute; inset: calc(var(--sp-5) / -2); }
  button.srv-mark:hover { color: var(--text); }
  .btn-like { cursor: pointer; }
  .btn-like:hover { border-color: var(--border-strong); color: var(--text); }
  .btn-like:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 1px; }
</style>

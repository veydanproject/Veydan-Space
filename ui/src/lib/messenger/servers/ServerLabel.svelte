<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Частный · S1»: the label of a chat or a group that lives on a private
  server of mine, beside its name in the list, in its header and on the
  page of the person. A neutral chip of the kit: it tells where the chat
  lives, it is not a state that calls. A server this device does not name
  yet (the list is being read) is «Частный» alone.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { serverStore } from './serverStore.svelte';

  interface Props {
    /** The private server (`server_id` of the chat). */
    server: string;
    size?: 'sm' | 'md';
  }
  let { server, size = 'sm' }: Props = $props();

  const name = $derived(serverStore.nameOf(server));
  const text = $derived(name ? $t('msg_srv_label', { name }) : $t('msg_srv_label_bare'));
</script>

<span class="srv-label" class:md={size === 'md'} title={$t('msg_srv_label_hint', { name: name || server })}>
  <Icon name="server-2" size={size === 'md' ? 12 : 11} /><span class="text">{text}</span>
</span>

<style>
  .srv-label {
    display: inline-flex; align-items: center; gap: 3px; flex-shrink: 1; min-width: 0; max-width: 50%;
    padding: 0 6px; border-radius: var(--radius-pill); border: 1px solid var(--border);
    background: var(--surface-2); color: var(--text-2);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); line-height: 16px; white-space: nowrap;
  }
  .srv-label :global(svg) { flex-shrink: 0; }
  .srv-label.md { max-width: 100%; padding: 1px 8px; font-size: var(--fs-xs); line-height: 18px; }
  .text { overflow: hidden; text-overflow: ellipsis; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Return to the call» on a phone (design 15): a call (or a group call's
  room) left for another screen stays one tap away. A row of the screen's
  flow under its head, 44 high, the screen's width less 12 on each side:
  the phone, «Return to the call», the name, the time at the right; the
  content goes down under it, the head and the search bar stay clear.
  `fixed`: the shell's own one (CallWatcher), for a screen with no row of
  its own; an inline one counts itself in `callReturn.hosts` meanwhile.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { callHref, groupCallHref } from '../mobile/routes';
  import { callStore } from './callStore.svelte';
  import { groupCallStore } from './groupCallStore.svelte';
  import { groupStore } from '../groups/groupStore.svelte';
  import { callPeer } from './peer';
  import { statusText } from './status';
  import { callReturn } from './returnHosts.svelte';

  let { fixed = false }: { fixed?: boolean } = $props();

  onMount(() => {
    if (fixed) return;
    callReturn.hosts += 1;
    return () => { callReturn.hosts -= 1; };
  });

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(messengerStore.visible && callStore.call && page.url.pathname !== callHref() ? callStore.call : null);
  const room = $derived(messengerStore.visible && !call && groupCallStore.call && page.url.pathname !== groupCallHref() ? groupCallStore.call : null);
  const row = $derived(
    call ? { href: callHref(), ringing: call.phase === 'incoming', name: callPeer(call).name, status: statusText(call, null, callStore.now, tr) }
      : room ? { href: groupCallHref(), ringing: false, name: groupStore.groups[room.group_id]?.name ?? '', status: groupCallStore.statusText(tr) }
      : null,
  );
</script>

{#if row}
  <div class="call-return" class:fixed>
    <button type="button" class="go" class:ringing={row.ringing} onclick={() => goto(row.href)}>
      <Icon name={row.ringing ? 'phone-incoming' : 'phone'} size={18} />
      <span class="what">{$t('msg_call_return')}</span>
      <span class="who">{row.name}</span>
      <span class="time">{row.status}</span>
    </button>
  </div>
{/if}

<style>
  .call-return { flex-shrink: 0; position: relative; z-index: calc(var(--z-base) + 2); padding: var(--sp-2) var(--sp-3) 0; }
  /* A screen with no row of its own (another product's): just under its head, as before. */
  .call-return.fixed { position: fixed; z-index: calc(var(--z-panel) + 25); top: calc(var(--sat, 0px) + 56px); left: 0; right: 0; }
  .go {
    display: flex; align-items: center; gap: var(--sp-2); width: 100%; height: 44px; padding: 0 var(--sp-3);
    border: 1px solid var(--pane-strong-border); border-radius: var(--radius); cursor: pointer;
    background: color-mix(in srgb, var(--success-bg) 45%, var(--pane-strong)); color: var(--text);
    box-shadow: var(--pane-highlight), var(--pane-shadow); -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter);
    font: inherit; font-size: var(--fs-sm); text-align: left;
  }
  .go:active { filter: brightness(1.08); }
  .go > :global(svg), .what { flex-shrink: 0; color: var(--success-text); }
  .go.ringing > :global(svg), .go.ringing .what { color: var(--accent-text); }
  .what { font-weight: var(--fw-semibold); white-space: nowrap; }
  .who { flex: 1; min-width: 0; font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .time { flex-shrink: 0; font-family: var(--font-mono); font-variant-numeric: tabular-nums; color: var(--text-2); white-space: nowrap; }
</style>

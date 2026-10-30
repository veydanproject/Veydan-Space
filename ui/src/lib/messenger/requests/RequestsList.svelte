<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The requests of strangers, newest first: who (initials only, a stranger's
  picture is never fetched), the first line of their text and when. The
  desktop shows it in the rail in place of the chats, with its own way
  back; a phone screen brings its own bar.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import PanelHead from '../shared/PanelHead.svelte';
  import { listStamp } from '../shared/time';
  import { messengerError, type RequestView } from '../api';
  import { firstLine, requestName, requestStore } from './requestStore.svelte';

  interface Props {
    onopen: (request: RequestView) => void;
    /** Back to the chats: the desktop rail. */
    onback?: () => void;
  }
  let { onopen, onback }: Props = $props();

  /** The first read failed; shown only until a read (this one or the module's) succeeds. */
  let loadError = $state('');
  let moreError = $state('');
  const error = $derived(requestStore.loaded ? moreError : loadError || moreError);

  // The module store read the requests at the start and keeps them by its
  // events: the list reads them itself only when that read is still to come.
  onMount(() => {
    if (!requestStore.loaded) requestStore.load().catch((e) => (loadError = messengerError(e)));
  });

  function more() {
    moreError = '';
    requestStore.loadMore().catch((e) => (moreError = messengerError(e)));
  }
</script>

<div class="requests">
  {#if onback}<PanelHead title={$t('msg_req_title')} {onback} />{/if}

  <div class="scroll">
    <p class="intro">{$t('msg_req_intro')}</p>

    {#if !requestStore.loaded && !error}
      <p class="note">{$t('loading')}</p>
    {:else if requestStore.items.length === 0 && !error}
      <div class="empty">
        <Icon name="user-question" size={28} />
        <p>{$t('msg_req_empty')}</p>
        <span>{$t('msg_req_empty_hint')}</span>
      </div>
    {:else}
      <ul class="list-rows">
        {#each requestStore.items as r (r.peer)}
          <li>
            <button class="list-row" class:active={requestStore.selected === r.peer} onclick={() => onopen(r)}>
              <Avatar url={null} label={requestName(r)} seed={r.peer} size={48} />
              <span class="list-row-body">
                <span class="list-row-top">
                  <span class="list-row-name">{requestName(r)}</span>
                  <span class="list-row-time">{listStamp(r.at)}</span>
                </span>
                <span class="list-row-bottom">
                  <span class="list-row-line">{firstLine(r.text)}</span>
                  {#if r.held > 0}<span class="count-pill lg quiet" title={$t('msg_req_held', { n: String(r.held) })} aria-label={$t('msg_req_held', { n: String(r.held) })}>+{r.held}</span>{/if}
                </span>
              </span>
            </button>
          </li>
        {/each}
      </ul>
      {#if requestStore.nextBefore !== null}
        <button class="btn btn-ghost btn-sm more" disabled={requestStore.loadingMore} onclick={more}>{$t('msg_req_more')}</button>
      {/if}
    {/if}

    {#if error}<p class="error-msg">{error}</p>{/if}
  </div>
</div>

<style>
  .requests { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 0 var(--sp-2) var(--sp-2); display: flex; flex-direction: column; }
  .intro { margin: var(--sp-3) var(--sp-3) var(--sp-3); font-size: var(--fs-sm); line-height: 1.45; color: var(--text-3); }
  .note { margin: 0; padding: var(--sp-6) var(--sp-4); text-align: center; font-size: var(--fs-sm); color: var(--text-2); }
  /* The rows are the .list-row of base.css, as the chats: 64, an avatar of 48, a hairline between two rows. */
  /* The count held behind the request is the .count-pill of base.css, .quiet: not an unread one. */
  .empty {
    display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center;
    padding: var(--sp-8) var(--sp-4); color: var(--text-3);
  }
  .empty p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .empty span { font-size: var(--fs-xs); line-height: 1.5; }
  .more { align-self: center; margin: var(--sp-2) 0; }
  .error-msg { margin: var(--sp-2) var(--sp-3); }
</style>

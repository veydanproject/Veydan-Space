<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/LogTab.svelte at 8ce09c39, then changed for channels. -->

<!--
  What the moderators of a channel did, newest first: their decisions on
  posts and comments and the operations of the journal that keep people
  from writing or change the rules. It is read again whenever the channel
  tells of one. The wording of a line is in `log.ts`, where the tests pin it.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { stamp } from '$lib/messenger/seams/shared';
  import { nameStore, shortKey } from '../names.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { moderationError } from '../errors';
  import { channelsApi, type ChannelDetail, type LogEntry } from '../api';
  import { aboutPerson, actionText, detailText } from './log';
  import { weekdayNames } from './schedule';

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);

  let items = $state<LogEntry[]>([]);
  let next = $state<number | undefined>(undefined);
  let loading = $state(false);
  let error = $state('');

  async function load(more = false) {
    if (loading) return;
    const id = cid;
    loading = true; error = '';
    try {
      const page = await channelsApi.moderation.log(id, more ? next : undefined, 50);
      if (id !== cid) return;
      items = more ? [...items, ...page.items] : page.items;
      next = page.next_before ?? undefined;
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      loading = false;
    }
  }

  // The first page, and again after every decision the channel tells of (not at every message or read of the page).
  $effect(() => {
    void channelStore.journalRev[cid];
    untrack(() => load());
  });

  const people = $derived(new Set(detail.staff.map((s) => s.pubkey)));
  const names = $derived(weekdayNames(localeTag($locale)));

  /** The dictionary of now: read inside the functions below, so a change of language re-words the lines. */
  function say(key: string, params?: Record<string, string>): string {
    return $t(key as 'chan_title', params);
  }

  const what = (e: LogEntry): string => actionText(e, say);
  const more = (e: LogEntry): string => detailText(e, say, stamp, names);

  /** A person by name; a post or a comment by the start of its words, «…» quoted (a short id only when it is gone from here). */
  function whom(e: LogEntry): string {
    if (!e.target) return '';
    if (aboutPerson(e) || people.has(e.target)) return nameStore.label(e.target);
    return e.excerpt ? `«${e.excerpt}»` : shortKey(e.target);
  }
</script>

{#if error}<div class="error-msg">{error}</div>{/if}

{#if !items.length && !loading && !error}
  <p class="empty">{$t('chan_mod_log_empty')}</p>
{:else}
  <ul class="log">
    {#each items as e, i (`${e.at}:${e.by}:${e.action}:${e.target}:${i}`)}
      {@const detailLine = more(e)}
      <li class="entry">
        <div class="line"><b>{what(e)}</b>{#if whom(e)}<span class="target" class:excerpt={!!e.excerpt && !aboutPerson(e)}>{whom(e)}</span>{/if}</div>
        <!-- &nbsp;: Svelte drops the space a block starts with. -->
        <div class="meta">{nameStore.label(e.by)} · {stamp(e.at)}{#if detailLine}&nbsp;· {detailLine}{/if}</div>
      </li>
    {/each}
  </ul>
{/if}

{#if loading}<div class="loading"><Icon name="loader" size={14} /></div>
{:else if next}<button class="btn btn-ghost btn-sm more" onclick={() => load(true)}>{$t('chan_load_more')}</button>{/if}

<style>
  .empty { margin: 0; font-size: var(--fs-sm); color: var(--text-3); text-align: center; padding: var(--sp-4) 0; }
  .log { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .entry { display: flex; flex-direction: column; gap: 2px; padding: 8px 4px; border-bottom: 1px solid var(--divider); min-width: 0; }
  .line { font-size: var(--fs-sm); color: var(--text); overflow-wrap: anywhere; }
  .target { color: var(--text-2); margin-left: 0.35em; }
  /* The words of a post: one line, the rest cut. */
  .target.excerpt { display: inline-block; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; vertical-align: bottom; }
  .meta { font-size: var(--fs-2xs); color: var(--text-3); overflow-wrap: anywhere; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .more { align-self: center; margin-top: var(--sp-2); }
</style>

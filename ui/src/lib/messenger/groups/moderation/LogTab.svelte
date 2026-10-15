<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What the moderators of a group did, newest first: their decisions on
  messages and the operations of the log that keep people from writing or
  change the rules. It is read again whenever the group tells of one.
  The wording of a line is in `log.ts`, where the tests pin it.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { nameStore, shortKey } from '../names.svelte';
  import { chatStore } from '../../chats/chatStore.svelte';
  import { moderationError } from '../errors';
  import { stamp } from '../../shared/time';
  import { messengerApi, type LogEntry, type MessengerGroup } from '../../api';
  import { aboutPerson, actionText, detailText } from './log';
  import { weekdayNames } from './schedule';

  interface Props {
    group: MessengerGroup;
  }
  let { group }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);

  let items = $state<LogEntry[]>([]);
  let next = $state<number | undefined>(undefined);
  let loading = $state(false);
  let error = $state('');

  async function load(more = false) {
    if (loading) return;
    loading = true; error = '';
    try {
      const page = await messengerApi.moderation.groupLog(group.id, more ? next : undefined, 50);
      items = more ? [...items, ...page.items] : page.items;
      next = page.next_before ?? undefined;
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      loading = false;
    }
  }

  // The first page, and again after every decision the group tells of
  // (they reach the panel as a new queue count or a refreshed group).
  $effect(() => {
    void group;
    untrack(() => load());
  });

  const people = $derived(new Set([...group.members.map((m) => m.pubkey), ...group.banned]));
  const names = $derived(weekdayNames(localeTag($locale)));

  /** The dictionary of now: read inside the functions below, so a change of language re-words the lines. */
  function say(key: string, params?: Record<string, string>): string {
    return $t(key as 'msg_you', params);
  }

  const what = (e: LogEntry): string => actionText(e, say);
  const more = (e: LogEntry): string => detailText(e, say, stamp, names);

  /**
   * A person by name; a message by its words while the chat holds it, else
   * by a short id. A line about a person is known by its shape, not by the
   * members list: a member sees no list of the banned.
   */
  function whom(e: LogEntry): string {
    if (!e.target) return '';
    if (aboutPerson(e) || people.has(e.target)) return nameStore.label(e.target);
    const m = chatStore.messages.find((x) => x.id === e.target);
    return m?.text ? `«${m.text.length > 60 ? `${m.text.slice(0, 60)}…` : m.text}»` : shortKey(e.target);
  }
</script>

{#if error}<div class="error-msg">{error}</div>{/if}

{#if !items.length && !loading && !error}
  <p class="empty">{$t('msg_mod_log_empty')}</p>
{:else}
  <ul class="log">
    {#each items as e, i (`${e.at}:${e.by}:${e.action}:${e.target}:${i}`)}
      {@const detail = more(e)}
      <li class="entry">
        <div class="line"><b>{what(e)}</b>{#if whom(e)} <span class="target">{whom(e)}</span>{/if}</div>
        <div class="meta">{nameStore.label(e.by)} · {stamp(e.at)}{#if detail} · {detail}{/if}</div>
      </li>
    {/each}
  </ul>
{/if}

{#if loading}<div class="loading"><Icon name="loader" size={14} /></div>
{:else if next}<button class="btn btn-ghost btn-sm more" onclick={() => load(true)}>{$t('msg_mod_more')}</button>{/if}

<style>
  .empty { margin: 0; font-size: var(--fs-sm); color: var(--text-3); text-align: center; padding: var(--sp-4) 0; }
  .log { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .entry { display: flex; flex-direction: column; gap: 2px; padding: 8px 4px; border-bottom: 1px solid var(--border); min-width: 0; }
  .line { font-size: var(--fs-sm); color: var(--text); overflow-wrap: anywhere; }
  /* Markup drops the blank between the two; the gap is the style's. */
  .target { color: var(--text-2); margin-left: 0.35em; }
  .meta { font-size: var(--fs-2xs); color: var(--text-3); overflow-wrap: anywhere; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .more { align-self: center; margin-top: var(--sp-2); }
</style>

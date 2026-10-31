<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The requests for a level, for the moderators of a channel (communities-wire
  §14.4): a card per request, newest first — who asks (as a request to
  join shows them), the level and what it opens, their note — with «Дать»
  (a `set_levels` in the journal) and «Отказать» with a reason that goes
  back to them as a note. Read again whenever the channel tells of a
  request (`channel.level_request`) or of a change.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { copied, toast } from '$lib/core/ui/toast.svelte';
  import { Avatar, shortKey } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { nameStore } from '../names.svelte';
  import { channelError } from '../errors';
  import { channelsApi, errorText, type ChannelDetail } from '../api';
  import { cameAt, waited, waitedText } from '../moderation/requests';
  import { EVERYONE, levelsOf } from './sections';
  import type { LevelRequestView } from './types';

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const lv = $derived(levelsOf(detail));

  let requests = $state<LevelRequestView[]>([]);
  let loaded = $state(false);
  let acting = $state<string | null>(null);
  let error = $state('');
  let refusing = $state<{ open: boolean; r: LevelRequestView | null; reason: string }>({ open: false, r: null, reason: '' });

  const key = (r: LevelRequestView) => `${r.requester}:${r.level}`;

  async function load() {
    const id = cid;
    try {
      const list = await channelsApi.levels.requests(id);
      if (id === cid) { requests = [...(list ?? [])].sort((a, b) => b.created_at - a.created_at); loaded = true; }
    } catch (e) {
      error = channelError(e, tr);
    }
  }

  $effect(() => {
    void channelStore.levelRequests[cid];
    void channelStore.journalRev[cid];
    untrack(() => load());
  });

  /** Seconds now: «ждёт» moves on without anything else changing. */
  let now = $state(Math.floor(Date.now() / 1000));
  $effect(() => {
    const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(timer);
  });
  const lang = $derived(localeTag($locale));

  const levelOf = (id: number) => lv.levels.find((l) => l.id === id) ?? null;
  const titleOf = (id: string) => (id === EVERYONE ? $t('chan_scope_everyone') : (lv.scopes.find((s) => s.scope_id === id)?.title ?? ''));

  async function answer(r: LevelRequestView, grant: boolean, reason?: string) {
    if (acting) return;
    acting = key(r); error = '';
    try {
      await channelStore.answerLevelRequest(cid, r.requester, r.level, grant, reason);
      requests = requests.filter((x) => key(x) !== key(r));
      refusing.open = false;
    } catch (e) {
      error = channelError(e, tr);
      // The answer may be in the journal already (the note did not go): the
      // list says what is still to answer.
      void load();
    } finally {
      acting = null;
    }
  }

  async function copyKey(pk: string) {
    try { await navigator.clipboard.writeText(pk); copied(); } catch (e) { toast.error(errorText(e)); }
  }
</script>

{#if error}<div class="error-msg" role="alert">{error}</div>{/if}

{#if loaded && requests.length === 0}
  <div class="empty-state">
    <div class="empty-icon"><Icon name="layers" size={24} /></div>
    <p>{$t('chan_level_req_empty')}</p>
    <span>{$t('chan_level_req_empty_hint')}</span>
  </div>
{:else}
  <ul class="cards">
    {#each requests as r (key(r))}
      {@const name = nameStore.label(r.requester) || shortKey(r.requester)}
      {@const p = nameStore.profile(r.requester)}
      {@const level = levelOf(r.level)}
      {@const wait = waited(r.created_at, now)}
      {@const busy = acting !== null}
      <li class="card" aria-label={name}>
        <div class="head">
          <Avatar url={nameStore.picture(r.requester)} label={name} seed={r.requester} size={40} />
          <div class="who">
            <span class="name" title={name}>{name}</span>
            {#if p?.nip05?.trim()}
              <span class="nip05" class:ok={!!p.nip05_verified}><Icon name={p.nip05_verified ? 'rosette-discount-check' : 'world'} size={12} /><span class="nip05-text">{p.nip05}</span></span>
            {/if}
            <button type="button" class="npub mono" title={$t('chan_req_copy_key')} aria-label={$t('chan_req_copy_key')} onclick={() => copyKey(r.requester)}>
              {shortKey(r.requester)}<Icon name="copy" size={11} />
            </button>
          </div>
        </div>
        <div class="level">
          <span class="lvl"><Icon name="layers" size={12} />{level?.name ?? $t('chan_level_req_unknown')}</span>
          {#if level}
            {@const opens = level.scopes.map(titleOf).filter(Boolean)}
            {#if opens.length}<span class="opens">{$t('chan_level_request_adds', { list: opens.map((x) => `«${x}»`).join(', ') })}</span>{/if}
          {/if}
        </div>
        {#if r.note}<blockquote class="note">{r.note}</blockquote>{:else}<p class="note-none">{$t('chan_req_no_note')}</p>{/if}
        <div class="foot">
          <span class="when">
            <Icon name="clock" size={12} />
            <span>{cameAt(r.created_at, new Date(now * 1000), lang, (k) => $t(`chan_day_${k}` as 'chan_day_today'))}</span><span class="sep">·</span>
            <span class="wait">{wait ? $t('chan_req_waiting', { time: waitedText(wait, lang) }) : $t('chan_req_just_now')}</span>
          </span>
          <span class="acts">
            <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (refusing = { open: true, r, reason: '' })}>{$t('chan_level_req_refuse')}</button>
            <button type="button" class="btn btn-primary btn-sm" disabled={busy || !level} onclick={() => answer(r, true)}><Icon name="check" size={14} />{$t('chan_level_req_grant')}</button>
          </span>
        </div>
      </li>
    {/each}
  </ul>
{/if}

<Dialog bind:open={refusing.open} title={$t('chan_level_req_refuse_title', { name: refusing.r ? nameStore.label(refusing.r.requester) : '' })} width="min(420px, calc(100vw - 24px))">
  <div class="dlg">
    <p class="hint">{$t('chan_level_req_refuse_hint')}</p>
    <textarea class="field" rows="3" maxlength="500" bind:value={refusing.reason} placeholder={$t('chan_level_req_reason')} aria-label={$t('chan_level_req_reason')} disabled={acting !== null}></textarea>
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={acting !== null} onclick={() => (refusing.open = false)}>{$t('chan_cancel')}</button>
    <button type="button" class="btn btn-danger" disabled={acting !== null}
      onclick={() => refusing.r && answer(refusing.r, false, refusing.reason.trim() || undefined)}>{$t('chan_level_req_refuse')}</button>
  {/snippet}
</Dialog>

<style>
  .cards { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-3); }
  .card { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3); background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-lg); min-width: 0; }
  .head { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding-top: 1px; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nip05 { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); min-width: 0; }
  .nip05.ok { color: var(--success-text, var(--success)); }
  .nip05-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .npub { align-self: flex-start; display: inline-flex; align-items: center; gap: 5px; border: none; background: none; padding: 0; cursor: pointer; font-size: var(--fs-2xs); color: var(--text-3); }
  .npub:hover { color: var(--accent-text); }
  .level { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 8px; }
  .lvl {
    display: inline-flex; align-items: center; gap: 4px; padding: 2px 10px; border-radius: var(--radius-pill);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2); background: var(--accent-tint);
  }
  .opens { font-size: var(--fs-xs); color: var(--text-2); }
  .note {
    margin: 0; padding: var(--sp-2) var(--sp-3); border-left: 3px solid var(--accent-border); border-radius: var(--radius-xs);
    background: var(--surface); font-size: var(--fs-sm); line-height: 1.45; color: var(--text-body); white-space: pre-wrap; overflow-wrap: anywhere;
  }
  .note-none { margin: 0; font-size: var(--fs-sm); color: var(--text-3); font-style: italic; }
  .foot { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2) var(--sp-3); flex-wrap: wrap; padding-top: 2px; }
  .when { display: inline-flex; align-items: center; flex-wrap: wrap; gap: 2px 5px; font-size: var(--fs-2xs); color: var(--text-3); min-width: 0; }
  .wait { color: var(--text-2); font-weight: var(--fw-semibold); }
  .acts { display: flex; gap: 6px; margin-left: auto; }
  .empty-state { padding: var(--sp-10) var(--sp-4); }
  .empty-state p { margin: 0; }
  .dlg { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 10px; resize: vertical;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  @media (pointer: coarse) {
    .npub { padding: 4px 0; font-size: var(--fs-xs); }
    .acts { flex: 1 1 100%; }
    .acts .btn { flex: 1; justify-content: center; min-height: var(--control-h); }
    .field { font-size: var(--fs-md); }
  }
</style>

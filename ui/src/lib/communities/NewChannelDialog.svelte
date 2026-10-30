<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/NewGroupDialog.svelte at 8ce09c39, then changed for channels. -->

<!--
  Start a channel, or come into one by its link. A new channel gets a
  name, a few words, a cover, how readers get in (by the link, by a
  request, by an invitation only) and how much of the past a newcomer
  reads; its relays and the hosts of its files start as those of the
  account, and can be narrowed or added to. This device becomes its keeper.
  A link shows the channel's card first (`channel_preview_link`, signed by
  its owner), then «Войти» or «Подать заявку» opens it (`channel_open_link`).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import { Avatar, ServerPicker, accountServers, homeServersLabel, isTauriHost, nameOfServer, serverStore } from '$lib/messenger/seams/shared';
  import { channelsApi, type ChannelPreview } from './api';
  import { cover as coverOf } from './cover';
  import { channelStore, chatIdOf } from './channelStore.svelte';
  import { channelError } from './errors';
  import type { ChannelDetail } from './api';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
  import ChoiceRow from '$lib/core/ui/ChoiceRow.svelte';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);

  interface Props {
    open: boolean;
    /** Open on the link tab with this link (a link that was followed). */
    link?: string;
    onopened?: (chatId: string) => void;
  }
  let { open = $bindable(), link = '', onopened }: Props = $props();

  type Tab = 'create' | 'join';
  const JOINS = ['open', 'request', 'invite'] as const;
  let tab = $state<Tab>('create');
  let join = $state<(typeof JOINS)[number]>('open');
  let history = $state<'open' | 'from_join'>('open');
  let name = $state('');
  let about = $state('');
  let cover = $state<string | null>(null);
  /** The servers of the account (its relays that write, its file hosts that are on). */
  let home = $state<{ relays: string[]; storage: string[] }>({ relays: [], storage: [] });
  /** A private server of mine the channel lives on instead; `null`: the account's. */
  let server = $state<string | null>(null);
  /** Without Veydan Cloud the files of a channel go to the free media of the messenger (14 days): no long history then. */
  let plan = $state<{ free: boolean; retention_days: number }>({ free: false, retention_days: 0 });
  let url = $state('');
  let note = $state('');
  /** Opened from a card of a link: the link is known, shown only on demand, and nothing else is offered. */
  let fromCard = $state(false);
  let showLink = $state(false);
  let busy = $state(false);
  let error = $state('');

  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      error = ''; name = ''; about = ''; note = ''; cover = null; join = 'open'; history = 'open'; server = null;
      url = link; tab = link ? 'join' : 'create'; fromCard = !!link; showLink = false;
      accountServers().then((s) => (home = s)).catch(() => {});
      channelsApi.storagePlan().then((p) => (plan = p)).catch(() => {});
      void serverStore.load();
    }
    wasOpen = open;
  });

  // Where it lives is chosen as a group's is (ServerPicker): the account's servers, or one
  // private server of mine, its relay and its file host; no addresses are typed or shown.
  const servers = $derived(serverStore.list ?? []);
  const chosen = $derived(servers.find((s) => s.id === server) ?? null);
  const chosenRelays = $derived(chosen ? [chosen.relay] : home.relays);
  const chosenStorage = $derived(chosen ? (chosen.media ? [chosen.media] : []) : home.storage);
  const serversOk = $derived(chosenRelays.length > 0 && chosenStorage.length > 0);
  // «С момента входа» keeps every epoch's keys for a newcomer's month: the free media keeps
  // them 14 days (communities-wire §9, the owner's rule of 2026-10-23), so it needs Cloud or a server of mine.
  const longHistory = $derived(!!chosen || !plan.free);
  $effect(() => {
    if (!longHistory && history === 'from_join') history = 'open';
  });

  async function pickCover() {
    if (!isTauriHost) { cover = '/home/dev/Pictures/cover.png'; return; }
    const { open: pick } = await import('@tauri-apps/plugin-dialog');
    const picked = await pick({ multiple: false, directory: false, filters: [{ name: $t('chan_cover'), extensions: ['jpg', 'jpeg', 'png'] }] });
    if (typeof picked === 'string') cover = picked;
  }

  /** The channel is here: the dialog goes at once, its row opens, the rows are read again behind it. */
  function finish(d: ChannelDetail) {
    open = false;
    onopened?.(chatIdOf(d.channel.id));
    channelStore.refreshList().catch(() => {});
  }

  /** The cover of a channel made now; a refusal is a toast with «Повторить» (the dialog is gone by then). */
  async function setCover(id: string, src: string) {
    try {
      await channelStore.setAvatar(id, src);
    } catch (e) {
      toast.error(channelError(e, tr), { onAction: () => void setCover(id, src) });
    }
  }

  async function create() {
    error = ''; busy = true;
    let d: ChannelDetail;
    try {
      d = await channelStore.create({
        name: name.trim(), about: about.trim() || undefined, join, history, relays: chosenRelays, storage: chosenStorage,
      });
    } catch (e) {
      error = channelError(e, tr);
      return;
    } finally {
      busy = false;
    }
    const picked = cover;
    finish(d);
    // The cover goes once the channel is there (an operation of its journal), after the dialog; a refusal is a toast that tries again.
    if (picked) void setCover(d.channel.id, picked);
  }

  const linkLooksRight = $derived(/^veydan:\/\/channel\/\S+$/i.test(url.trim()));

  // ── The card of the link's channel, before coming in ──────────────────
  let preview = $state<ChannelPreview | null>(null);
  let previewing = $state(false);
  let previewError = $state('');
  let previewedFor = '';
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function loadPreview(u: string) {
    preview = null; previewError = ''; previewing = true;
    try {
      const p = await channelsApi.previewLink(u);
      if (url.trim() === u) preview = p;
    } catch (e) {
      if (url.trim() === u) previewError = channelError(e, tr);
    } finally {
      if (url.trim() === u) previewing = false;
    }
  }

  // A link typed or pasted is looked up once it stops changing.
  $effect(() => {
    const u = url.trim();
    const ok = open && tab === 'join' && linkLooksRight;
    untrack(() => {
      clearTimeout(timer);
      if (!ok) { previewedFor = ''; preview = null; previewError = ''; previewing = false; return; }
      if (u === previewedFor) return;
      previewedFor = u;
      previewing = true;
      timer = setTimeout(() => void loadPreview(u), 300);
    });
  });

  /** What the link lets me do: open what I am in, come in, ask, or nothing (by invitation). */
  const way = $derived.by((): 'open' | 'enter' | 'request' | 'closed' | null => {
    const p = preview;
    if (!p) return null;
    if (p.membership === 'joined') return 'open';
    if (p.has_secret || p.join === 'open' || p.invited) return 'enter';
    return p.join === 'request' ? 'request' : 'closed';
  });
  const readers = $derived(preview ? $t('chan_readers_n', { n: new Intl.NumberFormat(localeTag($locale)).format(preview.members) }) : '');

  async function enter() {
    if (!linkLooksRight) return;
    if (way === 'open' && preview) { open = false; onopened?.(chatIdOf(preview.community_id)); return; }
    error = ''; busy = true;
    try { finish(await channelStore.openLink(url.trim(), note.trim() || undefined)); }
    catch (e) { error = channelError(e, tr); }
    finally { busy = false; }
  }
</script>

<Dialog bind:open title={tab === 'join' ? $t('chan_join_title') : $t('chan_new_title')} width="min(520px, calc(100vw - 24px))">
  <div class="body">
    {#if !fromCard}
    <div class="seg tabs" role="tablist">
      <button role="tab" class="seg-btn" class:active={tab === 'create'} aria-selected={tab === 'create'} onclick={() => (tab = 'create')}>{$t('chan_new_create')}</button>
      <button role="tab" class="seg-btn" class:active={tab === 'join'} aria-selected={tab === 'join'} onclick={() => (tab = 'join')}>{$t('chan_new_join')}</button>
    </div>
    {/if}

    {#if tab === 'create'}
      <div class="top">
        <button class="cover" class:picked={!!cover} onclick={pickCover} disabled={busy} title={$t('chan_cover_set')} aria-label={$t('chan_cover_set')}>
          {#if cover}<Icon name="check" size={18} />{:else}<Icon name="camera" size={22} />{/if}
        </button>
        <div class="names">
          <input class="field tall" type="text" bind:value={name} maxlength="64" placeholder={$t('chan_name')} aria-label={$t('chan_name')} disabled={busy}
            onkeydown={(e) => { if (e.key === 'Enter' && name.trim() && serversOk) create(); }} />
          {#if cover}<span class="option-hint">{$t('chan_cover_picked', { name: cover.split(/[\\/]/).pop() ?? '' })}</span>{/if}
        </div>
      </div>
      <textarea class="field" rows="2" bind:value={about} maxlength="500" placeholder={$t('chan_about')} aria-label={$t('chan_about')} disabled={busy}></textarea>

      <section class="sec">
        <span class="cap" id="chan-new-join">{$t('chan_rule_join')}</span>
        <div class="kinds" role="radiogroup" aria-labelledby="chan-new-join">
          {#each JOINS as k (k)}
            <ChoiceRow icon={k === 'open' ? 'world' : k === 'request' ? 'user-question' : 'lock-square-rounded'}
              title={$t(`chan_join_${k}` as 'chan_join_open')} hint={$t(`chan_join_${k}_hint` as 'chan_join_open_hint')}
              checked={join === k} onclick={() => (join = k)} disabled={busy} />
          {/each}
        </div>
      </section>

      {#if longHistory}
        <ToggleRow title={$t('chan_history_for_new')} hint={$t(history === 'open' ? 'chan_history_open_hint' : 'chan_history_from_join_hint')}
          on={history === 'open'} disabled={busy} onclick={() => (history = history === 'open' ? 'from_join' : 'open')} />
      {:else}
        <div class="option">
          <div class="option-info">
            <span class="option-title">{$t('chan_history_for_new')}</span>
            <span class="option-hint">{$t('chan_history_open_hint')} {$t('chan_history_needs_storage')}</span>
          </div>
        </div>
      {/if}

      <section class="sec where">
        {#if servers.length}
          <span class="cap">{$t('chan_where')}</span>
          <ServerPicker bind:value={server} none={homeServersLabel()} disabled={busy} label={$t('chan_where')} />
        {/if}
        <span class="option-hint">{chosen ? $t('chan_where_private', { name: nameOfServer(chosen) }) : $t('chan_where_home')}</span>
        {#if !serversOk}<p class="hint bad"><Icon name="alert-triangle" size={16} /><span>{chosen ? $t('chan_where_no_files') : $t('chan_servers_none')}</span></p>
        {:else if !chosen && plan.free}<p class="note"><Icon name="info" size={18} /><span>{$t('chan_free_media_note', { days: String(plan.retention_days) })}</span></p>{/if}
      </section>

      <p class="note"><Icon name="info" size={18} /><span>{$t('chan_keeper_note')}</span></p>
      {#if error}<div class="error-msg" role="alert"><Icon name="alert-triangle" size={18} /><span>{error}</span></div>{/if}
    {:else}
      {#if fromCard && !showLink}
        <button type="button" class="link-toggle" aria-expanded="false" onclick={() => (showLink = true)}><Icon name="link" size={18} />{$t('chan_join_show_link')}</button>
      {:else}
        <label class="sec">
          <span class="cap">{$t('chan_join_link')}</span>
          <textarea class="field mono" class:bad={!!url.trim() && !linkLooksRight} rows="3" bind:value={url} placeholder="veydan://channel/…" aria-label={$t('chan_join_link')} spellcheck="false" disabled={busy}></textarea>
        </label>
      {/if}
      {#if url.trim() && !linkLooksRight}
        <p class="hint bad"><Icon name="alert-triangle" size={16} /><span>{$t('chan_link_bad')}</span></p>
      {:else if linkLooksRight}
        {#if preview}
          {@const c = coverOf(preview.picture)}
          <div class="ch-card">
            <Avatar url={c.url} src={c.src} label={preview.name} seed={preview.community_id} size={48} />
            <div class="ch-card-text">
              <span class="ch-card-name">{preview.name}</span>
              <span class="ch-card-sub">
                <Icon name={preview.join === 'open' ? 'world' : preview.join === 'request' ? 'user-question' : 'lock-square-rounded'} size={14} />
                {$t(`chan_join_${preview.join}` as 'chan_join_open')} · {readers}
              </span>
              {#if preview.about}<span class="ch-card-about">{preview.about}</span>{/if}
            </div>
          </div>
          {#if way === 'request'}
            <input class="field tall" type="text" bind:value={note} maxlength="300" placeholder={$t('chan_join_note')} aria-label={$t('chan_join_note')} disabled={busy} />
          {:else if way === 'closed'}
            <p class="note"><Icon name="lock-square-rounded" size={18} /><span>{$t('chan_preview_invite_only')}</span></p>
          {/if}
        {:else if previewError}
          <div class="error-msg" role="alert"><Icon name="alert-triangle" size={18} /><span>{previewError}</span></div>
          <button class="btn btn-ghost btn-sm retry" onclick={() => { previewedFor = url.trim(); void loadPreview(url.trim()); }}>
            <Icon name="refresh-cw" size={16} />{$t('chan_retry')}
          </button>
        {:else}
          <div class="found" role="status"><span class="spinner" aria-hidden="true"></span><span>{$t('chan_preview_loading')}</span></div>
        {/if}
      {:else}
        <p class="hint">{$t('chan_join_hint')}</p>
      {/if}
      {#if error}<div class="error-msg" role="alert"><Icon name="alert-triangle" size={18} /><span>{error}</span></div>{/if}
    {/if}
  </div>

  {#snippet footer()}
    {#if tab === 'create'}
      <button class="btn btn-primary go" disabled={busy || !name.trim() || !serversOk} aria-busy={busy} onclick={create}>
        {#if busy}<span class="spinner light" aria-hidden="true"></span>{/if}{$t('chan_create')}
      </button>
    {:else}
      <button class="btn btn-primary go" disabled={busy || !linkLooksRight || !way || way === 'closed' || previewing} aria-busy={busy} onclick={enter}>
        {#if busy}<span class="spinner light" aria-hidden="true"></span>{/if}{way === 'open' ? $t('chan_preview_open') : way === 'request' ? $t('chan_preview_request') : $t('chan_join_cta')}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  /* The spinner of a primary button, on its fill. */
  .spinner.light { border-color: rgba(255, 255, 255, 0.35); border-top-color: #fff; }
  .body { display: flex; flex-direction: column; gap: var(--sp-4); }
  .body > :global(*) { flex-shrink: 0; min-width: 0; }
  .tabs { display: flex; width: 100%; }
  .tabs .seg-btn { flex: 1; justify-content: center; height: 36px; font-size: var(--fs-base); }
  /* Only the button under the line: the whole width, 48 high. */
  .go { flex: 1; justify-content: center; min-height: 48px; font-size: var(--fs-body); }

  .top { display: flex; align-items: center; gap: var(--sp-3); }
  .cover {
    width: 64px; height: 64px; flex-shrink: 0; border-radius: 50%; border: 1px dashed var(--accent-border); background: var(--accent-bg);
    color: var(--accent-text); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; padding: 0;
  }
  .cover.picked { border-style: solid; background: var(--accent-grad); color: #fff; }
  .names { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); }

  .sec { display: flex; flex-direction: column; gap: var(--sp-2); }
  .cap { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }

  .kinds { display: flex; flex-direction: column; gap: var(--sp-2); }

  .field {
    width: 100%; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: var(--sp-2) var(--sp-3); resize: vertical;
  }
  .field.tall { height: var(--control-h-lg); padding: 0 var(--sp-3); }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .field.bad { border-color: var(--danger-text); }
  .mono { font-size: var(--fs-xs); overflow-wrap: anywhere; }

  .option {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px; padding: var(--sp-3) var(--sp-4);
    font: inherit; color: var(--text); text-align: left;
    border: 1px solid var(--pane-strong-border); border-radius: var(--radius-lg); background: var(--pane-strong);
  }
  .option-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); font-size: var(--fs-base); line-height: 1.4; }
  .option-title { font-weight: var(--fw-semibold); }
  .option-hint { font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .where :global(.chip) { min-height: 36px; padding-inline: var(--sp-3); font-size: var(--fs-sm); gap: var(--sp-2); }

  .note { display: flex; gap: var(--sp-2); align-items: flex-start; margin: 0; font-size: var(--fs-sm); line-height: 1.45; color: var(--text-2); }
  .note :global(svg) { flex-shrink: 0; color: var(--text-3); margin-top: 1px; }
  .error-msg { display: flex; gap: var(--sp-2); align-items: flex-start; line-height: 1.45; }
  .error-msg :global(svg) { flex-shrink: 0; margin-top: 1px; }

  .found { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md); background: var(--surface-3); border: 1px solid var(--border); }
  .ch-card { display: flex; gap: var(--sp-3); align-items: flex-start; padding: var(--sp-4); border-radius: var(--radius-lg); background: var(--pane-strong); border: 1px solid var(--pane-strong-border); }
  .ch-card-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .ch-card-name { font-size: var(--fs-md); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ch-card-sub { display: inline-flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-3); }
  .ch-card-about { font-size: var(--fs-base); color: var(--text-2); line-height: 1.45; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .retry { align-self: flex-start; }
  .link-toggle {
    align-self: flex-start; display: inline-flex; align-items: center; gap: var(--sp-2); padding: var(--sp-1) 0; border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-sm); color: var(--text-3);
  }
  .link-toggle:hover { color: var(--text-2); }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.5; }
  .hint.bad { display: flex; gap: var(--sp-2); align-items: center; color: var(--danger-text); }
  .hint.bad :global(svg) { flex-shrink: 0; }
  @media (pointer: coarse) { .field { font-size: var(--fs-md); } .field.tall { height: 48px; } }
</style>

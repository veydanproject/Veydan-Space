<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Start a group, or come into one by its link. -->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import { groupError } from './errors';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
  import ChoiceRow from '$lib/core/ui/ChoiceRow.svelte';
  import { groupStore } from './groupStore.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { linkStore } from '../content/linkStore.svelte';
  import { tokenize } from '../content/tokenize';
  import { type MessengerGroup, type ServerPreview } from '../api';
  import { acceptServer } from '../servers/serverAccept.svelte';
  import ServerBox from '../servers/ServerBox.svelte';
  import ServerPicker from '../servers/ServerPicker.svelte';
  import { nameOfServer, serverStore } from '../servers/serverStore.svelte';
  import { messengerStore } from '../store.svelte';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as "msg_you", params);

  interface Props {
    open: boolean;
    /** Open on the link tab with this link (a link that was followed). */
    link?: string;
    onopened?: (chatId: string) => void;
  }
  let { open = $bindable(), link = '', onopened }: Props = $props();

  type Tab = 'create' | 'join';
  let tab = $state<Tab>('create');
  let kind = $state<'private' | 'public'>('private');
  let name = $state('');
  let about = $state('');
  let history = $state(true);
  /** Where the group lives: a private server of mine, or `null` for the home set. */
  let server = $state<string | null>(null);
  let url = $state('');
  let note = $state('');
  let busy = $state(false);
  let error = $state('');

  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      error = ''; name = ''; about = ''; note = ''; kind = 'private'; history = true; server = null;
      void serverStore.load();
      url = link; tab = link ? 'join' : 'create';
    }
    wasOpen = open;
  });

  // What the text is, is for the runtime to say: a link is never taken apart here.
  let parsed = $state<{ kind: 'public' | 'private'; name: string; link: string; server: ServerPreview | null; serverKnown: boolean } | null>(null);
  $effect(() => {
    const text = url.trim();
    parsed = null;
    const parts = tokenize(text);
    const only = parts.length === 1 && parts[0].type === 'internal' ? parts[0] : null;
    if (!only) return;
    let stale = false;
    linkStore.resolve(only.link)
      .then((v) => {
        if (!stale && v.kind === 'group') parsed = { kind: v.group_kind, name: v.name, link: v.link, server: v.server ?? null, serverKnown: v.server_known ?? true };
      })
      .catch(() => {});
    return () => { stale = true; };
  });

  function explain(e: unknown): string {
    return groupError(e, tr);
  }

  async function finish(g: MessengerGroup) {
    await chatStore.loadChats();
    await chatStore.open(g.chat_id);
    open = false;
    onopened?.(g.chat_id);
  }

  async function create() {
    error = ''; busy = true;
    try { await finish(await groupStore.create(kind, name.trim(), about.trim(), kind === 'public' || history, server)); }
    catch (e) { error = explain(e); }
    finally { busy = false; }
  }

  /** The choice of where the group lives, shown only with a private server of mine. */
  const servers = $derived(serverStore.list ?? []);
  const homeLabel = $derived(messengerStore.manifest?.mode === 'own' ? $t('msg_srv_mode') : $t('msg_srv_veydan_title'));
  const chosen = $derived(servers.find((s) => s.id === server) ?? null);

  async function join() {
    if (!parsed) return;
    const p = parsed;
    error = ''; busy = true;
    try {
      // A group on a server of its own: the server first, after asking; «Отмена» adds and sends nothing.
      if (p.server && !p.serverKnown && !(await acceptServer(p.server))) return;
      await finish(await groupStore.openLink(p.link, note.trim()));
      linkStore.refresh();
    }
    catch (e) { error = explain(e); }
    finally { busy = false; }
  }
</script>

<Dialog bind:open title={$t('msg_group_new_title')} width="min(460px, calc(100vw - 24px))">
  <div class="body">
    <div class="seg tabs" role="tablist">
      <button role="tab" class="seg-btn" class:active={tab === 'create'} aria-selected={tab === 'create'} onclick={() => (tab = 'create')}>{$t('msg_group_new_create')}</button>
      <button role="tab" class="seg-btn" class:active={tab === 'join'} aria-selected={tab === 'join'} onclick={() => (tab = 'join')}>{$t('msg_group_new_join')}</button>
    </div>

    {#if tab === 'create'}
      <div class="kinds" role="radiogroup">
        {#each ['private', 'public'] as const as k}
          <ChoiceRow icon={k === 'public' ? 'world' : 'lock-square-rounded'}
            title={$t(`msg_group_kind_${k}` as 'msg_group_kind_public')} hint={$t(`msg_group_kind_${k}_text` as 'msg_group_kind_public_text')}
            checked={kind === k} onclick={() => (kind = k)} disabled={busy} />
        {/each}
      </div>
      <input class="field tall" type="text" bind:value={name} maxlength="64" placeholder={$t('msg_group_name')} aria-label={$t('msg_group_name')} disabled={busy}
        onkeydown={(e) => { if (e.key === 'Enter' && name.trim()) create(); }} />
      <textarea class="field" rows="2" bind:value={about} maxlength="500" placeholder={$t('msg_group_about')} aria-label={$t('msg_group_about')} disabled={busy}></textarea>
      {#if servers.length}
        <div class="where">
          <span class="cap">{$t('msg_group_where')}</span>
          <ServerPicker bind:value={server} none={homeLabel} disabled={busy} label={$t('msg_group_where')} />
          <span class="where-hint">{chosen ? $t('msg_group_where_private', { name: nameOfServer(chosen) }) : $t('msg_group_where_home')}</span>
        </div>
      {/if}
      {#if kind === 'private'}
        <!-- The whole row is the switch. -->
        <ToggleRow surface="panel" title={$t('msg_group_history_for_new')} hint={$t(history ? 'msg_group_note_history_on' : 'msg_group_note_history_off')}
          on={history} disabled={busy} onclick={() => (history = !history)} />
      {:else}
        <p class="hint">{$t('msg_group_note_public')}</p>
      {/if}
      {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
    {:else}
      <textarea class="field mono" rows="3" bind:value={url} placeholder="veydan://group/…" aria-label={$t('msg_group_new_join')} spellcheck="false" disabled={busy}></textarea>
      {#if parsed}
        <div class="found">
          <Icon name={parsed.kind === 'public' ? 'world' : 'lock-square-rounded'} size={16} />
          <span><b>{parsed.name || $t('msg_group_unnamed')}</b> · {$t(`msg_group_kind_${parsed.kind}` as 'msg_group_kind_public')}</span>
        </div>
        {#if parsed.server}
          <ServerBox server={parsed.server} />
        {:else if !parsed.serverKnown}
          <p class="hint bad">{$t('msg_srv_group_unknown')}</p>
        {/if}
        <p class="hint">{$t(parsed.kind === 'public' ? 'msg_group_join_hint_public' : 'msg_group_join_hint_private')}</p>
        {#if parsed.kind === 'private'}
          <input class="field tall" type="text" bind:value={note} maxlength="300" placeholder={$t('msg_group_join_note')} aria-label={$t('msg_group_join_note')} disabled={busy} />
        {/if}
      {:else if url.trim()}
        <p class="hint bad">{$t('msg_group_link_bad')}</p>
      {:else}
        <p class="hint">{$t('msg_group_join_hint')}</p>
      {/if}
      {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
    {/if}
  </div>

  <!-- The verb of the form is fixed at the foot; the body scrolls above it. -->
  {#snippet footer()}
    {#if tab === 'create'}
      <button class="btn btn-primary go" disabled={busy || !name.trim()} onclick={create}>{$t('msg_group_create')}</button>
    {:else}
      <button class="btn btn-primary go" disabled={busy || !parsed} onclick={join}>
        {$t(parsed?.kind === 'private' ? 'msg_group_join_ask' : 'msg_group_join')}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .body > :global(*) { flex-shrink: 0; min-width: 0; }
  .tabs { display: flex; width: 100%; }
  .tabs .seg-btn { flex: 1; justify-content: center; height: 36px; font-size: var(--fs-base); }
  /* The one button under the line: the whole width, 44 high (48 under a finger). */
  .go { flex: 1; justify-content: center; min-height: var(--control-h-lg); }

  .kinds { display: flex; flex-direction: column; gap: var(--sp-2); }

  .field {
    width: 100%; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius); padding: var(--sp-3); resize: none;
  }
  .field.tall { height: var(--control-h-lg); padding: 0 var(--sp-3); }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .mono { font-size: var(--fs-xs); overflow-wrap: anywhere; }

  .where { display: flex; flex-direction: column; gap: var(--sp-2); }
  .cap { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .where-hint { font-size: var(--fs-xs); color: var(--text-3); }


  .found { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); padding: var(--sp-3); border-radius: var(--radius-md); background: var(--surface-3); border: 1px solid var(--border); overflow-wrap: anywhere; }
  .found :global(svg) { flex-shrink: 0; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  .hint.bad { color: var(--danger-text); }
  @media (pointer: coarse) {
    .field { font-size: var(--fs-md); }
    .field.tall, .go { min-height: var(--sp-12); }
  }
</style>

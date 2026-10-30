<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Forward messages of a chat to contacts and groups of mine (a sheet on a
  phone, a dialog on a computer; the list of ShareDialog). Only what the
  runtime takes is offered (`forwardTarget`), up to ten chats. The sender
  may be hidden: then the messages go as mine, with no «Forwarded from».
  The command answers when everything is out, a file uploaded again for a
  chat of another server too; until then the button waits.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { messengerStore } from '../store.svelte';
  import { dmErrorCode, messengerError, type MessengerChat } from '../api';
  import { chatMatches, contactHaystacks } from '../shared/search';
  import { mediaErrorText } from '../media/errors';
  import { holdBack } from '../attach/sheetBack';
  import { ForwardSessions, forwardOutcome, forwardTarget, MAX_FORWARD_MESSAGES, MAX_FORWARD_TARGETS } from '../dm/select';
  import { openChat } from './actions';

  interface Props {
    open: boolean;
    /** The chat the messages are of. */
    chatId: string;
    /** The messages to forward. */
    ids: string[];
    /** Everything went out (or the chats that refused are told): the mode of choosing closes. */
    onforwarded?: () => void;
  }
  let { open = $bindable(), chatId, ids, onforwarded }: Props = $props();

  /** The newest chats shown first, before the rest by name. */
  const RECENT = 6;

  let query = $state('');
  let chosen = $state<string[]>([]);
  let hideSender = $state(false);
  let comment = $state('');
  let busy = $state(false);
  let error = $state('');

  // A send outlives its sheet (the sheet closes while a file goes up again):
  // its answer belongs to the opening it was started in.
  const sessions = new ForwardSessions();
  let wasOpen = false;
  $effect(() => {
    if (open !== wasOpen) sessions.turn();
    if (open && !wasOpen) {
      query = ''; chosen = []; hideSender = false; comment = ''; error = ''; busy = false;
      chatStore.loadChats().catch(() => {});
    }
    wasOpen = open;
  });

  // The system «Назад» closes the sheet, not the chat under it.
  $effect(() => {
    if (!open) return;
    return untrack(() => holdBack(() => (open = false), 'forwardSheet'));
  });

  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const fit = $derived(chatStore.chats.filter(forwardTarget));
  const found = $derived(query.trim() ? fit.filter((c) => chatMatches(c, query, haystacks)) : null);
  const recent = $derived(
    [...fit].filter((c) => c.last_message_at).sort((a, b) => (b.last_message_at ?? 0) - (a.last_message_at ?? 0)).slice(0, RECENT),
  );
  const rest = $derived(fit.filter((c) => !recent.includes(c)).sort((a, b) => a.title.localeCompare(b.title)));
  const picked = $derived(chosen.map((id) => fit.find((c) => c.id === id)).filter((c): c is MessengerChat => !!c));
  const full = $derived(chosen.length >= MAX_FORWARD_TARGETS);

  function toggle(c: MessengerChat) {
    if (busy) return;
    if (chosen.includes(c.id)) chosen = chosen.filter((id) => id !== c.id);
    else if (!full) chosen = [...chosen, c.id];
  }

  const titleOf = (id: string) => chatStore.chats.find((c) => c.id === id)?.title ?? '';

  /** A refusal of the whole forward, or of one chat (`title`), in words. */
  function explain(e: unknown, title = ''): string {
    const text = messengerError(e);
    const code = /\bforward_[a-z_]+\b/.exec(text)?.[0];
    if (code === 'forward_message_unfit') return $t('msg_fwd_err_message_unfit');
    if (code === 'forward_target_unfit') return $t('msg_fwd_err_target_unfit');
    if (code === 'forward_too_many_messages') return $t('msg_fwd_limit', { n: String(MAX_FORWARD_MESSAGES) });
    if (code === 'forward_too_many_targets') return $t('msg_fwd_targets_limit', { n: String(MAX_FORWARD_TARGETS) });
    const dm = dmErrorCode(e);
    if (dm) return $t(`msg_err_${dm}` as 'msg_err_dm_blocked', { name: title });
    return mediaErrorText(e, (key) => $t(key));
  }

  async function send() {
    if (busy || !chosen.length || !ids.length) return;
    const opening = sessions.start();
    error = ''; busy = true;
    try {
      const out = forwardOutcome(await chatStore.forward(chatId, ids, chosen, hideSender, comment));
      if (!out.short.length) {
        const n = out.done.length;
        const one = n === 1 ? out.done[0] : null;
        toast.success($t(countKey('msg_fwd_done', n, $locale), { n: String(n) }),
          one ? { action: $t('msg_fwd_open'), onAction: () => void openChat(one), duration: 5000 } : {});
      } else {
        const lines = out.short.map((s) => {
          const title = titleOf(s.target);
          const why = s.error ? explain(s.error, title) : mediaErrorText(s.skipped[0], (key) => $t(key));
          return title ? `${title}: ${why}` : why;
        });
        toast.error($t('msg_fwd_short'), { sub: lines.join('; '), action: false });
      }
      // Closed or opened again meanwhile: the mode of choosing is no longer what was sent.
      if (!sessions.live(opening)) return;
      open = false;
      onforwarded?.();
    } catch (e) {
      // Its sheet is gone: the toast tells it, the new sheet keeps its own state.
      if (!sessions.live(opening)) toast.error(explain(e), { action: false });
      else error = explain(e);
    } finally {
      if (sessions.live(opening)) busy = false;
    }
  }
</script>

{#snippet row(c: MessengerChat)}
  {@const on = chosen.includes(c.id)}
  <li>
    <button class="row" class:on disabled={busy || (!on && full)} onclick={() => toggle(c)} aria-pressed={on}>
      <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={34} />
      <span class="name">{c.title}</span>
      {#if c.kind === 'group'}<span class="dim"><Icon name="users" size={12} /></span>{/if}
      <span class="tick" class:on aria-hidden="true">{#if on}<Icon name="check" size={12} />{/if}</span>
    </button>
  </li>
{/snippet}

{#snippet footer()}
  <button class="btn btn-primary send" disabled={busy || !chosen.length} onclick={send}>
    {#if busy}<span class="spin"><Icon name="loader" size={14} /></span>{$t('msg_fwd_sending')}
    {:else}<Icon name="forward" size={14} />{$t('msg_fwd_send', { n: String(chosen.length) })}{/if}
  </button>
{/snippet}

<Dialog bind:open title={$t('msg_fwd_forward')} width="min(440px, calc(100vw - 24px))" expandable {footer}>
  <div class="body">
    <input type="text" class="field" bind:value={query} placeholder={$t('msg_share_search')} spellcheck="false" disabled={busy} />
    {#if error}<div class="error-msg">{error}</div>{/if}
    <div class="lists">
      {#if found}
        {#if found.length}<ul class="list divided">{#each found as c (c.id)}{@render row(c)}{/each}</ul>
        {:else}<p class="hint">{$t('msg_fwd_empty')}</p>{/if}
      {:else if fit.length}
        {#if recent.length}
          <div class="section">{$t('msg_fwd_recent')}</div>
          <ul class="list divided">{#each recent as c (c.id)}{@render row(c)}{/each}</ul>
        {/if}
        {#if rest.length}
          <div class="section">{$t('msg_fwd_all')}</div>
          <ul class="list divided">{#each rest as c (c.id)}{@render row(c)}{/each}</ul>
        {/if}
      {:else}
        <p class="hint">{$t('msg_fwd_empty')}</p>
      {/if}
    </div>
    {#if picked.length}
      <div class="picked">
        {#each picked as c (c.id)}
          <button class="chip" disabled={busy} onclick={() => toggle(c)} title={$t('msg_fwd_remove', { name: c.title })} aria-label={$t('msg_fwd_remove', { name: c.title })}>
            <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={32} />
            <span class="chip-x" aria-hidden="true"><Icon name="x" size={9} /></span>
          </button>
        {/each}
      </div>
      {#if full}<p class="hint">{$t('msg_fwd_targets_limit', { n: String(MAX_FORWARD_TARGETS) })}</p>{/if}
    {/if}
    <div class="option">
      <div class="option-info">
        <span>{$t('msg_fwd_hide_sender')}</span>
        <span class="hint">{hideSender ? $t('msg_fwd_hide_sender_on') : $t('msg_fwd_hide_sender_off')}</span>
      </div>
      <button class="toggle" class:on={hideSender} role="switch" aria-checked={hideSender} aria-label={$t('msg_fwd_hide_sender')}
        disabled={busy} onclick={() => (hideSender = !hideSender)}></button>
    </div>
    <input type="text" class="field" bind:value={comment} placeholder={$t('msg_fwd_comment')} disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); void send(); } }} />
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); min-height: 0; }
  .field {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .lists { max-height: 320px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .section { font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--text-3); text-transform: uppercase; letter-spacing: 0.04em; padding: var(--sp-2) 8px 2px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: 6px 8px; text-align: left;
    border: none; border-radius: var(--radius-sm); background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .row:disabled { cursor: default; opacity: 0.5; }
  .row.on { background: var(--accent-bg); }
  .name { flex: 1; min-width: 0; font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; }
  .tick {
    flex-shrink: 0; width: 20px; height: 20px; border-radius: 50%; border: 1.5px solid var(--border-strong, var(--text-3));
    display: inline-flex; align-items: center; justify-content: center; color: #fff;
  }
  .tick.on { background: var(--accent); border-color: var(--accent); }
  /* The chosen chats: their faces in a strip, a tap takes one out. */
  .picked { display: flex; gap: var(--sp-2); overflow-x: auto; padding: 2px 0; }
  .chip { position: relative; flex-shrink: 0; padding: 0; border: none; background: none; cursor: pointer; border-radius: 50%; }
  .chip-x {
    position: absolute; right: -2px; bottom: -2px; width: 16px; height: 16px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center; background: var(--text-2); color: var(--surface); border: 2px solid var(--surface);
  }
  /* «Скрыть отправителя»: the words on the left, the switch (base.css `.toggle`) on the right. */
  .option { display: flex; align-items: center; gap: var(--sp-3); }
  .option-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-sm); line-height: 1.4; }
  .toggle:disabled { opacity: 0.5; cursor: default; }
  .hint { margin: 0; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; }
  .send { display: inline-flex; align-items: center; justify-content: center; gap: 6px; }
  .send :global(svg) { flex-shrink: 0; }
  .spin { display: inline-flex; animation: spin 1.1s linear infinite; }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
  @media (pointer: coarse) { .field { font-size: var(--fs-md); } .row { padding: 9px 8px; } .lists { max-height: none; } }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Отправить в…»: what another app shared into Veydan (a text, photos,
  videos, files; core/shares) and the chat it goes to, over any screen. One
  chat chosen opens with the share in its composer, to add a word and send;
  several chosen get it from here at once, with a comment. Beside the chats
  the other modules' places («Новая заметка»). The list is the forward's
  (ForwardDialog): the chats a forward may go to, the newest first.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import { holdBack } from '$lib/core/ui/sheetBack';
  import { BACK_WAIT_MS } from '$lib/core/ui/back';
  import type { ShareTarget } from '$lib/core/module';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { messengerStore } from '../store.svelte';
  import { dmErrorCode, messengerError, type MessengerChat } from '../api';
  import { chatMatches, contactHaystacks } from '../shared/search';
  import { mediaErrorText } from '../media/errors';
  import { openChat } from '../content/actions';
  import { shareStore } from './shareStore.svelte';
  import { filesLine, kindOf, MAX_SHARE_TARGETS, shareTarget } from './shareIn';

  const share = $derived(shareStore.pending);
  const open = $derived(share !== null);

  /** The newest chats shown first, before the rest by name. */
  const RECENT = 6;

  let query = $state('');
  let chosen = $state<string[]>([]);
  let comment = $state('');
  let busy = $state(false);
  let error = $state('');

  // A new share is a new choice.
  let lastShare: typeof share = null;
  $effect(() => {
    if (share !== lastShare) {
      lastShare = share;
      query = ''; chosen = []; comment = ''; error = ''; busy = false;
      if (share) chatStore.loadChats().catch(() => {});
    }
  });

  // The system «Назад» closes the sheet, not the screen under it.
  $effect(() => {
    if (!open) return;
    return untrack(() => holdBack(close, 'shareInSheet'));
  });

  function close() {
    if (busy) return;
    shareStore.close();
  }

  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const fit = $derived(chatStore.chats.filter(shareTarget));
  const found = $derived(query.trim() ? fit.filter((c) => chatMatches(c, query, haystacks)) : null);
  const recent = $derived(
    [...fit].filter((c) => c.last_message_at).sort((a, b) => (b.last_message_at ?? 0) - (a.last_message_at ?? 0)).slice(0, RECENT),
  );
  const rest = $derived(fit.filter((c) => !recent.includes(c)).sort((a, b) => a.title.localeCompare(b.title)));
  const picked = $derived(chosen.map((id) => fit.find((c) => c.id === id)).filter((c): c is MessengerChat => !!c));
  const full = $derived(chosen.length >= MAX_SHARE_TARGETS);

  /** What is shared, in a line: the first files and how many more. */
  const files = $derived(share ? filesLine(share.files) : { names: [], more: 0 });
  const KIND_ICON = { image: 'image', video: 'video', audio: 'music', file: 'file-text' } as const;
  const icon = $derived(share?.files.length ? KIND_ICON[kindOf(share.files[0])] : 'file-text');

  function toggle(c: MessengerChat) {
    if (busy) return;
    if (chosen.includes(c.id)) chosen = chosen.filter((id) => id !== c.id);
    else if (!full) chosen = [...chosen, c.id];
  }

  const titleOf = (id: string) => chatStore.chats.find((c) => c.id === id)?.title ?? '';

  /** A refusal of one chat (`title`), in words. */
  function explain(e: unknown, title = ''): string {
    const dm = dmErrorCode(e);
    if (dm) return $t(`msg_err_${dm}` as 'msg_err_dm_blocked', { name: title });
    return mediaErrorText(e, (key) => $t(key)) || messengerError(e);
  }

  /**
   * The sheet holds a place in the history while it is open (holdBack): the
   * chat is gone to only once the sheet has closed and given its place back,
   * else the back it owes would pop the chat's page.
   */
  function closedFirst(): Promise<void> {
    shareStore.handOver();
    return new Promise((resolve) => {
      const done = () => { clearTimeout(timer); window.removeEventListener('popstate', done); resolve(); };
      const timer = setTimeout(done, BACK_WAIT_MS);
      window.addEventListener('popstate', done);
    });
  }

  async function go() {
    const s = share;
    // Read before the sheet closes: a closed sheet forgets its choice.
    const targets = picked;
    if (busy || !s || !targets.length) return;
    if (targets.length === 1) {
      busy = true;
      try {
        await closedFirst();
        await shareStore.toChat(s, targets[0].id);
      } catch (e) {
        error = explain(e);
      } finally {
        busy = false;
      }
      return;
    }
    busy = true;
    error = '';
    try {
      const out = await shareStore.sendToMany(s, targets, comment);
      if (!out.short.length) {
        const n = out.done.length;
        const one = n === 1 ? out.done[0] : null;
        toast.success($t(countKey('msg_sharein_done', n, $locale), { n: String(n) }),
          one ? { action: $t('common_open'), onAction: () => void openChat(one), duration: 5000 } : {});
      } else {
        const lines = out.short.map((x) => {
          const title = titleOf(x.target);
          const why = explain(x.error, title);
          return title ? `${title}: ${why}` : why;
        });
        toast.error($t('msg_sharein_short'), { sub: lines.join('; '), action: false });
      }
    } finally {
      busy = false;
    }
  }

  /** Another module's place («Новая заметка»): the share is its from here on. */
  async function elsewhere(target: ShareTarget) {
    const s = share;
    if (busy || !s) return;
    busy = true;
    try {
      await closedFirst();
      await target.take(s);
    } catch (e) {
      toast.error(messengerError(e), { action: false });
    } finally {
      busy = false;
    }
  }
</script>

{#snippet row(c: MessengerChat)}
  {@const on = chosen.includes(c.id)}
  <li>
    <button class="row" class:on disabled={busy || (!on && full)} onclick={() => toggle(c)} aria-pressed={on}>
      <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={36} />
      <span class="name">{c.title}</span>
      {#if c.kind === 'group'}<span class="dim"><Icon name="users" size={12} /></span>{/if}
      <span class="tick" class:on aria-hidden="true">{#if on}<Icon name="check" size={12} />{/if}</span>
    </button>
  </li>
{/snippet}

{#snippet footer()}
  <button class="btn btn-primary send" disabled={busy || !picked.length} onclick={go}>
    {#if busy}<span class="spin"><Icon name="loader" size={14} /></span>{$t('msg_fwd_sending')}
    {:else if picked.length > 1}<Icon name="send" size={14} />{$t('msg_fwd_send', { n: String(picked.length) })}
    {:else}<Icon name="message-circle" size={14} />{$t('msg_sharein_open')}{/if}
  </button>
{/snippet}

<Dialog {open} title={$t('msg_sharein_title')} width="min(440px, calc(100vw - 24px))" expandable onclose={close} {footer}>
  {#if share}
    <div class="body">
      <!-- What came: the files by name, or the text. -->
      <div class="what">
        <span class="what-ico"><Icon name={share.files.length ? icon : 'file-text'} size={18} /></span>
        <div class="what-text">
          {#if share.files.length}
            <span class="what-main">{$t(countKey('msg_sharein_files', share.files.length, $locale), { n: String(share.files.length) })}</span>
            <span class="what-sub">{files.names.join(', ')}{files.more ? ` +${files.more}` : ''}</span>
          {:else}
            {#if share.subject}<span class="what-main">{share.subject}</span>{/if}
            <span class="what-sub" class:what-main={!share.subject}>{share.text}</span>
          {/if}
        </div>
      </div>
      {#if share.refused.length}<Notice tone="warn">{$t('msg_sharein_refused', { names: share.refused.join(', ') })}</Notice>{/if}
      {#if error}<Notice tone="danger">{error}</Notice>{/if}
      <input type="text" class="field" bind:value={query} placeholder={$t('msg_share_search')} spellcheck="false" disabled={busy} />
      <div class="lists">
        {#if shareStore.others.length && !found}
          <ul class="list divided">
            {#each shareStore.others as other (other.title)}
              <li>
                <button class="row" disabled={busy} onclick={() => elsewhere(other)}>
                  <span class="place"><Icon name={other.icon} size={18} /></span>
                  <span class="name">{$t(other.title)}</span>
                  <span class="dim"><Icon name="chevron-right" size={14} /></span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
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
        {#if full}<p class="hint">{$t('msg_fwd_targets_limit', { n: String(MAX_SHARE_TARGETS) })}</p>{/if}
      {/if}
      {#if picked.length > 1}
        <input type="text" class="field" bind:value={comment} placeholder={$t('msg_fwd_comment')} disabled={busy}
          onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); void go(); } }} />
      {/if}
    </div>
  {/if}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); min-height: 0; }
  /* What is shared: an icon of its kind, the count or the text, the names. */
  .what { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; padding: var(--sp-2) var(--sp-2) 0; }
  .what-ico {
    width: 38px; height: 38px; flex-shrink: 0; border-radius: var(--radius-md); display: inline-flex; align-items: center; justify-content: center;
    background: var(--accent-bg); color: var(--accent);
  }
  .what-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; font-size: var(--fs-sm); line-height: var(--lh-snug); }
  .what-main { font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .what-sub { color: var(--text-2); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; word-break: break-word; }
  .field {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .lists { max-height: 320px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .section { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); padding: var(--sp-2) 8px 2px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 48px; padding: var(--sp-1) var(--sp-2); text-align: left;
    border: none; border-radius: var(--radius-md); background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .row:disabled { cursor: default; opacity: 0.45; }
  .row.on { background: var(--accent-bg); }
  /* Another module's place, drawn as a face of its own. */
  .place {
    width: 36px; height: 36px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .name { flex: 1; min-width: 0; font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; }
  .tick {
    flex-shrink: 0; width: 20px; height: 20px; border-radius: 50%; border: 1.5px solid var(--border-strong, var(--text-3));
    display: inline-flex; align-items: center; justify-content: center; color: var(--on-accent);
  }
  .tick.on { background: var(--accent); border-color: var(--accent); }
  .picked { display: flex; gap: var(--sp-2); overflow-x: auto; padding: 2px 0; }
  .chip { position: relative; flex-shrink: 0; padding: 0; border: none; background: none; cursor: pointer; border-radius: 50%; }
  .chip-x {
    position: absolute; right: -2px; bottom: -2px; width: 16px; height: 16px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center; background: var(--text-2); color: var(--surface); border: 2px solid var(--surface);
  }
  .hint { margin: 0; line-height: var(--lh-snug); color: var(--text-2); font-size: var(--fs-sm); }
  .send { display: inline-flex; align-items: center; justify-content: center; gap: 6px; }
  .send :global(svg) { flex-shrink: 0; }
  .spin { display: inline-flex; animation: spin 1.1s linear infinite; }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
  @media (pointer: coarse) { .field { font-size: var(--fs-md); } .row { min-height: 56px; } .lists { max-height: none; } }
</style>

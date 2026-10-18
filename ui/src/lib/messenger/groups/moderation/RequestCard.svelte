<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One request to join, as a card: who asks (their name, address and key),
  what they wrote, when it came, what this device knows of them without
  asking the network, and the answers. A stranger's picture is never
  fetched for it: Avatar shows what the runtime already holds.
-->
<script lang="ts">
  import { locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { copied, toast } from '$lib/core/ui/toast.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import { messengerError, type MessengerGroupRequest } from '../../api';
  import { cameAt, claimedApart, shortNpub, waited, waitedText, type RequestFacts } from './requests';

  interface Props {
    request: MessengerGroupRequest;
    facts: RequestFacts;
    selected: boolean;
    /** Its answers are off: an act on it, or on many, is under way. */
    busy: boolean;
    /** Seconds now, ticking, so «ждёт» moves on its own. */
    now: number;
    ontick: () => void;
    onanswer: (approve: boolean) => void;
    onmenu: (at: { x: number; y: number }) => void;
  }
  let { request, facts, selected, busy, now, ontick, onanswer, onmenu }: Props = $props();

  let noteBox = $state<HTMLElement | null>(null);
  let open = $state(false);
  let over = $state(false);

  // «ещё» only when the clamped note is cut.
  $effect(() => {
    const el = noteBox;
    if (!el) { over = false; return; }
    void request.note;
    const measure = () => { if (!open) over = el.scrollHeight > el.clientHeight + 1; };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });

  const lang = $derived(localeTag($locale));
  const came = $derived(cameAt(request.received_at, new Date(now * 1000), lang, (k) => $t(`msg_day_${k}` as 'msg_day_today')));
  const wait = $derived(waited(request.received_at, now));
  const waitText = $derived(wait ? $t('msg_greq_waiting', { time: waitedText(wait, lang) }) : $t('msg_greq_just_now'));
  const claimed = $derived(claimedApart(request) ? cameAt(request.created_at, new Date(now * 1000), lang, (k) => $t(`msg_day_${k}` as 'msg_day_today')) : null);

  async function copyKey() {
    try {
      await navigator.clipboard.writeText(request.npub);
      copied();
    } catch (e) {
      toast.error(messengerError(e));
    }
  }

  function menuAt(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onmenu({ x: r.right, y: r.bottom + 4 });
  }
</script>

<li class="card" class:selected aria-label={facts.name}>
  <div class="head">
    <input class="tick" type="checkbox" checked={selected} disabled={busy} onchange={ontick} aria-label={$t('msg_greq_select', { name: facts.name })} />
    <Avatar url={facts.picture} label={facts.name} seed={request.requester} size={40} />
    <div class="who">
      <span class="name" title={facts.name}>{facts.name}</span>
      {#if facts.nip05}
        <span class="nip05" class:ok={facts.nip05Verified} title={facts.nip05Verified ? $t('msg_greq_nip05_ok') : $t('msg_greq_nip05_unchecked')}>
          <Icon name={facts.nip05Verified ? 'rosette-discount-check' : 'world'} size={12} /><span class="nip05-text">{facts.nip05}</span>
        </span>
      {/if}
      <button type="button" class="npub mono" title={$t('msg_greq_copy_key')} aria-label={$t('msg_greq_copy_key')} onclick={copyKey}>
        {shortNpub(request.npub)}<Icon name="copy" size={11} />
      </button>
    </div>
    <button type="button" class="icon-btn more" disabled={busy} title={$t('msg_greq_actions')} aria-label={$t('msg_greq_actions')} onclick={menuAt}>
      <Icon name="more-vertical" size={16} />
    </button>
  </div>

  {#if request.note}
    <blockquote class="note"><span class="note-text" class:open class:over={over && !open} bind:this={noteBox}>{request.note}</span></blockquote>
    {#if over || open}
      <button type="button" class="note-more" onclick={() => (open = !open)} aria-expanded={open}>{open ? $t('msg_greq_less') : $t('msg_greq_more')}</button>
    {/if}
  {:else}
    <p class="note-none">{$t('msg_greq_no_note')}</p>
  {/if}

  <div class="facts">
    {#if facts.contact}
      <span class="fact ok"><Icon name="book-user" size={12} />{$t('msg_greq_contact')}</span>
      {#if facts.talked}<span class="fact"><Icon name="message-circle" size={12} />{$t('msg_greq_talked')}</span>{/if}
    {:else}
      <span class="fact"><Icon name="user-question" size={12} />{$t('msg_greq_stranger')}</span>
    {/if}
    {#if facts.common > 0}<span class="fact"><Icon name="users" size={12} />{$t('msg_greq_common', { n: String(facts.common) })}</span>{/if}
    {#if facts.missing}<span class="fact warn"><Icon name="info" size={12} />{$t('msg_greq_no_profile')}</span>{/if}
  </div>
  {#if facts.about}<p class="about" title={facts.about}>{facts.about}</p>{/if}

  <div class="foot">
    <span class="when">
      <Icon name="clock" size={12} />
      <span>{came}</span><span class="sep">·</span><span class="wait">{waitText}</span>
      {#if claimed}<span class="sep">·</span><span class="claimed" title={$t('msg_greq_their_clock_hint')}>{$t('msg_greq_their_clock', { time: claimed })}</span>{/if}
    </span>
    <span class="acts">
      <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => onanswer(false)}>{$t('msg_group_request_reject')}</button>
      <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => onanswer(true)}><Icon name="check" size={14} />{$t('msg_group_request_approve')}</button>
    </span>
  </div>
</li>

<style>
  .card {
    list-style: none; display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-lg); min-width: 0;
    transition: background var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease);
  }
  .card.selected { background: var(--accent-bg); border-color: var(--accent-border); }
  .head { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; }
  .tick { margin-top: 11px; flex-shrink: 0; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding-top: 1px; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nip05 { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); min-width: 0; }
  .nip05.ok { color: var(--success-text, var(--success)); }
  .nip05 :global(svg) { flex-shrink: 0; }
  .nip05-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .npub {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 5px; max-width: 100%;
    border: none; background: none; padding: 0; cursor: pointer; font-size: var(--fs-2xs); color: var(--text-3);
  }
  .npub:hover { color: var(--accent-text); }
  .more { flex-shrink: 0; }
  .note {
    margin: 0; padding: var(--sp-2) var(--sp-3); border-left: 3px solid var(--accent-border); border-radius: var(--radius-xs);
    background: var(--surface); font-size: var(--fs-sm); line-height: 1.45; color: var(--text-body);
  }
  /* Five lines, the rest on «ещё»; the padding is the quote's, so no part of a sixth line shows. */
  .note-text {
    display: block; white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word;
    max-height: calc(5 * 1.45em); overflow: hidden;
  }
  .note-text.over { mask-image: linear-gradient(to bottom, #000 calc(100% - 1.45em), transparent); }
  .note-text.open { max-height: none; overflow: visible; }
  .note-more {
    align-self: flex-start; margin-top: calc(-1 * var(--sp-1)); border: none; background: none; padding: 0 var(--sp-3); cursor: pointer;
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text);
  }
  .note-none { margin: 0; font-size: var(--fs-sm); color: var(--text-3); font-style: italic; }
  .facts { display: flex; flex-wrap: wrap; gap: 6px; }
  .fact {
    display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--text-2); background: var(--surface-3); border: 1px solid var(--border);
  }
  .fact.ok { color: var(--success-text, var(--success)); }
  .fact.warn { color: var(--warn-text); }
  .about { margin: 0; font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .foot { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2) var(--sp-3); flex-wrap: wrap; padding-top: 2px; }
  .when { display: inline-flex; align-items: center; flex-wrap: wrap; gap: 2px 5px; font-size: var(--fs-2xs); color: var(--text-3); min-width: 0; }
  .when :global(svg) { flex-shrink: 0; }
  .wait { color: var(--text-2); font-weight: var(--fw-semibold); }
  .claimed { cursor: help; text-decoration: underline dotted; text-underline-offset: 2px; }
  .acts { display: flex; gap: 6px; margin-left: auto; }
  @media (pointer: coarse) {
    .tick { margin-top: 10px; }
    .npub { padding: 4px 0; font-size: var(--fs-xs); }
    .acts { flex: 1 1 100%; }
    .acts .btn { flex: 1; justify-content: center; min-height: var(--control-h); }
  }
</style>

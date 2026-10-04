<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A message came while the app is on the screen: the system notification
  stays away (the push handler knows the app is up), and a card here says
  it instead, unless the chat it is about is open.

  Up to three cards, the newest on top; a chat that writes again updates
  its card and counts. A line at the bottom runs out with the card's time;
  a finger on the card holds it. Swiped up or aside, the card goes; tapped,
  it opens the chat.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { BASE, chatHref } from '../mobile/routes';
  import { albumLine, albumOf, bodyLine, joinAlbum, type Album, type NoticeBody } from './wording';

  /** `os`: a computer's system shows it already (the window was not on the screen). */
  interface Notify { title: string; body: NoticeBody | null; chat_id: string | null; sender: string | null; request?: boolean; os?: boolean }

  interface Card {
    key: string;
    chatId: string | null;
    title: string;
    group: boolean;
    request: boolean;
    sender: string | null;
    text: string;
    /** Files sent together, while they come one after another. */
    album: Album | null;
    count: number;
    /** Changes with every message: restarts the line. */
    round: number;
    held: boolean;
    dx: number;
    dy: number;
    gone: boolean;
  }

  /**
   * `onopen`: how the shell opens a chat (a desk has it in place); a phone
   * goes to the chat's page. `desk`: a computer, with a mouse: the cards sit
   * under the title bar and each has a cross to close it.
   */
  let { onopen, desk = false }: { onopen?: (chatId: string | null) => void; desk?: boolean } = $props();

  const SHOWN_FOR_MS = 5000;
  const MAX_CARDS = 3;
  const SWIPE_ASIDE = 90;
  const SWIPE_UP = 40;
  const MOVED = 8;

  let cards = $state<Card[]>([]);
  let seenAt = 0;
  let round = 0;

  $effect(() => {
    const head = messengerStore.feed[0];
    if (!head || head.name !== 'notify' || head.at <= seenAt) return;
    seenAt = head.at;
    arrive(head.payload as Notify);
  });

  function arrive(n: Notify) {
    if (n.os) return;
    // The chat it is about is on the screen: the message is already there.
    if (n.chat_id && page.url.pathname.startsWith(BASE) && chatStore.activeId === n.chat_id) return;
    const chat = n.chat_id ? chatStore.chats.find((c) => c.id === n.chat_id) : undefined;
    const group = chat?.kind === 'group' || (n.chat_id?.startsWith('group:') ?? false);
    const key = n.chat_id ?? `note-${++round}`;
    const old = cards.find((c) => c.key === key);
    // One more of an album that is coming: the card counts them.
    const joined = joinAlbum(old?.album, albumOf(n.body));
    const album = joined ?? albumOf(n.body);
    const text = joined ? albumLine(joined, $t) : bodyLine(n.body, $t);
    const card: Card = {
      key,
      chatId: n.chat_id,
      title: n.title,
      group,
      request: n.request ?? chat?.mode === 'request_received',
      sender: n.sender,
      text,
      album,
      // The album says how many it has; the count is of the messages besides.
      count: old ? old.count + (joined ? 0 : 1) : 1,
      round: ++round,
      held: false,
      dx: 0,
      dy: 0,
      gone: false,
    };
    cards = [card, ...cards.filter((c) => c.key !== key)].slice(0, MAX_CARDS);
  }

  /** The face on the card: the one who wrote. */
  function face(card: Card): { url: string | null; label: string; seed: string | null } {
    if (card.sender) {
      const chat = card.chatId ? chatStore.chats.find((c) => c.id === card.chatId) : undefined;
      const url = !card.group && chat?.picture ? chat.picture : nameStore.picture(card.sender);
      return { url, label: nameStore.label(card.sender), seed: card.sender };
    }
    return { url: null, label: card.title, seed: card.chatId };
  }

  function dismiss(key: string) {
    cards = cards.filter((c) => c.key !== key);
  }

  function leave(card: Card) {
    card.gone = true;
    setTimeout(() => dismiss(card.key), 180);
  }

  function open(card: Card) {
    dismiss(card.key);
    if (onopen) onopen(card.chatId);
    else goto(card.chatId ? chatHref(card.chatId) : BASE);
  }

  // ─── Touch ────────────────────────────────────────────────────────────

  let start: { x: number; y: number; key: string } | null = null;

  function down(e: PointerEvent, card: Card) {
    start = { x: e.clientX, y: e.clientY, key: card.key };
    card.held = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function move(e: PointerEvent, card: Card) {
    if (!start || start.key !== card.key) return;
    card.dx = e.clientX - start.x;
    // Only upwards: a card pulled down goes nowhere.
    card.dy = Math.min(0, e.clientY - start.y);
  }

  function up(card: Card) {
    if (!start || start.key !== card.key) return;
    start = null;
    card.held = false;
    const moved = Math.hypot(card.dx, card.dy);
    if (Math.abs(card.dx) > SWIPE_ASIDE || card.dy < -SWIPE_UP) {
      leave(card);
    } else if (moved < MOVED) {
      open(card);
    } else {
      card.dx = 0;
      card.dy = 0;
    }
  }

  function cancel(card: Card) {
    start = null;
    card.held = false;
    card.dx = 0;
    card.dy = 0;
  }
</script>

{#if cards.length}
  <div class="stack" class:desk aria-live="polite">
    {#each cards as card (card.key)}
      {@const f = face(card)}
      <div
        class="card"
        class:held={card.held}
        class:gone={card.gone}
        style="--dx:{card.dx}px; --dy:{card.dy}px; --fade:{1 - Math.min(Math.abs(card.dx) / 240, 0.6)}"
        role="button"
        tabindex="0"
        onpointerdown={(e) => down(e, card)}
        onpointermove={(e) => move(e, card)}
        onpointerup={() => up(card)}
        onpointercancel={() => cancel(card)}
        onkeydown={(e) => { if (e.key === 'Enter') open(card); if (e.key === 'Escape') leave(card); }}
      >
        <div class="face">
          <Avatar url={f.url} label={f.label} seed={f.seed} size={40} />
          {#if card.group}<span class="badge"><Icon name="users" size={10} /></span>{/if}
        </div>
        <div class="text">
          <div class="head">
            <span class="title">{card.group || !card.sender ? card.title : nameStore.label(card.sender)}</span>
            {#if card.count > 1}<span class="more">+{card.count - 1}</span>{/if}
            <span class="when">{$t('msg_notice_now')}</span>
            {#if desk}
              <button
                class="close"
                aria-label={$t('msg_notice_close')}
                title={$t('msg_notice_close')}
                onpointerdown={(e) => e.stopPropagation()}
                onkeydown={(e) => e.stopPropagation()}
                onclick={(e) => { e.stopPropagation(); leave(card); }}
              ><Icon name="x" size={14} /></button>
            {/if}
          </div>
          {#if card.request}<div class="label">{$t('msg_notice_request')}</div>{/if}
          <div class="body">
            {#if card.group && card.sender}<span class="who">{nameStore.label(card.sender)}:</span>{/if}
            {card.text}
          </div>
        </div>
        {#key card.round}
          <div
            class="line"
            class:paused={card.held}
            style="animation-duration:{SHOWN_FOR_MS}ms"
            onanimationend={() => leave(card)}
          ></div>
        {/key}
      </div>
    {/each}
  </div>
{/if}

<style>
  .stack {
    position: fixed; top: calc(var(--sat) + 8px); left: 12px; right: 12px; z-index: 70;
    display: flex; flex-direction: column; gap: 8px; align-items: center;
    pointer-events: none;
  }
  /* A desk: in the middle, under the navigation bar. */
  .stack.desk { top: 72px; }
  .close {
    flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center;
    width: 22px; height: 22px; margin: -3px -4px -3px 0; padding: 0; border: none; border-radius: var(--radius-sm);
    background: transparent; color: var(--text-3); cursor: pointer;
  }
  .close:hover { background: var(--surface-2); color: var(--text); }
  .card {
    position: relative; overflow: hidden; pointer-events: auto; touch-action: none; user-select: none;
    width: 100%; max-width: 420px;
    display: flex; align-items: flex-start; gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-3) calc(var(--sp-3) + 2px);
    border: 1px solid var(--border); border-radius: var(--radius-lg);
    background: var(--surface); color: var(--text);
    box-shadow: 0 10px 28px rgb(0 0 0 / 0.18);
    transform: translate(var(--dx), var(--dy)); opacity: var(--fade);
    transition: transform 180ms ease, opacity 180ms ease;
    animation: drop 260ms cubic-bezier(0.2, 0.9, 0.3, 1.2);
    cursor: pointer;
  }
  .card.held { transition: none; }
  .card.gone { transform: translate(calc(var(--dx) * 3), calc(var(--dy) - 120px)); opacity: 0; }
  .face { position: relative; flex-shrink: 0; }
  .badge {
    position: absolute; right: -3px; bottom: -3px; width: 18px; height: 18px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center;
    background: var(--accent); color: #fff; border: 2px solid var(--surface);
  }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .head { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  .title { font-weight: var(--fw-semibold); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .more {
    flex-shrink: 0; font-size: var(--fs-2xs); font-weight: var(--fw-semibold);
    padding: 0 6px; border-radius: var(--radius-pill); background: var(--accent-tint); color: var(--accent-text-2);
  }
  .when { margin-left: auto; flex-shrink: 0; font-size: var(--fs-2xs); color: var(--text-3); }
  .label { font-size: var(--fs-2xs); color: var(--accent-text-2); }
  .body {
    font-size: var(--fs-sm); color: var(--text-2); line-height: 1.35;
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere;
  }
  .who { color: var(--text); font-weight: var(--fw-medium, 500); }
  .line {
    position: absolute; left: 0; bottom: 0; height: 2px; width: 100%;
    background: var(--accent); transform-origin: left center;
    animation-name: run; animation-timing-function: linear; animation-fill-mode: forwards;
  }
  .line.paused { animation-play-state: paused; }
  @keyframes run { from { transform: scaleX(1); } to { transform: scaleX(0); } }
  @keyframes drop { from { transform: translateY(-24px); opacity: 0; } to { transform: none; opacity: 1; } }
</style>

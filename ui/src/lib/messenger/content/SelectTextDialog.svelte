<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Выбрать текст»: the words of one message in its bubble, over the chat
  blurred past reading, to be selected by a finger or the mouse. The chat's
  head stays sharp (whom it is with is no secret), the window under it; a
  tap on the head closes the window, as one beside the bubble. On a computer
  only the chat is blurred, the list of chats and all round it stay sharp
  (there one shoots the part of the screen one wants; a phone shoots it all),
  a tap on them closing the window too. The text is
  drawn as in the bubble, but is no field: a phone shows no keyboard, its own
  handles and «Копировать» work. Links are shown and lead nowhere here.
  Closed by ×, a tap beside the bubble (unless something is selected),
  the system «Назад» and Esc.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { portal } from '$lib/core/ui/portal';
  import { layer } from '$lib/core/ui/layer';
  import { backdropDismiss } from '$lib/core/ui/backdrop';
  import { backdropIn, backdropOut, dialogIn, dialogOut } from '$lib/core/ui/motion';
  import { holdBack } from '$lib/core/ui/sheetBack';
  import { toast } from '$lib/core/ui/toast.svelte';
  import { clock } from '../shared/time';
  import { tint } from '../shared/tint';
  import type { MessengerMessage } from '../api';
  import MessageContent from './MessageContent.svelte';

  interface Props {
    /** The message shown; null closes the window. */
    message: MessengerMessage | null;
    /** Chats of many: the name of whoever wrote it, over its words. */
    author?: (pubkey: string) => string;
    /** Left sharp, not blurred: the chat's head. */
    clear?: HTMLElement | null;
    /** What is blurred, round the head; none: the whole screen. */
    area?: HTMLElement | null;
    onclose: () => void;
  }
  let { message: m, author, clear = null, area = null, onclose }: Props = $props();

  const out = $derived(m?.direction === 'out');

  // The blur lies round the head, in four strips within the area: one blur over all
  // with a hole would not do, the phone's head is glass and blurs what is under it itself.
  // All is measured from the overlay; the area's sides, as the padding, from its own.
  let overlay = $state<HTMLElement>();
  let span = $state({ top: 0, left: 0, right: 0, bottom: 0 });
  let hole = $state({ top: 0, left: 0, right: 0, bottom: 0 });
  $effect(() => {
    const box = overlay, head = clear, part = area;
    if (!box) return;
    const measure = () => {
      const o = box.getBoundingClientRect(), a = part?.getBoundingClientRect() ?? o, r = head?.getBoundingClientRect();
      span = { top: a.top - o.top, left: a.left - o.left, right: o.right - a.right, bottom: o.bottom - a.bottom };
      hole = r
        ? { top: r.top - o.top, left: r.left - o.left, right: o.right - r.right, bottom: r.bottom - o.top }
        : { top: span.top, left: span.left, right: span.right, bottom: span.top };
    };
    measure();
    const seen = new ResizeObserver(measure);
    if (head) seen.observe(head);
    if (part) seen.observe(part);
    seen.observe(box);
    return () => seen.disconnect();
  });

  // The system «Назад» closes the window, not the chat under it.
  $effect(() => {
    if (!m) return;
    return untrack(() => holdBack(() => onclose(), 'selectText'));
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && m) {
      e.stopPropagation();
      onclose();
    }
  }

  // A tap beside the bubble first takes the selection away: it closes nothing then.
  let selected = false;
  const onPointerDown = () => (selected = !!window.getSelection()?.toString());
  const dismiss = () => { if (!selected) onclose(); };

  function copyAll() {
    if (!m?.text) return;
    navigator.clipboard.writeText(m.text).then(() => toast.success($t('msg_text_copied'))).catch(() => {});
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if m}
  <div class="overlay" bind:this={overlay} style:--clear="{hole.bottom}px" style:--side-l="{span.left}px" style:--side-r="{span.right}px" style:--foot="{span.bottom}px" use:portal use:layer onpointerdowncapture={onPointerDown} use:backdropDismiss={dismiss} role="presentation">
    <div class="veil" in:backdropIn out:backdropOut style:top="{span.top}px" style:left="{span.left}px" style:right="{span.right}px" style:height="{Math.max(0, hole.top - span.top)}px"></div>
    <div class="veil" in:backdropIn out:backdropOut style:top="{hole.top}px" style:left="{span.left}px" style:width="{Math.max(0, hole.left - span.left)}px" style:height="{hole.bottom - hole.top}px"></div>
    <div class="veil" in:backdropIn out:backdropOut style:top="{hole.top}px" style:right="{span.right}px" style:width="{Math.max(0, hole.right - span.right)}px" style:height="{hole.bottom - hole.top}px"></div>
    <div class="veil" in:backdropIn out:backdropOut style:top="{hole.bottom}px" style:left="{span.left}px" style:right="{span.right}px" style:bottom="{span.bottom}px"></div>
    <div class="box" role="dialog" aria-modal="true" aria-label={$t('msg_select_text')} tabindex="-1" in:dialogIn out:dialogOut>
      <div class="bubble" class:out>
        {#if author && !out}<span class="author" style="color: {tint(m.sender_pubkey)}">{author(m.sender_pubkey)}</span>{/if}
        <div class="body">
          <MessageContent text={m.text ?? ''} rich={m.rich} cards={false} still />
          <span class="meta">
            {#if m.edited_at}<span>{$t('msg_message_edited')}</span>{/if}
            <span>{clock(m.created_at)}</span>
          </span>
        </div>
      </div>
      <div class="actions" class:out>
        <button class="btn btn-primary copy" onclick={copyAll}><Icon name="copy" size={16} />{$t('msg_select_copy_all')}</button>
        <button class="btn close" onclick={onclose} aria-label={$t('common_close')} title={$t('common_close')}><Icon name="x" size={18} /></button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* The window catches every tap, the head's and the list's too; the bubble stands
     under the head, over the chat. */
  .overlay {
    position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0); overflow: hidden;
    z-index: var(--z-modal);
    display: flex; align-items: center; justify-content: center; padding: calc(var(--clear) + var(--sp-4)) calc(var(--side-r) + var(--sp-4)) calc(var(--foot) + var(--sp-4)) calc(var(--side-l) + var(--sp-4));
  }
  /* The chat round the head is not to be read: a strong blur and a shade. Each strip
     fades in by itself: an opacity over them would leave them nothing to blur. */
  .veil {
    position: absolute; pointer-events: none;
    background: color-mix(in srgb, var(--bg) 35%, transparent);
    -webkit-backdrop-filter: blur(24px) saturate(0.8); backdrop-filter: blur(24px) saturate(0.8);
  }
  /* Where nothing blurs, a fill through which nothing is read. */
  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    .veil { background: color-mix(in srgb, var(--bg) 94%, transparent); }
  }
  .box { position: relative; display: flex; flex-direction: column; align-items: stretch; gap: var(--sp-3); width: min(620px, 100%); max-height: 100%; outline: none; }
  .bubble {
    align-self: flex-start; box-sizing: border-box; max-width: 100%; max-height: 70vh; overflow-y: auto; overscroll-behavior: contain;
    padding: var(--sp-1-5) var(--sp-3); display: flex; flex-direction: column; gap: 2px; min-width: 64px;
    background: var(--bubble-in-bg); color: var(--text); border: 1px solid var(--bubble-in-border);
    border-radius: var(--radius-bubble); border-bottom-left-radius: var(--radius-badge);
    box-shadow: var(--shadow-lg);
  }
  .bubble.out {
    align-self: flex-end; background: var(--bubble-out-bg); border-color: var(--bubble-out-border); color: var(--bubble-out-text);
    border-bottom-left-radius: var(--radius-bubble); border-bottom-right-radius: var(--radius-badge);
  }
  /* Here the words are to be selected, a phone's too (the app selects nothing else there). */
  .overlay .bubble, .overlay .bubble :global(*) { user-select: text; -webkit-user-select: text; -webkit-touch-callout: default; }
  .author { font-size: var(--fs-sm); font-weight: var(--fw-bold); line-height: var(--lh-tight); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .body { display: block; }
  .body::after { content: ''; display: block; clear: both; }
  .meta {
    float: right; margin-left: var(--sp-3); box-sizing: border-box; height: calc(var(--fs-body) * var(--lh-body));
    display: inline-flex; align-items: flex-end; gap: var(--sp-1); padding-bottom: var(--sp-1);
    font-size: var(--fs-xs); font-variant-numeric: tabular-nums; color: var(--text-2); line-height: var(--lh-icon); white-space: nowrap;
  }
  .bubble.out .meta { color: var(--bubble-out-meta); }
  .overlay .meta, .overlay .meta :global(*) { user-select: none; -webkit-user-select: none; }
  /* The buttons stand at the bubble's edge: under theirs at the left, under mine at the right. */
  .actions { display: flex; justify-content: flex-start; gap: var(--sp-2); flex: none; }
  .actions.out { justify-content: flex-end; }
  .copy { gap: var(--sp-2); }
  .close { padding-inline: var(--sp-2); }
</style>

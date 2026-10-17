<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { drafts, markdownDrafts } from '../shared/drafts.svelte';
  import { applyMark, markOfKey, type Mark } from './format';
  import { readingOf } from '../content/markdown';
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import EmojiPicker from '../shared/emoji/EmojiPicker.svelte';
  import { usageStore } from '../shared/emoji/usageStore.svelte';
  import { onKeyboard } from '../shared/keyboard';
  import RecorderBar from '../media/RecorderBar.svelte';
  import CircleRecorder from '../media/CircleRecorder.svelte';
  import TouchRecorder from '../media/TouchRecorder.svelte';
  import { edgeHold } from '../shared/edge-hold';
  import { HOLD_MS, IDLE, step, type Gesture, type GestureEvent } from './record-gesture';
  import { captureSupported, type Captured } from '../media/capture';
  import { mediaOf, type MessengerMessage, type MessengerRecording, type TextFormat } from '../api';
  import { replyLine, replyThumb } from '../push/wording';

  interface Props {
    disabled?: boolean;
    placeholder?: string;
    replyTo: MessengerMessage | null;
    editing: MessengerMessage | null;
    peerTitle: string;
    oncancel: () => void;
    /** `fmt`: `md` when the words go as Markdown (the format menu was used). */
    onsend: (text: string, fmt: TextFormat | null) => Promise<void>;
    /** Buttons next to the input (attach); on a phone they stand inside the field. */
    tools?: Snippet;
    /** Keeps an unsent text per conversation. */
    draftKey?: string;
    /** Arrow up in an empty field: edit my last message. */
    oneditlast?: () => void;
    /** Voice messages and circles; absent or `false` hides the buttons. */
    onrecording?: (rec: MessengerRecording) => Promise<void>;
    canRecord?: boolean;
    /** Take the focus when a conversation opens (not on a phone: it would raise the keyboard). */
    autofocus?: boolean;
    /** Picked files that wait above the field: being read in, or ready to go with the text as their caption. */
    attached?: "loading" | "ready" | null;
    attachments?: Snippet;
  }
  let {
    disabled = false, placeholder, replyTo, editing, peerTitle, oncancel, onsend, tools, draftKey, oneditlast,
    onrecording, canRecord = true, autofocus = true, attached = null, attachments,
  }: Props = $props();

  let emojiOpen = $state(false);

  // The phone (design kit, M23): the emoji panel takes the place of the
  // on-screen keyboard under the composer, at its height, so the composer does
  // not jump. The field keeps the caret with inputmode="none" (the keyboard
  // stays down); the smile button turns into a keyboard and back. The panel's
  // height is what the keyboard leaves free of the keyboard's own height: it
  // grows as the keyboard goes down and shrinks as it comes up.
  // 'open' — the panel; 'tokb' — giving way to the keyboard; 'down' — both going.
  // Like a sheet, the panel can be pulled up by its grip (to look for an
  // emoji), and folded by its button.
  let dock = $state<'' | 'open' | 'tokb' | 'down'>('');
  let expanded = $state(false);
  let dragH = $state<number | null>(null);
  /** The height animates — not while it follows the keyboard or a finger (the composer would jump). */
  let glide = $state(true);
  let kbNow = $state(0);
  let kbH = $state(280);
  let root = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (!touch) return;
    return onKeyboard((inset) => {
      kbNow = inset;
      if (inset > 120) kbH = inset;
    });
  });
  const maxH = () => Math.round(window.innerHeight * 0.72);
  const baseH = $derived(Math.max(kbH - kbNow, 0));
  const dockH = $derived(
    dragH ?? (dock === 'down' || dock === '' ? 0 : dock === 'open' && expanded ? Math.max(maxH() - kbNow, 0) : baseH),
  );
  // With the panel the composer gives up its bottom padding (10 + the home
  // indicator) and the panel takes it in: the row above stays exactly where
  // it was in every step. The keyboard's inset already holds the home
  // indicator, so a panel in its place adds the 10 alone — the same gap above
  // it as above the keyboard, which then seems to slide over the panel; an
  // empty one is the padding itself.
  const dockCss = $derived(dockH > 0 ? `${dockH + 10}px` : 'calc(10px + var(--sab, 0px))');
  // Giving way ends when the keyboard is up (or, failing that, after a while); going down after the shrink.
  $effect(() => {
    if (dock === 'tokb' && kbNow >= kbH - 8) dock = '';
    if (dock !== 'tokb' && dock !== 'down') return;
    const timer = setTimeout(() => (dock = ''), dock === 'down' ? 220 : 800);
    return () => clearTimeout(timer);
  });
  // The conversation keeps its bottom while the panel changes height (ChatWindow follows the keyboard's event).
  $effect(() => {
    void dockH;
    const timer = setTimeout(() => window.dispatchEvent(new Event('veydan-keyboard')), 220);
    return () => clearTimeout(timer);
  });
  // A tap outside the composer and the panel takes both down.
  $effect(() => {
    if (dock !== 'open') return;
    const outside = (e: PointerEvent) => {
      if (root?.contains(e.target as Node)) return;
      if ((e.target as HTMLElement).closest?.('.m-sheet, .context-menu')) return;
      closeDock();
      el?.blur();
    };
    // The attach button opens its own sheet in the panel's place. Not on the finger going down: the composer
    // would slide down under the finger, and the tap would land on whatever came under it (a picture).
    const attach = (e: MouseEvent) => {
      if (root?.contains(e.target as Node) && (e.target as HTMLElement).closest?.('.attach')) closeDock();
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('click', attach, true);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('click', attach, true);
    };
  });

  /** Refocus the field so that the browser applies its inputmode (shows or hides the keyboard). */
  function refocus() {
    if (!el) return;
    if (document.activeElement === el) el.blur();
    el.focus({ preventScroll: true });
  }
  function openDock() {
    el?.setAttribute('inputmode', 'none');
    // Over an open keyboard the panel takes its place as the keyboard goes: no animation of its own.
    glide = kbNow < 40;
    expanded = false;
    dock = 'open';
    refocus();
  }
  function toKeyboard() {
    el?.removeAttribute('inputmode');
    glide = true;
    expanded = false;
    dock = 'tokb';
    refocus();
  }
  function closeDock() {
    el?.removeAttribute('inputmode');
    glide = true;
    expanded = false;
    if (dock === 'open') dock = 'down';
  }
  /** The fold button: a pulled-up panel back to its place, otherwise both down. */
  function fold() {
    if (expanded) { glide = true; expanded = false; return; }
    closeDock();
    el?.blur();
  }

  // The grip: a tap pulls the panel up or back; a drag follows the finger and lets go to the nearest place
  // (up, its own place, or down and away).
  let grip: { y: number; h: number; moved: boolean } | null = null;
  function gripDown(e: PointerEvent) {
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    grip = { y: e.clientY, h: dockH, moved: false };
  }
  function gripMove(e: PointerEvent) {
    if (!grip) return;
    const dy = e.clientY - grip.y;
    if (!grip.moved && Math.abs(dy) < 6) return;
    grip.moved = true;
    glide = false;
    dragH = Math.min(Math.max(grip.h - dy, 0), maxH());
  }
  function gripUp() {
    if (!grip) return;
    const { moved } = grip;
    grip = null;
    const h = dragH;
    dragH = null;
    glide = true;
    if (!moved) { expanded = !expanded; return; }
    if (h === null) return;
    if (h > baseH + 60) expanded = true;
    else if (h < baseH - 80) { expanded = false; closeDock(); el?.blur(); }
    else expanded = false;
  }
  function toggleEmoji() {
    if (!touch) { emojiOpen = !emojiOpen; return; }
    if (dock === 'open') toKeyboard(); else openDock();
  }
  /** «Стереть» of the panel: the symbol before the caret (a whole emoji, flags and families too). */
  function erase() {
    if (!el) return;
    const a = el.selectionStart ?? text.length;
    const b = el.selectionEnd ?? text.length;
    let from = a;
    if (a === b) {
      if (a === 0) return;
      const before = text.slice(0, a);
      const seg = typeof Intl !== 'undefined' && 'Segmenter' in Intl ? [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(before)] : null;
      from = seg && seg.length ? seg[seg.length - 1].index : a - 1;
    }
    text = text.slice(0, from) + text.slice(b);
    queueMicrotask(() => { el!.selectionStart = el!.selectionEnd = from; resize(); });
  }
  let recording = $state<"voice" | "circle" | null>(null);
  let recError = $state("");
  const voiceOk = captureSupported("voice");
  const circleOk = captureSupported("circle");

  function startRecording(kind: "voice" | "circle") {
    recError = "";
    emojiOpen = false;
    closeDock();
    recording = kind;
  }

  async function recorded(kind: "voice" | "circle", c: Captured) {
    recording = null;
    endHeld();
    try {
      await onrecording?.({ kind, mime: c.mime, duration_ms: c.durationMs, waveform: kind === "voice" ? c.waveform : undefined, blob: c.blob });
    } catch {
      // The chat shows why; nothing to keep here.
    }
  }

  function recordingFailed(code: string) {
    recording = null;
    endHeld();
    recError = code;
  }

  // On a phone Enter is a new line; sending is the button.
  const touch = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;

  // The phone has one record button: a tap switches between voice and circle,
  // a hold records (record-gesture.ts).
  const MODE_KEY = 'veydan.msg.rec.mode';
  function savedMode(): "voice" | "circle" {
    let saved: string | null = null;
    try { saved = localStorage.getItem(MODE_KEY); } catch { /* no storage: the default */ }
    if (!voiceOk) return "circle";
    return saved === "circle" && circleOk ? "circle" : "voice";
  }
  let mode = $state<"voice" | "circle">(savedMode());
  let gesture = $state<Gesture>(IDLE);
  /** What the held button records; stays until it is sent or discarded. */
  let held = $state<"voice" | "circle" | null>(null);
  let heldLocked = $state(false);
  let heldPhase = $state<"recording" | "preview">("recording");
  let heldEl = $state<ReturnType<typeof TouchRecorder> | null>(null);
  let hint = $state("");
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  let hintTimer: ReturnType<typeof setTimeout> | null = null;

  function endHeld() {
    held = null;
    heldLocked = false;
    heldPhase = "recording";
  }

  function showHint() {
    hint = mode;
    if (hintTimer) clearTimeout(hintTimer);
    hintTimer = setTimeout(() => { hint = ""; }, 1800);
  }

  function act(e: GestureEvent) {
    const r = step(gesture, e);
    gesture = r.g;
    if (r.action === "toggle") {
      if (voiceOk && circleOk) {
        mode = mode === "voice" ? "circle" : "voice";
        try { localStorage.setItem(MODE_KEY, mode); } catch { /* the choice is not kept */ }
      }
      showHint();
    } else if (r.action === "start") {
      recError = "";
      hint = "";
      emojiOpen = false;
      closeDock();
      heldLocked = false;
      heldPhase = "recording";
      held = mode;
      navigator.vibrate?.(12);
    } else if (r.action === "discard") {
      navigator.vibrate?.(20);
      if (heldEl) heldEl.cancel(); else endHeld();
    } else if (r.action === "lock") {
      heldLocked = true;
      navigator.vibrate?.(12);
    } else if (r.action === "release") {
      if (heldEl) heldEl.stop(); else endHeld();
    }
  }

  function recDown(e: PointerEvent) {
    e.preventDefault();
    // Locked or under review: the tap sends, on the lift.
    if (held) return;
    try { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); } catch { /* the pointer is already gone */ }
    act({ type: "down", x: e.clientX, y: e.clientY });
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = setTimeout(() => { holdTimer = null; act({ type: "hold" }); }, HOLD_MS);
  }
  function recMove(e: PointerEvent) {
    if (gesture.phase === "holding") act({ type: "move", x: e.clientX, y: e.clientY });
  }
  function recUp() {
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
    if (gesture.phase === "idle") { if (held) heldEl?.send(); return; }
    act({ type: "up" });
  }
  function recLost() {
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
    act({ type: "cancel" });
  }

  const MAX_BYTES = 32 * 1024;
  let text = $state('');
  let el = $state<HTMLTextAreaElement | null>(null);

  const bytes = $derived(new TextEncoder().encode(text).length);
  const tooLong = $derived(bytes > MAX_BYTES);
  // Not gated on a send in flight: the next message can be typed and sent
  // while the previous one is still on its way (order is kept by the chat).
  // Picked files go with or without words, once they are read in.
  const canSend = $derived(!disabled && !tooLong && attached !== "loading" && (text.trim().length > 0 || attached === "ready"));
  const canRecordNow = $derived((!!held || (!text.trim() && !attached)) && !editing && !!onrecording && canRecord && !disabled && (voiceOk || circleOk));

  // The words go as Markdown: set by the format menu (or by editing a
  // Markdown message), shown as a chip over the field, taken off by its ×.
  // The field holds the marks as they are typed; an empty field is plain.
  let md = $state(false);
  $effect(() => { if (!text) md = false; });

  // Entering edit mode loads the message text; leaving it clears the field.
  let lastEditing: string | null = null;
  $effect(() => {
    const id = editing?.id ?? null;
    if (id === lastEditing) return;
    lastEditing = id;
    text = editing?.text ?? '';
    md = editing?.fmt === 'md';
    queueMicrotask(() => { resize(); el?.focus(); });
  });
  $effect(() => { if (replyTo) el?.focus(); });

  // Switching conversations: park the text of the old one, restore the new one.
  let lastKey: string | undefined;
  $effect(() => {
    const key = draftKey;
    if (key === lastKey) return;
    if (lastKey !== undefined && !editing) {
      drafts.set(lastKey, text);
      if (md) markdownDrafts.add(lastKey); else markdownDrafts.delete(lastKey);
    }
    lastKey = key;
    text = key ? (drafts.get(key) ?? "") : "";
    md = !!key && !!text && markdownDrafts.has(key);
    sel = null;
    queueMicrotask(() => { resize(); if (autofocus) el?.focus(); });
  });
  $effect(() => {
    if (!draftKey || editing) return;
    if (text.trim()) drafts.set(draftKey, text);
    else drafts.delete(draftKey);
    if (text.trim() && md) markdownDrafts.add(draftKey); else markdownDrafts.delete(draftKey);
  });

  // The format menu stands while a part of the field is selected. The
  // selection is kept here as well: a tap on the menu must not lose it.
  let sel = $state<{ a: number; b: number } | null>(null);
  function readSel() {
    if (!el || document.activeElement !== el) return;
    const a = el.selectionStart ?? 0;
    const b = el.selectionEnd ?? 0;
    sel = a === b ? null : { a: Math.min(a, b), b: Math.max(a, b) };
  }
  $effect(() => {
    document.addEventListener('selectionchange', readSel);
    return () => document.removeEventListener('selectionchange', readSel);
  });

  /** The buttons of the format menu, with their shortcuts on a computer (`markOfKey`). */
  const MARKS = [
    { mark: 'bold', icon: 'bold', label: 'msg_fmt_bold', key: 'B' },
    { mark: 'italic', icon: 'italic', label: 'msg_fmt_italic', key: 'I' },
    { mark: 'strike', icon: 'strikethrough', label: 'msg_fmt_strike', key: 'Shift+X' },
    { mark: 'code', icon: 'code', label: 'msg_fmt_code', key: 'Shift+M' },
    { mark: 'quote', icon: 'quote', label: 'msg_fmt_quote', key: '' },
  ] as const satisfies readonly { mark: Mark; icon: string; label: string; key: string }[];
  const MOD = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';

  /** A button of the menu, or its shortcut: the mark on the selected part; the message goes as Markdown. */
  function format(mark: Mark) {
    if (!el) return;
    const at = sel ?? { a: el.selectionStart ?? 0, b: el.selectionEnd ?? 0 };
    if (at.a === at.b) return;
    const r = applyMark(text, at.a, at.b, mark);
    text = r.text;
    md = true;
    sel = r.start === r.end ? null : { a: r.start, b: r.end };
    queueMicrotask(() => {
      if (!el) return;
      el.focus({ preventScroll: true });
      el.setSelectionRange(r.start, r.end);
      resize();
    });
  }

  function resize() {
    if (!el) return;
    el.style.height = 'auto';
    // The phone field grows to three lines, then scrolls.
    el.style.height = `${Math.min(el.scrollHeight, touch ? 82 : 180)}px`;
  }

  export function focus() { el?.focus(); }

  export function insert(s: string) {
    if (!el) { text += s; return; }
    const a = el.selectionStart ?? text.length;
    const b = el.selectionEnd ?? text.length;
    text = text.slice(0, a) + s + text.slice(b);
    queueMicrotask(() => { el!.selectionStart = el!.selectionEnd = a + s.length; resize(); el!.focus(); });
  }

  // The field is cleared at once and never loses focus, so the keyboard
  // stays open between messages. A failed send puts the text back.
  async function submit() {
    if (!canSend) return;
    const value = text.trim();
    const wasEditing = !!editing;
    const fmt: TextFormat | null = md && value ? 'md' : null;
    text = '';
    sel = null;
    queueMicrotask(resize);
    try {
      await onsend(value, fmt);
    } catch {
      if (!text && !wasEditing) {
        text = value;
        md = fmt === 'md';
      }
      queueMicrotask(resize);
    }
  }

  /** Buttons next to the field act without taking the focus from it. */
  const keepFocus = (e: Event) => e.preventDefault();

  function onkeydown(e: KeyboardEvent) {
    const mark = touch ? null : markOfKey(e);
    if (mark && el && el.selectionStart !== el.selectionEnd) { e.preventDefault(); format(mark); }
    else if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && !touch) { e.preventDefault(); submit(); }
    else if (e.key === 'Escape' && (replyTo || editing)) { e.preventDefault(); oncancel(); }
    else if (e.key === "ArrowUp" && !text && !editing && oneditlast) { e.preventDefault(); oneditlast(); }
  }
</script>

{#snippet formatBar()}
  <div class="fmt" role="toolbar" aria-label={$t('msg_fmt_menu')}>
    {#each MARKS as m (m.mark)}
      {@const label = touch || !m.key ? $t(m.label) : `${$t(m.label)} (${MOD}+${m.key})`}
      <button class="fb" tabindex="-1" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => format(m.mark)} title={label} aria-label={label}>
        <Icon name={m.icon} size={16} />
      </button>
    {/each}
    <span class="sep" aria-hidden="true"></span>
    <button class="fb word" class:on={md} tabindex="-1" aria-pressed={md} onpointerdown={keepFocus} onmousedown={keepFocus}
      onclick={() => (md = !md)} title={$t(md ? 'msg_fmt_md_off' : 'msg_fmt_md_on')}>Markdown</button>
  </div>
{/snippet}

<div class="composer" class:docked={touch && !!dock} bind:this={root}>
  {#if editing || replyTo}
    <div class="context">
      <Icon name={editing ? 'pencil' : 'reply'} size={14} />
      {#if !editing && replyTo}
        {@const thumb = replyThumb({ text: readingOf(replyTo), deleted: replyTo.deleted, media: { ...mediaOf(replyTo) } })}
        {#if thumb}<img class="ctx-thumb" src={thumb} alt="" aria-hidden="true" />{/if}
      {/if}
      <span class="ctx-body">
        <span class="ctx-title">{editing ? $t('msg_composer_editing') : $t('msg_composer_reply_to', { name: replyTo!.direction === 'out' ? $t('msg_you') : peerTitle })}</span>
        <span class="ctx-text">{editing ? (editing.text ?? '') : replyLine({ text: readingOf(replyTo!), deleted: replyTo!.deleted, media: { ...mediaOf(replyTo!) } }, replyTo!.card?.label ?? null, $t)}</span>
      </span>
      <button class="icon" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={oncancel} title={$t('msg_back')}><Icon name="x" size={14} /></button>
    </div>
  {/if}
  {#if attachments}{@render attachments()}{/if}
  {#if md || (sel && !touch && !recording)}
    <div class="above">
      {#if md}
        <span class="mdchip" title={$t('msg_fmt_md_on')}>
          <Icon name="file-code" size={13} /><span>Markdown</span>
          <button class="icon" tabindex="-1" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => (md = false)} title={$t('msg_fmt_md_off')} aria-label={$t('msg_fmt_md_off')}>
            <Icon name="x" size={12} />
          </button>
        </span>
      {/if}
      {#if sel && !touch && !recording}{@render formatBar()}{/if}
    </div>
  {/if}
  <div class="row">
    {#if touch}
      {#if held}
        <TouchRecorder bind:this={heldEl} bind:phase={heldPhase} kind={held} locked={heldLocked} dx={gesture.dx} dy={gesture.dy}
          oncancel={endHeld} ondone={(c) => recorded(held!, c)} onerror={recordingFailed} />
      {/if}
        <!-- Stays in the page while a recording goes: taking the field away would close the keyboard. -->
        <div class="field" class:away={!!held}>
          <button class="tool" data-emoji-toggle tabindex="-1" {disabled} aria-expanded={dock === 'open'}
            onpointerdown={keepFocus} onmousedown={keepFocus} onclick={toggleEmoji} aria-label={dock === 'open' ? $t('msg_emoji_keyboard') : $t('msg_emoji_title')}>
            <Icon name={dock === 'open' ? 'keyboard' : 'smile'} size={21} />
          </button>
          <!-- A tap on the field while the panel is open brings the keyboard back. -->
          <textarea
            bind:this={el} bind:value={text} rows="1" {disabled}
            placeholder={attached ? $t('msg_attach_caption') : (placeholder ?? $t('msg_composer_placeholder'))}
            oninput={() => { resize(); readSel(); }} {onkeydown} onselect={readSel} onkeyup={readSel} onpointerup={readSel} onblur={() => (sel = null)}
            onpointerdown={() => { if (dock === 'open') { el?.removeAttribute('inputmode'); dock = 'tokb'; } }}
          ></textarea>
          {#if tools}{@render tools()}{/if}
        </div>
      {#if canRecordNow}
        {@const sends = !!held && (heldLocked || heldPhase === "preview")}
        {#if hint && !held}<div class="hint" role="status">{$t(`msg_rec_hint_${hint}` as "msg_rec_hint_voice")}</div>{/if}
        <button class="send record" use:edgeHold class:holding={gesture.phase === "holding"} tabindex="-1"
          style:transform={gesture.phase === "holding" ? `translate(${gesture.dx}px, ${gesture.dy}px) scale(1.45)` : undefined}
          onpointerdown={recDown} onpointermove={recMove} onpointerup={recUp} onpointercancel={recLost}
          onmousedown={keepFocus} oncontextmenu={keepFocus}
          aria-label={sends ? $t('msg_composer_send') : $t(mode === "circle" ? "msg_rec_circle" : "msg_rec_voice")}>
          {#key sends ? "send" : mode}
            <span class="glyph"><Icon name={sends ? "send" : mode === "circle" ? "circle-video" : "mic"} size={sends ? 17 : 19} /></span>
          {/key}
        </button>
      {:else}
        <button class="send" class:off={!canSend} aria-disabled={!canSend} tabindex="-1"
          onpointerdown={keepFocus} onmousedown={keepFocus} onclick={submit} aria-label={$t('msg_composer_send')}>
          <Icon name={editing ? 'check' : 'send'} size={17} />
        </button>
      {/if}
    {:else if recording === "voice"}
      <RecorderBar oncancel={() => (recording = null)} ondone={(c) => recorded("voice", c)} onerror={recordingFailed} />
    {:else}
    <!-- The order of the design kit: attach · the field with the smile at its end · circle · voice · send (with nothing to send it waits, dimmed, beside the voice; typing hides the recording buttons). -->
    {#if tools}{@render tools()}{/if}
    <div class="dfield">
      {#if emojiOpen}
        <EmojiPicker onpick={(e) => { insert(e); usageStore.used(e); }} onclose={() => (emojiOpen = false)} />
      {/if}
      <textarea
        bind:this={el} bind:value={text} rows="1" {disabled}
        placeholder={attached ? $t('msg_attach_caption') : (placeholder ?? $t('msg_composer_placeholder'))}
        oninput={() => { resize(); readSel(); }} {onkeydown} onselect={readSel} onkeyup={readSel} onmouseup={readSel} onblur={() => (sel = null)}
      ></textarea>
      <button class="tool emoji" class:on={emojiOpen} data-emoji-toggle tabindex="-1" {disabled}
        onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => (emojiOpen = !emojiOpen)} title={$t("msg_emoji_title")}>
        <Icon name="smile" size={18} />
      </button>
    </div>
    {#if canRecordNow}
      {#if circleOk}
        <button class="tool" tabindex="-1" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => startRecording("circle")} title={$t("msg_rec_circle")}>
          <Icon name="video" size={18} />
        </button>
      {/if}
      {#if voiceOk}
        <button class="tool voice" tabindex="-1" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => startRecording("voice")} title={$t("msg_rec_voice")}>
          <Icon name="mic" size={18} />
        </button>
      {/if}
    {/if}
    <button class="send" class:off={!canSend} aria-disabled={!canSend} tabindex="-1"
      onpointerdown={keepFocus} onmousedown={keepFocus} onclick={submit} title={editing ? $t('msg_composer_save') : $t('msg_composer_send')}>
      <Icon name={editing ? 'check' : 'send'} size={18} />
    </button>
    {/if}
  </div>
  <!-- The phone: under the field, above the keyboard, clear of the system's own menu over the selection. -->
  {#if touch && sel && !held}{@render formatBar()}{/if}
  {#if tooLong}<div class="warn">{$t('msg_composer_too_long')}</div>{/if}
  {#if recError}<div class="warn">{$t(`msg_rec_err_${recError}` as "msg_rec_err_denied")}</div>{/if}
  {#if touch && dock}
    <div class="dock" class:glide style:height={dockCss}>
      <div class="dock-panel">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="grip" onpointerdown={gripDown} onpointermove={gripMove} onpointerup={gripUp} onpointercancel={gripUp}>
        <button class="grab" tabindex="-1" onclick={(e) => e.preventDefault()} aria-label={expanded ? $t('msg_emoji_collapse') : $t('msg_emoji_expand')}></button>
        <button class="fold" tabindex="-1" onpointerdown={(e) => { e.preventDefault(); e.stopPropagation(); }} onclick={fold} aria-label={$t('msg_emoji_collapse')} title={$t('msg_emoji_collapse')}>
          <Icon name="chevron-down" size={20} />
        </button>
      </div>
      <EmojiPicker variant="dock" onpick={(e) => { insert(e); usageStore.used(e); }} onerase={erase} />
      </div>
    </div>
  {/if}
</div>

{#if recording === "circle"}
  <CircleRecorder oncancel={() => (recording = null)} ondone={(c) => recorded("circle", c)} onerror={recordingFailed} />
{/if}

<style>
  .composer { position: relative; background: transparent; padding: 0 var(--sp-3) var(--sp-3); display: flex; flex-direction: column; gap: var(--sp-2); }
  .row {
    padding: 6px; border-radius: var(--radius-card);
    background: var(--pane-strong); border: 1px solid var(--pane-strong-border); box-shadow: var(--pane-highlight), var(--pane-shadow);
  }
  .row:focus-within { border-color: var(--accent-border); }
  /* A computer: the field holds the smile at its end, and the emoji panel opens over it, its right edge on the smile. */
  .dfield { position: relative; flex: 1; min-width: 0; display: flex; align-items: flex-end; }
  .dfield textarea { flex: 1; min-width: 0; }
  .dfield :global(.picker) { left: auto; right: 0; bottom: calc(100% + 14px); }
  .tool.voice { border: 1px solid var(--border-2); }
  .context { display: flex; align-items: center; gap: var(--sp-2); padding: 4px 8px; border-left: 2px solid var(--accent); background: var(--surface-2); border-radius: var(--radius-xs); color: var(--accent-text-2); }
  .ctx-thumb { flex: 0 0 auto; width: 32px; height: 32px; object-fit: cover; border-radius: var(--radius-xs); background: var(--surface-3); }
  .ctx-body { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .ctx-title { font-size: var(--fs-2xs); font-weight: var(--fw-bold); }
  .ctx-text { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row { display: flex; align-items: flex-end; gap: var(--sp-2); }
  textarea {
    flex: 1; resize: none; font: inherit; font-size: var(--fs-body); line-height: var(--lh-body); color: var(--text);
    background: none; border: none; border-radius: 0; padding: 9px 8px;
    max-height: 180px; min-height: 40px;
  }
  textarea:focus { outline: none; box-shadow: none; }
  textarea:disabled { opacity: 0.6; }
  .send {
    width: 40px; height: 40px; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer;
    background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center;
    transition: filter var(--dur-fast) var(--ease), opacity var(--dur-fast) var(--ease);
  }
  .send:hover:not(.off) { filter: brightness(1.1); }
  .send.off { opacity: 0.35; cursor: default; }
  .tool { width: 40px; height: 40px; flex-shrink: 0; border: none; border-radius: 50%; background: none; color: var(--text-soft); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .tool:hover:not(:disabled) { color: var(--text); background: var(--surface-hover); }
  .tool.on { color: var(--accent-text-2); background: var(--accent-tint); }
  .tool:disabled { opacity: 0.4; cursor: default; }
  .icon { border: none; background: none; color: var(--text-3); cursor: pointer; display: inline-flex; padding: 4px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .warn { font-size: var(--fs-2xs); color: var(--danger-text); }
  /* Over the field: the chip of Markdown, and on a computer the format menu of a selection. */
  .above { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; min-height: 28px; }
  .mdchip {
    display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 2px 0 9px; border-radius: var(--radius-pill);
    background: var(--accent-tint); color: var(--accent-text-2); font-size: var(--fs-xs); font-weight: var(--fw-semibold);
  }
  .mdchip .icon { padding: 3px; border-radius: 50%; color: inherit; }
  .fmt {
    display: inline-flex; align-items: center; gap: 2px; padding: 3px; align-self: flex-start;
    border-radius: var(--radius); background: var(--pane-strong); border: 1px solid var(--pane-strong-border); box-shadow: var(--pane-highlight), var(--pane-shadow);
  }
  .fb {
    height: 30px; min-width: 32px; padding: 0 7px; border: none; border-radius: var(--radius-sm); background: none; cursor: pointer;
    color: var(--text-soft); display: inline-flex; align-items: center; justify-content: center; font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold);
  }
  .fb:hover { color: var(--text); background: var(--surface-hover); }
  .fb.on { color: var(--accent-text-2); background: var(--accent-tint); }
  .sep { width: 1px; height: 18px; margin: 0 3px; background: var(--border-2); }
  /* The emoji panel in the place of the keyboard: the composer's whole width, down to the screen's edge. */
  .composer.docked { padding-bottom: 0; }
  .dock {
    display: flex; flex-direction: column;
    /* Under the row without the column's gap, edge to edge. */
    margin: calc(-1 * var(--sp-2)) calc(-1 * var(--sp-2)) 0;
    padding-top: 10px;
    overflow: hidden;
  }
  /* The panel itself, 10 under the row like the keyboard. */
  .dock-panel {
    flex: 1; min-height: 0; display: flex; flex-direction: column;
    background: var(--surface);
    border-top: 1px solid var(--border);
    border-radius: var(--radius-card) var(--radius-card) 0 0;
    overflow: hidden;
  }
  .dock.glide { transition: height var(--dur-drawer) var(--ease-drawer); }
  /* The grip of a sheet (36×4), and the fold button at its end. */
  .grip { position: relative; display: flex; justify-content: center; align-items: center; flex-shrink: 0; height: 28px; touch-action: none; }
  .grab { width: 64px; height: 28px; padding: 0; border: 0; background: none; display: flex; align-items: center; justify-content: center; }
  .grab::before { content: ''; width: 36px; height: 4px; border-radius: var(--radius-pill); background: var(--border-2); }
  .fold { position: absolute; right: var(--sp-2); top: 50%; transform: translateY(-50%); width: 36px; height: 28px; padding: 0; border: 0; border-radius: var(--radius); background: none; color: var(--text-soft); display: inline-flex; align-items: center; justify-content: center; }
  .fold:active { background: var(--surface-hover); color: var(--text); }
  .dock :global(.picker.docked) { flex: 1; min-height: 0; height: auto; border-top: 0; }
  .dock :global(.picker.docked .body) { padding-bottom: calc(8px + var(--sab, 0px)); }
  @media (prefers-reduced-motion: reduce) { .dock.glide { transition: none; } }
  /* The phone: a glass bar over the conversation; the field holds its own buttons and stays one line high until the text needs more. */
  @media (pointer: coarse) {
    .composer { padding: var(--sp-1) var(--sp-2) calc(10px + var(--sab, 0px)); }
    .row {
      border-radius: var(--radius-bar);
      -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter);
    }
    .row:focus-within { border-color: var(--pane-strong-border); }
    .context { background: color-mix(in srgb, var(--m-field, var(--surface-2)) 65%, transparent); border-radius: var(--radius-sm); padding: 6px 8px; }
    .field {
      flex: 1; min-width: 0; display: flex; align-items: flex-end; min-height: 40px; border-radius: 20px;
      background: none;
      border: 1px solid transparent;
      --attach-w: 40px; --attach-h: 38px;
    }
    .field:focus-within { border-color: transparent; }
    .field.away { position: absolute; width: 1px; height: 1px; min-height: 0; overflow: hidden; opacity: 0; pointer-events: none; border: none; }
    .field textarea { min-width: 0; border: none; border-radius: 0; background: none; font-size: var(--fs-md); line-height: 22px; padding: 8px 2px; min-height: 38px; max-height: 82px; }
    /* The field shows the focus, not the text inside it. */
    .field textarea:focus { outline: none; box-shadow: none; }
    .field textarea::placeholder { white-space: nowrap; }
    .field .tool { width: 40px; height: 38px; margin-left: 2px; }
    .fmt { align-self: center; border-radius: var(--radius-bar); -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter); }
    .fb { height: 36px; min-width: 40px; }
    .fb:hover { color: var(--text-soft); background: none; }
    .fb:active { color: var(--text); background: var(--surface-hover); }
    .fb.on, .fb.on:hover { color: var(--accent-text-2); background: var(--accent-tint); }
    .send { width: 40px; height: 40px; }
    .record { position: relative; z-index: 3; touch-action: none; -webkit-touch-callout: none; user-select: none; -webkit-user-select: none; box-shadow: var(--shadow-accent); transition: transform var(--dur-fast) var(--ease); }
    .record.holding { transition: none; }
    .record.holding::after { content: ""; position: absolute; inset: -8px; z-index: -1; border-radius: 50%; background: var(--accent); opacity: 0.22; animation: halo 1.3s ease-in-out infinite; }
    .glyph { display: inline-flex; animation: swap var(--dur-base) var(--ease); }
    .hint {
      position: absolute; right: var(--sp-3); bottom: calc(100% + var(--sp-2)); z-index: 2; max-width: calc(100% - var(--sp-4));
      padding: 7px 12px; border-radius: var(--radius); background: var(--surface); border: 1px solid var(--border); box-shadow: var(--shadow);
      font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    }
  }
  @keyframes halo { 50% { transform: scale(1.18); opacity: 0.1; } }
  @keyframes swap { from { transform: rotate(-70deg) scale(0.4); opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .glyph, .record.holding::after { animation: none; } }
</style>

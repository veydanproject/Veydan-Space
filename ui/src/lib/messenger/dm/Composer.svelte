<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import EmojiPicker from '../shared/emoji/EmojiPicker.svelte';
  import { usageStore } from '../shared/emoji/usageStore.svelte';
  import RecorderBar from '../media/RecorderBar.svelte';
  import CircleRecorder from '../media/CircleRecorder.svelte';
  import TouchRecorder from '../media/TouchRecorder.svelte';
  import { edgeHold } from '../shared/edge-hold';
  import { HOLD_MS, IDLE, step, type Gesture, type GestureEvent } from './record-gesture';
  import { captureSupported, type Captured } from '../media/capture';
  import type { MessengerMessage, MessengerRecording } from '../api';

  interface Props {
    disabled?: boolean;
    placeholder?: string;
    replyTo: MessengerMessage | null;
    editing: MessengerMessage | null;
    peerTitle: string;
    oncancel: () => void;
    onsend: (text: string) => Promise<void>;
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
  let recording = $state<"voice" | "circle" | null>(null);
  let recError = $state("");
  const voiceOk = captureSupported("voice");
  const circleOk = captureSupported("circle");

  function startRecording(kind: "voice" | "circle") {
    recError = "";
    emojiOpen = false;
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

  const drafts: Map<string, string> = ((globalThis as Record<string, unknown>).__msgDrafts ??= new Map()) as Map<string, string>;
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

  // Entering edit mode loads the message text; leaving it clears the field.
  let lastEditing: string | null = null;
  $effect(() => {
    const id = editing?.id ?? null;
    if (id === lastEditing) return;
    lastEditing = id;
    text = editing?.text ?? '';
    queueMicrotask(() => { resize(); el?.focus(); });
  });
  $effect(() => { if (replyTo) el?.focus(); });

  // Switching conversations: park the text of the old one, restore the new one.
  let lastKey: string | undefined;
  $effect(() => {
    const key = draftKey;
    if (key === lastKey) return;
    if (lastKey !== undefined && !editing) drafts.set(lastKey, text);
    lastKey = key;
    text = key ? (drafts.get(key) ?? "") : "";
    queueMicrotask(() => { resize(); if (autofocus) el?.focus(); });
  });
  $effect(() => { if (draftKey && !editing) drafts.set(draftKey, text); });

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
    text = '';
    queueMicrotask(resize);
    try {
      await onsend(value);
    } catch {
      if (!text && !wasEditing) text = value;
      queueMicrotask(resize);
    }
  }

  /** Buttons next to the field act without taking the focus from it. */
  const keepFocus = (e: Event) => e.preventDefault();

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && !touch) { e.preventDefault(); submit(); }
    else if (e.key === 'Escape' && (replyTo || editing)) { e.preventDefault(); oncancel(); }
    else if (e.key === "ArrowUp" && !text && !editing && oneditlast) { e.preventDefault(); oneditlast(); }
  }
</script>

<div class="composer">
  {#if emojiOpen}
    <EmojiPicker onpick={(e) => { insert(e); usageStore.used(e); }} onclose={() => (emojiOpen = false)} />
  {/if}
  {#if editing || replyTo}
    <div class="context">
      <Icon name={editing ? 'pencil' : 'reply'} size={14} />
      <span class="ctx-body">
        <span class="ctx-title">{editing ? $t('msg_composer_editing') : $t('msg_composer_reply_to', { name: replyTo!.direction === 'out' ? $t('msg_you') : peerTitle })}</span>
        <span class="ctx-text">{(editing ?? replyTo)!.text ?? (replyTo?.card ? `👤 ${replyTo.card.label}` : '')}</span>
      </span>
      <button class="icon" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={oncancel} title={$t('msg_back')}><Icon name="x" size={14} /></button>
    </div>
  {/if}
  {#if attachments}{@render attachments()}{/if}
  <div class="row">
    {#if touch}
      {#if held}
        <TouchRecorder bind:this={heldEl} bind:phase={heldPhase} kind={held} locked={heldLocked} dx={gesture.dx} dy={gesture.dy}
          oncancel={endHeld} ondone={(c) => recorded(held!, c)} onerror={recordingFailed} />
      {/if}
        <!-- Stays in the page while a recording goes: taking the field away would close the keyboard. -->
        <div class="field" class:away={!!held}>
          <button class="tool" class:on={emojiOpen} data-emoji-toggle tabindex="-1" {disabled}
            onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => (emojiOpen = !emojiOpen)} aria-label={$t("msg_emoji_title")}>
            <Icon name="smile" size={21} />
          </button>
          <textarea
            bind:this={el} bind:value={text} rows="1" {disabled}
            placeholder={attached ? $t('msg_attach_caption') : (placeholder ?? $t('msg_composer_placeholder'))}
            oninput={resize} {onkeydown}
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
    {#if tools}{@render tools()}{/if}
    <button class="tool" class:on={emojiOpen} data-emoji-toggle tabindex="-1" {disabled}
      onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => (emojiOpen = !emojiOpen)} title={$t("msg_emoji_title")}>
      <Icon name="smile" size={18} />
    </button>
    <textarea
      bind:this={el} bind:value={text} rows="1" {disabled}
      placeholder={attached ? $t('msg_attach_caption') : (placeholder ?? $t('msg_composer_placeholder'))}
      oninput={resize} {onkeydown}
    ></textarea>
    {#if canRecordNow}
      {#if circleOk}
        <button class="tool" tabindex="-1" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => startRecording("circle")} title={$t("msg_rec_circle")}>
          <Icon name="video" size={18} />
        </button>
      {/if}
      {#if voiceOk}
        <button class="send" tabindex="-1" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => startRecording("voice")} title={$t("msg_rec_voice")}>
          <Icon name="mic" size={17} />
        </button>
      {/if}
    {:else}
    <button class="send" class:off={!canSend} aria-disabled={!canSend} tabindex="-1"
      onpointerdown={keepFocus} onmousedown={keepFocus} onclick={submit} title={$t('msg_composer_send')}>
      <Icon name={editing ? 'check' : 'send'} size={16} />
    </button>
    {/if}
    {/if}
  </div>
  {#if tooLong}<div class="warn">{$t('msg_composer_too_long')}</div>{/if}
  {#if recError}<div class="warn">{$t(`msg_rec_err_${recError}` as "msg_rec_err_denied")}</div>{/if}
</div>

{#if recording === "circle"}
  <CircleRecorder oncancel={() => (recording = null)} ondone={(c) => recorded("circle", c)} onerror={recordingFailed} />
{/if}

<style>
  .composer { position: relative; border-top: 1px solid var(--border); background: var(--surface); padding: var(--sp-2) var(--sp-3) var(--sp-3); display: flex; flex-direction: column; gap: var(--sp-2); }
  .context { display: flex; align-items: center; gap: var(--sp-2); padding: 4px 8px; border-left: 2px solid var(--accent); background: var(--surface-2); border-radius: 4px; color: var(--accent-text-2); }
  .ctx-body { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .ctx-title { font-size: var(--fs-2xs); font-weight: var(--fw-bold); }
  .ctx-text { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row { display: flex; align-items: flex-end; gap: var(--sp-2); }
  textarea {
    flex: 1; resize: none; font: inherit; font-size: var(--fs-sm); line-height: 1.45; color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 18px; padding: 8px 14px;
    max-height: 180px; min-height: 38px;
  }
  textarea:focus { outline: none; border-color: var(--accent-border); }
  textarea:disabled { opacity: 0.6; }
  .send {
    width: 38px; height: 38px; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer;
    background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center;
    transition: filter var(--dur-fast) var(--ease), opacity var(--dur-fast) var(--ease);
  }
  .send:hover:not(.off) { filter: brightness(1.1); }
  .send.off { opacity: 0.35; cursor: default; }
  .tool { width: 38px; height: 38px; flex-shrink: 0; border: none; border-radius: 50%; background: none; color: var(--text-2); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .tool:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .tool.on { color: var(--accent-text-2); background: var(--accent-tint); }
  .tool:disabled { opacity: 0.4; cursor: default; }
  .icon { border: none; background: none; color: var(--text-3); cursor: pointer; display: inline-flex; padding: 4px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .warn { font-size: var(--fs-2xs); color: var(--danger-text); }
  /* The phone: a glass bar over the conversation; the field holds its own buttons and stays one line high until the text needs more. */
  @media (pointer: coarse) {
    .composer {
      padding: 7px var(--sp-3) calc(var(--sp-2) + var(--sab, 0px));
      background: color-mix(in srgb, var(--surface) 62%, transparent);
      -webkit-backdrop-filter: blur(22px) saturate(1.7); backdrop-filter: blur(22px) saturate(1.7);
      border-top-color: color-mix(in srgb, var(--text) 10%, transparent);
    }
    .context { background: color-mix(in srgb, var(--m-field, var(--surface-2)) 65%, transparent); border-radius: var(--radius-sm); padding: 6px 8px; }
    .field {
      flex: 1; min-width: 0; display: flex; align-items: flex-end; min-height: 40px; border-radius: 20px;
      background: color-mix(in srgb, var(--m-field, var(--surface-2)) 65%, transparent);
      border: 1px solid color-mix(in srgb, var(--text) 10%, transparent);
      --attach-w: 40px; --attach-h: 38px;
    }
    .field:focus-within { border-color: var(--accent-border); }
    .field.away { position: absolute; width: 1px; height: 1px; min-height: 0; overflow: hidden; opacity: 0; pointer-events: none; border: none; }
    .field textarea { min-width: 0; border: none; border-radius: 0; background: none; font-size: 16px; line-height: 22px; padding: 8px 2px; min-height: 38px; max-height: 82px; }
    /* The field shows the focus, not the text inside it. */
    .field textarea:focus { outline: none; box-shadow: none; }
    .field textarea::placeholder { white-space: nowrap; }
    .field .tool { width: 40px; height: 38px; margin-left: 2px; }
    .field .tool.on { background: none; }
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

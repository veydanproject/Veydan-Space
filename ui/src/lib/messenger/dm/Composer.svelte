<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import EmojiPicker from '../shared/emoji/EmojiPicker.svelte';
  import RecorderBar from '../media/RecorderBar.svelte';
  import CircleRecorder from '../media/CircleRecorder.svelte';
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
    /** Buttons left of the input (attach, emoji). */
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
  }
  let {
    disabled = false, placeholder, replyTo, editing, peerTitle, oncancel, onsend, tools, draftKey, oneditlast,
    onrecording, canRecord = true, autofocus = true,
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
    try {
      await onrecording?.({ kind, mime: c.mime, duration_ms: c.durationMs, waveform: kind === "voice" ? c.waveform : undefined, blob: c.blob });
    } catch {
      // The chat shows why; nothing to keep here.
    }
  }

  function recordingFailed(code: string) {
    recording = null;
    recError = code;
  }

  const drafts: Map<string, string> = ((globalThis as Record<string, unknown>).__msgDrafts ??= new Map()) as Map<string, string>;
  const MAX_BYTES = 32 * 1024;
  let text = $state('');
  let el = $state<HTMLTextAreaElement | null>(null);

  const bytes = $derived(new TextEncoder().encode(text).length);
  const tooLong = $derived(bytes > MAX_BYTES);
  // Not gated on a send in flight: the next message can be typed and sent
  // while the previous one is still on its way (order is kept by the chat).
  const canSend = $derived(!disabled && text.trim().length > 0 && !tooLong);

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
    el.style.height = `${Math.min(el.scrollHeight, 180)}px`;
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

  // On a phone Enter is a new line; sending is the button.
  const touch = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && !touch) { e.preventDefault(); submit(); }
    else if (e.key === 'Escape' && (replyTo || editing)) { e.preventDefault(); oncancel(); }
    else if (e.key === "ArrowUp" && !text && !editing && oneditlast) { e.preventDefault(); oneditlast(); }
  }
</script>

<div class="composer">
  {#if emojiOpen}
    <EmojiPicker onpick={(e) => insert(e)} onclose={() => (emojiOpen = false)} />
  {/if}
  {#if editing || replyTo}
    <div class="context">
      <Icon name={editing ? 'pencil' : 'reply'} size={14} />
      <span class="ctx-body">
        <span class="ctx-title">{editing ? $t('msg_composer_editing') : $t('msg_composer_reply_to', { name: replyTo!.direction === 'out' ? $t('msg_you') : peerTitle })}</span>
        <span class="ctx-text">{(editing ?? replyTo)!.text ?? ''}</span>
      </span>
      <button class="icon" onpointerdown={keepFocus} onmousedown={keepFocus} onclick={oncancel} title={$t('msg_back')}><Icon name="x" size={14} /></button>
    </div>
  {/if}
  <div class="row">
    {#if recording === "voice"}
      <RecorderBar oncancel={() => (recording = null)} ondone={(c) => recorded("voice", c)} onerror={recordingFailed} />
    {:else}
    {#if tools}{@render tools()}{/if}
    <button class="tool" class:on={emojiOpen} data-emoji-toggle tabindex="-1" {disabled}
      onpointerdown={keepFocus} onmousedown={keepFocus} onclick={() => (emojiOpen = !emojiOpen)} title={$t("msg_emoji_title")}>
      <Icon name="smile" size={18} />
    </button>
    <textarea
      bind:this={el} bind:value={text} rows="1" {disabled}
      placeholder={placeholder ?? $t('msg_composer_placeholder')}
      oninput={resize} {onkeydown}
    ></textarea>
    {#if !text.trim() && !editing && onrecording && canRecord && !disabled && (voiceOk || circleOk)}
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
  @media (pointer: coarse) {
    .composer { padding-bottom: calc(var(--sp-2) + var(--sab, 0px)); }
    textarea { font-size: 16px; min-height: 44px; border-radius: 22px; padding: 10px 14px; }
    .send, .tool { width: 44px; height: 44px; }
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import type { NoteListItem } from '$lib/notes/mobile/api';
  import { locale, t, type Key } from '$lib/core/mobile/i18n';
  import { fmtListDate, noteColor, type RowAction, type RowMode } from '$lib/notes/mobile/notes-editor';

  interface Props {
    note: NoteListItem;
    mode?: RowMode;
    /** Multi-select is on: taps toggle instead of opening. */
    selecting?: boolean;
    selected?: boolean;
    onopen: (note: NoteListItem) => void;
    ontoggle: (note: NoteListItem) => void;
    onlongpress: (note: NoteListItem) => void;
    /** Swipe right. */
    onquick: (note: NoteListItem) => void;
    /** Buttons revealed by a swipe left. */
    onaction: (action: RowAction, note: NoteListItem) => void;
  }

  let { note, mode = 'normal', selecting = false, selected = false, onopen, ontoggle, onlongpress, onquick, onaction }: Props = $props();

  const REVEAL = 144;
  const TRIGGER = 60;

  let dx = $state(0);
  let dragging = $state(false);
  let startX = 0;
  let startY = 0;
  let axis: 'x' | 'y' | null = null;
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  let suppressTap = false;
  let tagDrag = false;

  type Btn = { action: RowAction; icon: string; label: Key; danger?: boolean };
  const buttons = $derived.by((): Btn[] => {
    switch (mode) {
      case 'trash':
        return [
          { action: 'restore', icon: 'rotate-ccw', label: 'notes_restore' },
          { action: 'purge', icon: 'trash-2', label: 'notes_purge', danger: true },
        ];
      case 'archived':
        return [
          { action: 'unarchive', icon: 'archive-restore', label: 'notes_unarchive' },
          { action: 'delete', icon: 'trash-2', label: 'notes_delete', danger: true },
        ];
      default:
        return [
          { action: 'pin', icon: 'pin', label: note.pinned ? 'notes_unpin' : 'notes_pin' },
          { action: 'archive', icon: 'archive', label: 'notes_archive' },
        ];
    }
  });

  function clearPress() {
    clearTimeout(pressTimer);
    pressTimer = undefined;
  }

  function onTouchStart(e: TouchEvent) {
    if (selecting) return;
    const t0 = e.touches[0];
    startX = t0.clientX;
    startY = t0.clientY;
    axis = null;
    dragging = true;
    suppressTap = false;
    pressTimer = setTimeout(() => {
      pressTimer = undefined;
      suppressTap = true;
      dx = 0;
      onlongpress(note);
    }, 500);
  }

  function onTouchMove(e: TouchEvent) {
    if (!dragging) return;
    const t0 = e.touches[0];
    const mx = t0.clientX - startX;
    const my = t0.clientY - startY;
    if (!axis) {
      if (Math.abs(mx) < 8 && Math.abs(my) < 8) return;
      axis = Math.abs(mx) > Math.abs(my) ? 'x' : 'y';
      clearPress();
    }
    if (axis !== 'x') return;
    suppressTap = true;
    // Rubber-band past the reveal width; quick menu needs a short pull right.
    const base = dx < -TRIGGER ? -REVEAL : 0;
    let next = base + mx;
    if (next > 96) next = 96 + (next - 96) * 0.2;
    if (next < -REVEAL) next = -REVEAL + (next + REVEAL) * 0.2;
    dx = next;
  }

  function onTouchEnd() {
    clearPress();
    if (!dragging) return;
    dragging = false;
    if (axis !== 'x') {
      dx = dx <= -TRIGGER ? -REVEAL : 0;
      return;
    }
    if (dx > TRIGGER) {
      dx = 0;
      onquick(note);
    } else if (dx < -TRIGGER) {
      dx = -REVEAL;
    } else {
      dx = 0;
    }
  }

  function onTap() {
    if (suppressTap) {
      suppressTap = false;
      return;
    }
    if (dx !== 0) {
      dx = 0;
      return;
    }
    if (selecting) ontoggle(note);
    else onopen(note);
  }

  function act(action: RowAction) {
    dx = 0;
    onaction(action, note);
  }
</script>

<div class="swipe m-card" class:selected>
  <div class="reveal" class:shown={dx < 0} style:width="{REVEAL}px">
    {#each buttons as b (b.action)}
      <button type="button" class="rbtn" class:danger={b.danger} onclick={() => act(b.action)}>
        <Icon name={b.icon} size={18} />
        <span>{$t(b.label)}</span>
      </button>
    {/each}
  </div>
  <button
    type="button"
    class="m-note"
    class:dragging
    style:transform="translateX({dx}px)"
    style:--c={noteColor(note.chips)}
    ontouchstart={onTouchStart}
    ontouchmove={onTouchMove}
    ontouchend={onTouchEnd}
    ontouchcancel={onTouchEnd}
    onclick={onTap}
    oncontextmenu={(e) => e.preventDefault()}
  >
    {#if selecting}
      <span class="check" class:on={selected}>{#if selected}<Icon name="check" size={14} />{/if}</span>
    {/if}
    <span class="m-doc" class:muted={mode === 'trash'}><Icon name="file-text" size={16} /></span>
    <span class="body">
      <span class="top">
        <span class="title">{note.title || $t('notes_untitled')}</span>
        <span class="date">
          {#if note.pinned && mode === 'normal'}<Icon name="pin" size={12} />{/if}
          {fmtListDate(note.updated_at, $locale)}
        </span>
      </span>
      {#if note.preview}<span class="preview">{note.preview}</span>{/if}
      {#if note.chips.length}
        <!-- Horizontal strip: don't let the card swipe steal the drag. -->
        <span
          class="tags"
          ontouchstart={(e) => { tagDrag = false; e.stopPropagation(); }}
          ontouchmove={(e) => { tagDrag = true; e.stopPropagation(); }}
          ontouchend={(e) => e.stopPropagation()}
          onclick={(e) => { if (tagDrag) { e.stopPropagation(); e.preventDefault(); } }}
        >
          {#each note.chips as c (c.kind + c.id)}
            <span class="m-chip small tinted" style:--chip={c.color}><ChipMark kind={c.kind} />{c.label}</span>
          {/each}
        </span>
      {/if}
    </span>
  </button>
</div>

<style>
  .swipe {
    position: relative;
    overflow: hidden;
  }
  .reveal {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    display: flex;
    visibility: hidden;
  }
  .reveal.shown { visibility: visible; }
  .rbtn {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: var(--accent);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
  }
  .rbtn.danger { background: var(--danger); }
  .m-note {
    background: var(--m-card);
    border-radius: var(--m-radius);
    transition: transform var(--dur-base) var(--ease);
  }
  .m-note.dragging { transition: none; }
  .tags {
    display: flex;
    flex-wrap: nowrap;
    gap: 6px;
    min-width: 0;
    margin-top: 2px;
    overflow-x: auto;
    scrollbar-width: none;
    touch-action: pan-x pan-y;
  }
  .tags::-webkit-scrollbar { display: none; }
</style>

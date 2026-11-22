<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import type { NoteListItem } from '$lib/notes/mobile/api';
  import { t, type Key } from '$lib/core/mobile/i18n';
  import { type RowAction, type RowMode } from '$lib/notes/mobile/notes-editor';
  import { relativeTime } from '$lib/notes/relative-time';

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
    /** This row's buttons are the shown ones; the list keeps one row open and closes it when it goes false. */
    revealed?: boolean;
    /** Another row is open: a touch here closes it, so the tap that does it opens nothing. */
    otherRevealed?: boolean;
    /** The buttons were shown (`true`) or hidden (`false`) by this row. */
    onreveal?: (open: boolean) => void;
    /** The scope line: the folder, workspace or profile the note is in, in its own colour; «Global» without it. */
    scope?: { label: string; color?: string };
    /** What the scope line names, so the chips do not repeat it. */
    hide?: { folder?: string; binding?: string };
  }

  let { note, mode = 'normal', selecting = false, selected = false, onopen, ontoggle, onlongpress, onquick, onaction, revealed = false, otherRevealed = false, onreveal, scope, hide }: Props = $props();

  const chips = $derived(note.chips.filter((c) => !(c.kind === 'folder' && c.id === hide?.folder) && `${c.kind}:${c.id}` !== hide?.binding));

  const REVEAL = 144;
  const TRIGGER = 60;

  let dx = $state(0);
  let dragging = $state(false);
  let startX = 0;
  let startY = 0;
  let axis: 'x' | 'y' | null = null;
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  let suppressTap = false;

  type Btn = { action: RowAction; icon: string; label: Key; danger?: boolean };
  const buttons = $derived.by((): Btn[] => {
    switch (mode) {
      case 'trash':
        return [
          { action: 'restore', icon: 'rotate-ccw', label: 'common_restore' },
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

  /** Rests the row open or closed and tells the list. */
  function settle(open: boolean) {
    dx = open ? -REVEAL : 0;
    if (open !== revealed) onreveal?.(open);
  }

  // The list opened another row or closed this one (a touch elsewhere).
  $effect(() => {
    if (!revealed && !dragging && dx < 0) dx = 0;
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
    suppressTap = otherRevealed;
    pressTimer = setTimeout(() => {
      pressTimer = undefined;
      suppressTap = true;
      settle(false);
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
      settle(dx <= -TRIGGER);
      return;
    }
    if (dx > TRIGGER) {
      settle(false);
      onquick(note);
    } else {
      settle(dx < -TRIGGER);
    }
  }

  function onTap() {
    if (suppressTap) {
      suppressTap = false;
      return;
    }
    if (dx !== 0) {
      settle(false);
      return;
    }
    if (selecting) ontoggle(note);
    else onopen(note);
  }

  function act(action: RowAction) {
    settle(false);
    onaction(action, note);
  }
</script>

<div class="swipe" data-note-id={note.id} class:selected class:moved={dx !== 0 || dragging}>
  <div class="clip">
    <div class="reveal" class:shown={dx < 0} style:width="{REVEAL}px">
      {#each buttons as b (b.action)}
        <button type="button" class="rbtn" class:danger={b.danger} onclick={() => act(b.action)}>
          <Icon name={b.icon} size={18} />
          <span>{$t(b.label)}</span>
        </button>
      {/each}
    </div>
    <div class="list-row row" class:dragging style:transform="translateX({dx}px)">
      <button
        type="button"
        class="list-row-main"
        ontouchstart={onTouchStart}
        ontouchmove={onTouchMove}
        ontouchend={onTouchEnd}
        ontouchcancel={onTouchEnd}
        onclick={onTap}
        oncontextmenu={(e) => e.preventDefault()}
      >
        {#if selecting}
          <span class="m-tick" class:on={selected}>{#if selected}<Icon name="check" size={14} />{/if}</span>
        {/if}
        <span class="list-row-body">
          <span class="list-row-top">
            <span class="list-row-name title">
              {#if note.pinned && mode === 'normal'}<Icon name="pin" size={14} />{/if}
              <span class="title-text">{note.title || $t('notes_untitled')}</span>
            </span>
            <span class="meta">
              {#if note.raw.has_draft}<span class="status-dot warn" title={$t('notes_unsaved_draft')}></span>{/if}
              <span class="badge">{note.raw.format}</span>
            </span>
          </span>
          <span class="mid">
            <span class="scope tinted-text" style:--chip={scope?.color}>{scope?.label ?? $t('notes_scope_global')}</span>
            <span class="time">{relativeTime(note.updated_at, $t)}</span>
          </span>
          {#if note.preview}<span class="preview">{note.preview}</span>{/if}
          {#if chips.length}
            <span class="chips">
              {#each chips.slice(-3) as c (c.kind + c.id)}
                <span class="badge tinted" style:--chip={c.color}><ChipMark kind={c.kind} /><span class="chip-label">{c.label}</span></span>
              {/each}
              {#if chips.length > 3}<span class="badge more">+{chips.length - 3}</span>{/if}
            </span>
          {/if}
        </span>
      </button>
      <!-- The row's sheet without the swipe: beside the row's button, not in it; gone while choosing or slid aside. -->
      {#if !selecting}
        <button type="button" class="icon-btn row-more" class:aside={dx !== 0} onclick={() => onquick(note)} aria-label={$t('nav_more')}>
          <Icon name="more-vertical" size={22} />
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  /* A row of the list: the common row of a record (base.css .list-row, 64, padding --sp-2 --sp-3; with a
     preview it grows), a hairline between rows (`.divided` of the list), the accent tint when chosen.
     A swipe needs the row opaque over its buttons. */
  /* The swipe is clipped inside, so the hairline over the row (`.divided`) stays seen. */
  .clip {
    position: relative;
    overflow: hidden;
    border-radius: var(--radius-lg);
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
    font-size: var(--fs-xs);
    font-weight: var(--fw-semibold);
  }
  .rbtn.danger { background: var(--danger); }
  .row { transition: transform var(--dur-base) var(--ease), background var(--dur-fast) var(--ease); }
  .moved .row { background: var(--m-card); }
  .selected .row { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-border); }
  .row.dragging { transition: none; }
  .row .list-row-body { gap: 4px; }
  .row-more.aside { visibility: hidden; }
  .row .title { flex: 1; display: flex; align-items: center; gap: var(--sp-1); }
  .row .title :global(svg) { flex: none; color: var(--text-2); }
  .title-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { flex: none; display: inline-flex; align-items: center; gap: var(--sp-2); }
  .preview {
    font-size: var(--fs-sm); color: var(--text-2);
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
  }
  .mid { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .scope {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
    font-weight: var(--fw-semibold);
    color: var(--text-2);
  }
  .time { flex: none; margin-left: auto; font-size: var(--fs-xs); color: var(--text-2); white-space: nowrap; }
  /* The labels and their «+N» stay on one line: a long label gives way (an ellipsis), «+N» never does. */
  .chips { display: flex; align-items: center; gap: 4px; min-width: 0; padding-top: 2px; overflow: hidden; }
  .chips .badge { min-width: 0; }
  .chips .more { flex-shrink: 0; }
  .chip-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
</style>

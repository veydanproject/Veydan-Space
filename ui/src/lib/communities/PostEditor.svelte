<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The extended editor of a post (the redesign of the channels, stage 6),
  opened by ⤢ of the channel's composer with what was typed there and the
  files picked, or by «Изменить» of a post. A computer: it takes the whole
  pane of the channel, the title, the text with the bar of Markdown and the
  files on the left, the post as the feed draws it on the right («В ленте»,
  three lines, or «Целиком», as its page). A phone (or a narrow pane): the
  whole screen, «Текст» and «Просмотр» as tabs, the bar over the keyboard.

  The words go as Markdown unless «Обычный текст» is picked. The title is
  the first line of the post (postEditor.ts `joinPost`); with files the
  words are the caption of the album. An edit changes the words alone: the
  files of a post that went do not change. A new post's draft is kept in
  memory while the app runs (`postDrafts`); closing the editor gives it
  back to the composer.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { tick, untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { AttachButton, onKeyboard, onPhone, myKey, type MdBlock } from '$lib/messenger/seams/shared';
  import FeedPost from './FeedPost.svelte';
  import { pickedIcon, type Picked } from './picked.svelte';
  import { channelsApi, mediaOf, type ChannelDetail, type FeedItemView, type TextFormat } from './api';
  import { channelError } from './errors';
  import {
    applyEditorMark, editorMarkOfKey, isEmptyDraft, joinPost, postDrafts, previewId, previewItems, type EditorMark, type PostDraft,
  } from './postEditor';

  /** Where a post goes: the name of the section, and its look when it has one (a CSS colour, an icon of the set). */
  export interface EditorSection {
    name: string;
    color?: string | null;
    icon?: string | null;
  }

  interface Props {
    /** The chat of the channel (`channel:<id>`): the draft is kept by it. */
    chatId: string;
    detail: ChannelDetail;
    /** What the editor opens with. */
    draft: PostDraft;
    section?: EditorSection | null;
    /** A new post: the composer's tray, the files picked (they stay in it when the editor closes). */
    picked?: Picked | null;
    /** Files may be added. */
    canAttach?: boolean;
    /** An edit: the post (the last of its album) and the parts of its album. */
    item?: FeedItemView | null;
    parts?: FeedItemView[] | null;
    /** Sends the post or the edit; a refusal is thrown and the editor stays. */
    onpublish: (text: string, fmt: TextFormat | null) => Promise<void>;
    /** Closed without sending: what was written, for the composer. */
    onclose: (draft: PostDraft) => void;
  }
  let { chatId, detail, draft, section = null, picked = null, canAttach = false, item = null, parts = null, onpublish, onclose }: Props = $props();
  /** The chat the draft is kept by: the one the editor opened in, also when the page moves on to another. */
  const draftKey = untrack(() => chatId);

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const phone = onPhone();
  const touch = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;
  const MOD = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';

  const editing = $derived(!!item);
  let title = $state(untrack(() => draft.title));
  let body = $state(untrack(() => draft.body));
  let md = $state(untrack(() => draft.md));
  let busy = $state(false);
  let error = $state('');

  /** The pane is too narrow for two columns: tabs, as on a phone. */
  let width = $state(0);
  const narrow = $derived(phone || (width > 0 && width < 820));
  let tab = $state<'text' | 'preview'>('text');
  let look = $state<'feed' | 'whole'>('feed');

  const text = $derived(joinPost(title, body, md));
  const files = $derived(picked?.files ?? []);
  const empty = $derived(!text.trim() && files.length === 0);
  const original = untrack(() => joinPost(draft.title, draft.body, draft.md));
  const unchanged = $derived(editing && text === original && md === (item?.fmt === 'md'));
  const canPublish = $derived(!busy && (editing ? !!text.trim() && !unchanged : !empty));

  // ── The draft, kept while the app runs ──────────────────────────────────

  let saved = $state(false);
  /** Sent or closed: nothing is kept any more. */
  let done = false;
  const current = (): PostDraft => ({ title, body, md });
  $effect(() => {
    if (editing) return;
    const d = current();
    saved = false;
    const timer = setTimeout(() => { if (!done) { postDrafts.set(draftKey, d); saved = !isEmptyDraft(d); } }, 500);
    return () => clearTimeout(timer);
  });
  // Left some other way (another channel, the page gone): the draft stays for the next ⤢.
  $effect(() => () => { if (!done && !editing) postDrafts.set(draftKey, untrack(current)); });

  // ── The preview ────────────────────────────────────────────────────────

  /** The words the preview shows and their blocks, read by the runtime a moment after the typing stops. */
  let shown = $state<{ text: string; rich: MdBlock[] | null }>(untrack(() => ({ text, rich: null })));
  let asked = 0;
  $effect(() => {
    const words = text;
    const on = md;
    if (!on || !words.trim()) { asked += 1; shown = { text: words, rich: null }; return; }
    const n = ++asked;
    const timer = setTimeout(() => {
      channelsApi.mdPreview(words).then(
        (rich) => { if (n === asked) shown = { text: words, rich }; },
        () => { if (n === asked) shown = { text: words, rich: null }; },
      );
    }, 150);
    return () => clearTimeout(timer);
  });

  const now = Math.floor(Date.now() / 1000);
  const items = $derived(previewItems({
    communityId: detail.channel.id, author: myKey() ?? '', at: now, text: shown.text, md, rich: shown.rich,
    files: picked ? files : [], edited: item, parts,
  }));
  const last = $derived(items[items.length - 1]);
  const pictures = $derived(picked ? Object.fromEntries(files.map((f) => [previewId(f.key), picked!.previews[f.path] ?? null])) : null);

  // ── Writing ─────────────────────────────────────────────────────────────

  let bodyEl = $state<HTMLTextAreaElement | null>(null);
  let titleEl = $state<HTMLInputElement | null>(null);

  $effect(() => {
    if (touch) return;
    untrack(() => void tick().then(() => (title ? bodyEl : titleEl)?.focus()));
  });

  const MARKS: { mark: EditorMark; icon?: string; word?: string; label: string; key?: string; group: number }[] = [
    { mark: 'bold', icon: 'bold', label: 'chan_fmt_bold', key: 'B', group: 0 },
    { mark: 'italic', icon: 'italic', label: 'chan_fmt_italic', key: 'I', group: 0 },
    { mark: 'strike', icon: 'strikethrough', label: 'chan_fmt_strike', key: 'Shift+X', group: 0 },
    { mark: 'h2', word: 'H2', label: 'chan_fmt_h2', group: 1 },
    { mark: 'h3', word: 'H3', label: 'chan_fmt_h3', group: 1 },
    { mark: 'quote', icon: 'quote', label: 'chan_fmt_quote', group: 2 },
    { mark: 'bullets', icon: 'list', label: 'chan_fmt_list', group: 2 },
    { mark: 'numbers', icon: 'list-ordered', label: 'chan_fmt_numbers', group: 2 },
    { mark: 'code', icon: 'code', label: 'chan_fmt_code', key: 'Shift+M', group: 2 },
    { mark: 'link', icon: 'link', label: 'chan_fmt_link', key: 'K', group: 2 },
    { mark: 'rule', icon: 'separator-horizontal', label: 'chan_fmt_rule', group: 2 },
  ];

  /** A button of the bar, or its shortcut: on the selected words of the text, or the line of its caret. */
  function mark(m: EditorMark) {
    const el = bodyEl;
    if (!el || !md) return;
    const r = applyEditorMark(body, el.selectionStart ?? body.length, el.selectionEnd ?? body.length, m);
    body = r.text;
    void tick().then(() => {
      el.focus({ preventScroll: true });
      el.setSelectionRange(r.start, r.end);
    });
  }

  function bodyKey(e: KeyboardEvent) {
    const m = touch ? null : editorMarkOfKey(e);
    if (m && md) { e.preventDefault(); mark(m); }
    else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void publish(); }
  }

  function titleKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void publish(); }
    else if (e.key === 'Enter') { e.preventDefault(); bodyEl?.focus(); }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && !e.defaultPrevented && !busy) { e.preventDefault(); close(); }
  }

  /** Buttons of the bar keep the focus (and the selection) in the text. */
  const keep = (e: Event) => e.preventDefault();

  async function publish() {
    if (!canPublish) return;
    busy = true;
    error = '';
    try {
      await onpublish(text, md && text.trim() ? 'md' : null);
      done = true;
      if (!editing) postDrafts.drop(draftKey);
    } catch (e) {
      error = e instanceof Error && e.message ? e.message : channelError(e, tr);
    } finally {
      busy = false;
    }
  }

  function close() {
    done = true;
    if (!editing) postDrafts.drop(draftKey);
    onclose(current());
  }

  // The phone: the bar stands over the keyboard.
  let kb = $state(0);
  $effect(() => {
    if (!narrow || !touch) return;
    return onKeyboard((inset) => (kb = inset));
  });

  /** The phone's page lies over the whole screen: in the body, so no glass of the pane holds it. */
  function toBody(node: HTMLElement) {
    if (!phone) return {};
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  /** The files of a post that went, as the edit shows them: what its readers see of each, not to be changed. */
  const kept = $derived((parts?.length ? parts : item ? [item] : []).filter((p) => !p.deleted && p.media_json).map((p) => {
    const m = mediaOf(p);
    return { id: p.id, name: m?.name ?? '', thumb: m?.thumb ? `data:image/jpeg;base64,${m.thumb}` : null, icon: m?.kind === 'image' ? 'image' : m?.kind === 'video' ? 'video' : 'file' };
  }));
  const fileCount = $derived(editing ? kept.length : files.length);
  const sectionName = $derived(section?.name ?? '');
</script>

{#snippet head()}
  <header class="head" class:narrow>
    <button class="icon back" onclick={close} aria-label={$t(narrow ? 'chan_close' : 'chan_back')} title={$t(narrow ? 'chan_close' : 'chan_back')}>
      <Icon name={narrow ? 'x' : 'chevron-left'} size={narrow ? 22 : 20} />
    </button>
    <div class="h-title">
      <span class="h-name">{editing ? $t('chan_edit_title') : $t('chan_editor_new')}</span>
      {#if sectionName}
        <span class="h-where">
          <span class="h-in">{$t('chan_editor_in')}</span>
          <span class="sec" style:--sec={section?.color ?? undefined}>
            <span class="sec-icon"><Icon name={section?.icon ?? 'folder'} size={12} /></span><span class="sec-name">{sectionName}</span>
          </span>
        </span>
      {/if}
    </div>
    {#if !narrow}
      {#if saved && !editing}<span class="saved" role="status">{$t('chan_editor_saved')}</span>{/if}
      {@render formatSwitch()}
    {/if}
    <button class="btn btn-primary publish" disabled={!canPublish} onclick={publish}>
      {#if !narrow}<Icon name={busy ? 'loader' : editing ? 'check' : 'send'} size={15} />{/if}
      {editing ? $t('chan_save') : $t('chan_editor_publish')}
    </button>
  </header>
{/snippet}

{#snippet formatSwitch()}
  <div class="seg fmt-switch" role="radiogroup" aria-label={$t('chan_editor_format')}>
    <button class="seg-btn" class:active={md} role="radio" aria-checked={md} onclick={() => (md = true)}>Markdown</button>
    <button class="seg-btn" class:active={!md} role="radio" aria-checked={!md} onclick={() => (md = false)}>{$t('chan_editor_plain')}</button>
  </div>
{/snippet}

{#snippet toolbar(docked: boolean)}
  <div class="bar" class:docked role="toolbar" aria-label={$t('chan_editor_toolbar')} style:bottom={docked ? `${kb}px` : undefined}>
    <div class="marks">
    {#each MARKS as m, i (m.mark)}
      {#if !docked && i > 0 && MARKS[i - 1].group !== m.group}<span class="bar-sep" aria-hidden="true"></span>{/if}
      {@const label = !touch && m.key ? `${$t(m.label as 'chan_title')} (${MOD}+${m.key})` : $t(m.label as 'chan_title')}
      <button class="tb" class:word={!!m.word} tabindex="-1" disabled={!md} onpointerdown={keep} onmousedown={keep} onclick={() => mark(m.mark)} title={label} aria-label={label}>
        {#if m.icon}<Icon name={m.icon} size={17} />{:else}{m.word}{/if}
      </button>
    {/each}
    </div>
    {#if docked && picked && canAttach}
      <span class="bar-attach"><AttachButton onfiles={(p) => picked?.add(p)} /></span>
    {/if}
  </div>
{/snippet}

{#snippet attachments()}
  {#if editing}
    {#if kept.length}
      <div class="lbl">{$t('chan_editor_attachments', { n: String(kept.length) })}</div>
      <div class="tray">
        {#each kept as k (k.id)}
          <div class="thumb locked" title={k.name}>
            {#if k.thumb}<img src={k.thumb} alt={k.name} draggable="false" />{:else}<Icon name={k.icon} size={20} /><span class="t-name">{k.name}</span>{/if}
            <span class="lock"><Icon name="lock" size={12} /></span>
          </div>
        {/each}
      </div>
      <p class="hint"><Icon name="lock" size={12} />{$t('chan_editor_locked')}</p>
    {/if}
  {:else if picked && (files.length || (canAttach && !(narrow && touch)))}
    <div class="lbl">{fileCount ? $t('chan_editor_attachments', { n: String(fileCount) }) : $t('chan_editor_attachments_none')}</div>
    <div class="tray">
      {#each files as f (f.key)}
        {@const pic = picked.previews[f.path]}
        <div class="thumb" class:picture={!!pic} title={f.name}>
          {#if pic}<img src={pic} alt={f.name} draggable="false" onerror={() => picked?.failed(f.path)} />
          {:else}<Icon name={pickedIcon(f.name)} size={20} /><span class="t-name">{f.name}</span>{/if}
          <button class="unpick" onclick={() => picked?.remove(f.key)} aria-label={$t('chan_editor_remove')} title={$t('chan_editor_remove')}><Icon name="x" size={12} /></button>
        </div>
      {/each}
      {#if canAttach && !(narrow && touch)}
        <div class="thumb add" title={$t('chan_add')}>
          <AttachButton onfiles={(p) => picked?.add(p)} />
          <span class="add-label" aria-hidden="true"><Icon name="plus" size={18} />{$t('chan_add')}</span>
        </div>
      {/if}
    </div>
    {#if files.length}<p class="hint">{$t('chan_editor_media_hint')}</p>{/if}
  {/if}
{/snippet}

{#snippet fields()}
  <label class="lbl" for="post-title">{$t('chan_editor_title_label')}</label>
  <input id="post-title" class="title-field" bind:this={titleEl} bind:value={title} onkeydown={titleKey} disabled={busy}
    placeholder={$t('chan_editor_title_ph')} autocomplete="off" maxlength="300" />
  <div class="lbl-row">
    <label class="lbl" for="post-body">{$t('chan_editor_text_label')}</label>
    {#if !narrow}<span class="lbl-note">{$t('chan_editor_lines_hint')}</span>{/if}
  </div>
  {#if !narrow}{@render toolbar(false)}{/if}
  <textarea id="post-body" class="body-field" class:mono={md} bind:this={bodyEl} bind:value={body} onkeydown={bodyKey} disabled={busy}
    placeholder={$t('chan_editor_text_ph')}></textarea>
  {#if narrow}
    <div class="fmt-row">{@render formatSwitch()}{#if saved && !editing}<span class="saved" role="status">{$t('chan_editor_saved')}</span>{/if}</div>
  {/if}
  {@render attachments()}
{/snippet}

{#snippet lookSwitch()}
  <div class="seg" role="tablist" aria-label={$t('chan_editor_preview')}>
    <button class="seg-btn" class:active={look === 'feed'} role="tab" aria-selected={look === 'feed'} onclick={() => (look = 'feed')}>{$t('chan_editor_in_feed')}</button>
    <button class="seg-btn" class:active={look === 'whole'} role="tab" aria-selected={look === 'whole'} onclick={() => (look = 'whole')}>{$t('chan_editor_whole')}</button>
  </div>
{/snippet}

{#snippet previewCard()}
  {#if empty && !editing}
    <p class="p-empty">{$t('chan_editor_preview_empty')}</p>
  {:else if last}
    <div class="p-card" class:whole={look === 'whole'}>
      {#key look}
        <FeedPost item={last} parts={items.length > 1 ? items : null} {detail} full={look === 'whole'} preview {pictures}
          onread={() => (look = 'whole')} onreact={() => {}} onmenu={() => {}} />
      {/key}
    </div>
    {#if sectionName}<p class="p-note">{$t('chan_editor_preview_note', { section: sectionName })}</p>{/if}
  {/if}
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="editor" class:narrow class:phone role="dialog" aria-modal={phone ? 'true' : undefined} tabindex="-1" aria-label={editing ? $t('chan_edit_title') : $t('chan_editor_new')}
  bind:clientWidth={width} use:toBody {onkeydown}>
  {@render head()}
  {#if error}<div class="err" role="alert"><Icon name="alert-triangle" size={13} />{error}</div>{/if}

  {#if narrow}
    <div class="tabs">
      <div class="seg tabs-seg" role="tablist" aria-label={$t('chan_editor_new')}>
        <button class="seg-btn" class:active={tab === 'text'} role="tab" aria-selected={tab === 'text'} onclick={() => (tab = 'text')}>{$t('chan_editor_text_label')}</button>
        <button class="seg-btn" class:active={tab === 'preview'} role="tab" aria-selected={tab === 'preview'} onclick={() => (tab = 'preview')}>{$t('chan_editor_tab_preview')}</button>
      </div>
    </div>
    <div class="n-body" style:padding-bottom={tab === 'text' && touch ? `${kb + 64}px` : undefined}>
      {#if tab === 'text'}
        <div class="col fields">{@render fields()}</div>
      {:else}
        <div class="col">
          <div class="p-head">{@render lookSwitch()}</div>
          {@render previewCard()}
        </div>
      {/if}
    </div>
    {#if tab === 'text'}
      {#if touch}{@render toolbar(true)}{:else}<div class="n-bar">{@render toolbar(false)}</div>{/if}
    {/if}
  {:else}
    <div class="split">
      <div class="left">{@render fields()}</div>
      <div class="right">
        <div class="p-head"><span class="lbl p-lbl">{$t('chan_editor_preview')}</span>{@render lookSwitch()}</div>
        <div class="p-scroll">{@render previewCard()}</div>
      </div>
    </div>
  {/if}
</div>

<style>
  .editor { position: absolute; inset: 0; z-index: 4; display: flex; flex-direction: column; min-height: 0; outline: none; }
  .editor.phone {
    /* Over the phone's bars and the composer's chips (30), under its sheets (50): the attach sheet opens over it. */
    position: fixed; z-index: 40; background: var(--bg-image), var(--bg);
    padding-top: var(--sat, 0px);
  }

  .head { display: flex; align-items: center; gap: var(--sp-3); min-height: 64px; padding: var(--sp-2) var(--sp-4); border-bottom: 1px solid var(--pane-border); background: var(--pane-head); flex-shrink: 0; }
  .head.narrow { gap: var(--sp-2); padding: var(--sp-1) var(--sp-3) var(--sp-1) var(--sp-1); border-bottom: none; background: none; min-height: 60px; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 8px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .h-title { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-2); }
  .h-name { font-size: var(--fs-lg, 18px); font-weight: var(--fw-bold); white-space: nowrap; }
  .h-in { color: var(--text-3); font-size: var(--fs-sm); }
  .narrow .h-title { flex-direction: column; align-items: flex-start; gap: 0; }
  .narrow .h-name { font-size: var(--fs-md, 17px); }
  .h-where { display: inline-flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .narrow .h-where { gap: 4px; max-width: 100%; }
  .narrow .h-in { font-size: var(--fs-2xs); }
  /* The section the post goes to, in its colour when it has one. */
  .sec {
    --tag: var(--sec, var(--accent));
    display: inline-flex; align-items: center; gap: 6px; min-width: 0; height: 28px; padding: 0 10px 0 4px; border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--tag) 16%, transparent); border: 1px solid color-mix(in srgb, var(--tag) 40%, transparent);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text);
  }
  .sec-icon { width: 20px; height: 20px; border-radius: 6px; display: inline-flex; align-items: center; justify-content: center; background: var(--tag); color: #fff; flex-shrink: 0; }
  .sec-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .narrow .sec { height: auto; padding: 0; border: none; background: none; gap: 4px; font-size: var(--fs-2xs); font-weight: var(--fw-medium, 500); color: color-mix(in srgb, var(--tag) 70%, var(--text)); }
  .narrow .sec-icon { width: auto; height: auto; background: none; color: inherit; }
  .saved { font-size: var(--fs-2xs); color: var(--text-3); white-space: nowrap; }
  .publish { flex-shrink: 0; gap: 6px; }
  .err { display: flex; align-items: center; gap: 6px; padding: 6px var(--sp-4); font-size: var(--fs-xs); color: var(--danger-text); background: var(--danger-bg); border-bottom: 1px solid var(--danger-border); }

  /* A computer: the fields on the left, the preview on the right. */
  .split { flex: 1; min-height: 0; display: flex; }
  .left { width: min(640px, 54%); flex-shrink: 0; display: flex; flex-direction: column; gap: 10px; padding: var(--sp-4) var(--sp-5); border-right: 1px solid var(--pane-border); overflow-y: auto; }
  .right { flex: 1; min-width: 0; display: flex; flex-direction: column; background: color-mix(in srgb, var(--surface-2) 40%, transparent); }
  .p-head { display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-3) var(--sp-5); flex-shrink: 0; }
  .p-lbl { flex: 1; }
  .p-scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-1) var(--sp-6) var(--sp-6); display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); }
  .p-card { width: min(520px, 100%); display: flex; flex-direction: column; }
  .p-card.whole { width: min(680px, 100%); padding: var(--sp-4); border-radius: 18px; background: var(--bubble-in-bg); border: 1px solid var(--bubble-in-border); }
  .p-note { margin: 0; text-align: center; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .p-empty { margin: auto; text-align: center; font-size: var(--fs-sm); color: var(--text-3); max-width: 300px; line-height: 1.5; }

  .lbl { font-size: var(--fs-2xs); font-weight: var(--fw-semibold); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-3); }
  .lbl-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .lbl-note { font-size: var(--fs-2xs); color: var(--text-3); }
  .title-field, .body-field {
    width: 100%; font: inherit; color: var(--text); background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field, 12px);
  }
  .title-field { height: 48px; padding: 0 14px; font-size: var(--fs-lg, 18px); font-weight: var(--fw-bold); }
  .body-field { flex: 1; min-height: 180px; resize: none; padding: 12px 14px; font-size: var(--fs-sm); line-height: 1.6; color: var(--text-body); }
  .body-field.mono { font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.7; }
  .title-field:focus, .body-field:focus { outline: none; border-color: var(--accent-border); }

  /* The bar of Markdown. */
  .bar { display: flex; align-items: center; gap: 4px; padding: 4px; border-radius: var(--radius); background: var(--surface-2); border: 1px solid var(--border); flex-shrink: 0; }
  .marks { flex: 1; min-width: 0; display: flex; align-items: center; gap: 1px; overflow-x: auto; scrollbar-width: none; }
  .marks::-webkit-scrollbar { display: none; }
  .bar-sep { width: 1px; height: 20px; margin: 0 3px; flex-shrink: 0; background: var(--border-2); }
  .tb {
    height: 32px; min-width: 31px; padding: 0 5px; flex-shrink: 0; border: none; border-radius: var(--radius-sm); background: none; cursor: pointer;
    color: var(--text-soft); display: inline-flex; align-items: center; justify-content: center; font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-bold);
  }
  .tb:hover:not(:disabled) { color: var(--text); background: var(--surface-hover); }
  .tb:disabled { opacity: 0.35; cursor: default; }
  /* The phone: docked at the bottom, over the keyboard, a row that scrolls sideways. */
  .bar.docked {
    position: fixed; left: 0; right: 0; z-index: 1;
    border-radius: 0; border: none; border-top: 1px solid var(--border); background: var(--surface); padding: 6px 6px calc(6px + var(--sab, 0px));
  }
  .bar.docked .tb { height: 44px; min-width: 44px; flex-shrink: 0; }
  .bar-attach { flex-shrink: 0; padding-left: 4px; border-left: 1px solid var(--border); --attach-w: 44px; --attach-h: 44px; }
  .n-bar { padding: 0 var(--sp-3) var(--sp-3); flex-shrink: 0; }

  /* The files. */
  .tray { display: flex; gap: 10px; flex-wrap: wrap; }
  .thumb {
    position: relative; width: 84px; height: 84px; flex-shrink: 0; border-radius: var(--radius-md); overflow: hidden;
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; padding: 4px;
    background: var(--surface-2); border: 1px solid var(--border); color: var(--accent-text-2);
  }
  .thumb.picture, .thumb.locked:has(img) { padding: 0; }
  .thumb img { display: block; width: 100%; height: 100%; object-fit: cover; user-select: none; }
  .t-name { max-width: 100%; font-size: var(--fs-2xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .unpick, .lock {
    position: absolute; top: 4px; right: 4px; width: 22px; height: 22px; border: none; border-radius: 50%; padding: 0;
    display: inline-flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.55); color: #fff;
  }
  .unpick { cursor: pointer; }
  .thumb.locked img { opacity: 0.8; }
  .thumb.add { border-style: dashed; background: none; color: var(--text-2); padding: 0; }
  .thumb.add:hover { border-color: var(--border-2); color: var(--text); }
  /* The attach button fills the tile; the words over it let the click through. */
  .thumb.add :global(.attach) { position: absolute; inset: 0; width: 100%; height: 100%; border-radius: 0; opacity: 0; }
  .add-label { pointer-events: none; display: flex; flex-direction: column; align-items: center; gap: 4px; font-size: var(--fs-2xs); font-weight: var(--fw-medium, 500); }
  .hint { margin: 0; display: flex; align-items: center; gap: 6px; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.45; }

  /* Narrow: tabs, one column. */
  .tabs { padding: var(--sp-1) var(--sp-3) var(--sp-2); flex-shrink: 0; }
  .tabs-seg { display: flex; width: 100%; }
  .tabs-seg .seg-btn { flex: 1; justify-content: center; }
  .n-body { flex: 1; min-height: 0; overflow-y: auto; }
  .col { display: flex; flex-direction: column; gap: 10px; padding: var(--sp-1) var(--sp-3) var(--sp-4); min-height: 100%; }
  .col .p-head { padding: 0; }
  .col .p-card { width: 100%; }
  .narrow .body-field { min-height: 240px; flex: 1 0 240px; }
  .fmt-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .fmt-row .seg-btn { height: 30px; font-size: var(--fs-xs); }
  @media (pointer: coarse) {
    .title-field { font-size: var(--fs-md, 17px); }
    .body-field { font-size: var(--fs-md, 16px); }
    .body-field.mono { font-size: var(--fs-sm); }
    .thumb { width: 72px; height: 72px; }
    .unpick { width: 26px; height: 26px; }
  }
</style>

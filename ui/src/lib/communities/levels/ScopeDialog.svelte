<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A section of a channel, new (`scope_add`, communities-wire §14.1) or the
  settings of one (`scope_set`, §14.7; the owner's and the admins'): its title, the line on its
  tile, its mark (an icon of the set, a background picture under it if any),
  its one colour (the icon's tile, and the frame of the background), and whether it
  is hidden — then its title and look are sealed in its own feed, and those
  it is closed to do not learn even its name. Beside them, how the tile and
  the button of the head look, and the order of the sections (each move
  goes at once). How a new section's keys go and how much of its past a
  newcomer reads are chosen under «Скрытый раздел»; neither changes later. A new
  section tells what its terms mean behind a «?» beside each (TermHint).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { confirmStore, isTauriHost, onPhone } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { canSetUpSections } from '../permissions';
  import type { ChannelDetail } from '../api';
  import SectionBadge from './SectionBadge.svelte';
  import TermHint from '$lib/core/ui/TermHint.svelte';
  import { EVERYONE, levelsOf } from './sections';
  import { SCOPE_TITLE_MAX, checkScope, type ScopeProblem } from './editor';
  import { SECTION_COLORS, SECTION_ICONS, moved, ordered, sectionColor, sectionInk } from './look';
  import { scopePhotos } from './scopePhotos.svelte';
  import type { ScopeHistory, ScopeScheme, ScopeSetInput, ScopeView } from './types';

  interface Props {
    open: boolean;
    detail: ChannelDetail;
    /** The section to set up; `null`: a new one. */
    scopeId?: string | null;
  }
  let { open = $bindable(), detail, scopeId = null }: Props = $props();

  const NOTE_MAX = 80;
  const phone = onPhone();
  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const scopes = $derived(ordered(levelsOf(detail).scopes));
  const manager = $derived(canSetUpSections(detail));

  /** The section the form is of (the order list switches it); `null`: new. */
  let target = $state<string | null>(null);
  const was = $derived<ScopeView | null>(target ? (scopes.find((s) => s.scope_id === target) ?? null) : null);
  const isNew = $derived(target === null);
  const everyone = $derived(target === EVERYONE);

  let title = $state('');
  let note = $state('');
  let icon = $state<string | null>(null);
  let color = $state<number | null>(null);
  let mode = $state<'icon' | 'photo'>('icon');
  /** A photo picked for a new section: it goes up once the section is made. */
  let pendingPhoto = $state<string | null>(null);
  let hidden = $state(false);
  let scheme = $state<ScopeScheme>('sd-v1');
  let history = $state<ScopeHistory>('open');
  let tried = $state(false);
  let busy = $state(false);
  let error = $state('');

  /** The form from a section (or blank for a new one, its colour the first one no other section has). */
  function fill(s: ScopeView | null) {
    title = s?.title ?? '';
    note = s?.note ?? '';
    icon = s?.icon ?? null;
    const used = new Set(scopes.map((x) => x.color));
    const free = SECTION_COLORS.findIndex((_, i) => !used.has(i));
    color = s ? (s.color ?? null) : Math.max(0, free);
    mode = s?.photo ? 'photo' : 'icon';
    hidden = !!s?.hidden;
    pendingPhoto = null;
    scheme = 'sd-v1'; history = 'open'; tried = false; error = ''; busy = false;
  }

  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) { target = scopeId; fill(scopeId ? (scopes.find((s) => s.scope_id === scopeId) ?? null) : null); }
    wasOpen = open;
  });

  /** A capital for a word kept lowercase as a tag of a row. */
  const cap = (s: string) => s.charAt(0).toLocaleUpperCase() + s.slice(1);

  const problem = $derived.by<ScopeProblem | null>(() => {
    if (everyone) return null;
    if (isNew) return checkScope(title, scopes);
    const p = checkScope(title, []);
    return p === 'scope_limit' ? null : p;
  });
  const PROBLEM: Record<ScopeProblem, string> = { title_empty: 'chan_scope_err_title', title_long: 'chan_scope_err_long', scope_limit: 'chan_err_scope_limit' };

  /** What the owner or an admin changed of the section, as `channel_scope_set` takes it. */
  const change = $derived.by((): ScopeSetInput => {
    const s = was;
    if (!s) return {};
    const out: ScopeSetInput = {};
    if (title.trim() !== s.title) out.title = title.trim();
    if (note.trim() !== (s.note ?? '')) out.note = note.trim();
    if (icon && icon !== (s.icon ?? null)) out.icon = icon;
    if (color !== (s.color ?? null)) out.color = color ?? -1;
    if (hidden !== !!s.hidden) out.hidden = hidden;
    if (mode === 'icon' && s.photo) out.no_photo = true;
    return out;
  });
  const dirty = $derived(isNew ? !!(title.trim() || note.trim()) : Object.keys(change).length > 0);
  /** Hidden from now on: its title was open to every reader until now. */
  const lateHide = $derived(!!was && !was.hidden && hidden);

  /** A picture from the device: an existing section's goes up at once, as a channel's cover does; a new one's after it is made. */
  async function pickPhoto() {
    let src: string | null = null;
    if (!isTauriHost) src = '/home/dev/Pictures/section.png';
    else {
      const { open: pick } = await import('@tauri-apps/plugin-dialog');
      const picked = await pick({ multiple: false, directory: false, filters: [{ name: $t('chan_scope_badge_photo'), extensions: ['jpg', 'jpeg', 'png'] }] });
      src = typeof picked === 'string' ? picked : null;
    }
    if (!src) return;
    mode = 'photo';
    if (isNew || !target) { pendingPhoto = src; return; }
    const id = target;
    busy = true; error = '';
    try { await channelStore.setScopePhoto(cid, id, src); }
    catch (e) { error = channelError(e, tr); }
    finally { busy = false; }
  }

  async function save() {
    tried = true;
    if (problem) return;
    busy = true; error = '';
    try {
      if (isNew) {
        const before = new Set(scopes.map((s) => s.scope_id));
        const d = await channelStore.addScope(cid, {
          title: title.trim(), scheme, history, icon: icon ?? undefined, color: color ?? undefined, note: note.trim() || undefined, hidden: hidden || undefined,
        });
        const made = levelsOf(d ?? channelStore.detail(cid)).scopes.find((s) => !before.has(s.scope_id));
        if (pendingPhoto && made) await channelStore.setScopePhoto(cid, made.scope_id, pendingPhoto);
      } else if (target && dirty) {
        await channelStore.setScope(cid, target, change);
      }
      open = false;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      busy = false;
    }
  }

  /** Another section's settings: what was changed here is asked about first. */
  async function switchTo(s: ScopeView) {
    if (s.scope_id === target || busy) return;
    if (dirty && !(await confirmStore.ask(tr('chan_scope_discard'), tr('chan_scope_discard_ok'), true))) return;
    target = s.scope_id;
    fill(s);
  }

  /** One place up or down: the new order goes at once. */
  async function move(index: number, by: number) {
    const ids = moved(scopes.map((s) => s.scope_id), index, by);
    busy = true; error = '';
    try { await channelStore.orderScopes(cid, ids); }
    catch (e) { error = channelError(e, tr); }
    finally { busy = false; }
  }

  const label = (s: Pick<ScopeView, 'scope_id' | 'title'>) => (s.scope_id === EVERYONE && !s.title ? $t('chan_scope_everyone') : s.title);
  /** The section as the form now says, for the previews. */
  const draft = $derived({ scope_id: target ?? 'new', title: title.trim(), icon: icon ?? undefined, color: color ?? undefined, photo: mode === 'photo' ? was?.photo : undefined });
  const draftColor = $derived(sectionColor(color) ?? 'var(--accent)');
  const draftInk = $derived(sectionInk(color));
  /** The background as the form now says: the section's picture, unless «Убрать» took it away. */
  const photoSrc = $derived(mode === 'photo' && target && was?.photo ? scopePhotos.src(cid, target, was.photo) : null);
  const hasPhoto = $derived(mode === 'photo' && !!(was?.photo || pendingPhoto));
  /** «Убрать»: the section is its icon again (an existing one's picture goes on «Сохранить», as the old «Иконка» did). */
  function dropPhoto() { mode = 'icon'; pendingPhoto = null; }
  const draftName = $derived(title.trim() || (everyone ? $t('chan_scope_everyone') : $t('chan_scope_title')));
</script>

<!-- A new one: beside its title, the «?» of what it is. -->
{#snippet newAside()}<TermHint term={$t('chan_scope_new')}>{$t('chan_scope_new_hint')}</TermHint>{/snippet}

<Dialog bind:open title={isNew ? $t('chan_scope_new') : $t('chan_scope_edit')} titleAside={isNew ? newAside : undefined} width="min(880px, calc(100vw - 24px))">
  <div class="cols" class:phone>
    <section class="look">
      <span class="label">{$t('chan_scope_look')}</span>
      <div class="look-row">
        <div class="preview" class:fresh={isNew} class:hidden class:framed={hasPhoto} class:pic={!!photoSrc} style:--t-color={draftColor}>
          {#if photoSrc}<img class="p-bg" src={photoSrc} alt="" /><span class="p-shade"></span>{/if}
          <span class="p-top">
            <SectionBadge scope={draft} communityId={cid} size={40} noPhoto />
            {#if hidden}<span class="hidden-mark"><Icon name="eye-off" size={14} />{$t('chan_scope_hidden_mark')}</span>{/if}
          </span>
          <span class="p-words"><span class="p-name">{draftName}</span>{#if note.trim()}<span class="p-sub">{note.trim()}</span>{/if}</span>
        </div>
        <div class="p-head" style:--t-color={draftColor}>
          <span class="p-head-text">{$t('chan_scope_head_button')}</span>
          <span class="p-stripe"></span>
        </div>
      </div>
    </section>

    {#if !isNew && manager && scopes.length > 1}
      <section class="order-box">
        <span class="label">{$t('chan_scope_order')}</span>
        <ol class="order">
          {#each scopes as s, i (s.scope_id)}
            <li class="o-row" class:on={s.scope_id === target}>
              <button type="button" class="o-pick" onclick={() => switchTo(s)} disabled={busy} aria-current={s.scope_id === target ? 'true' : undefined}>
                <SectionBadge scope={s} communityId={cid} size={24} />
                <span class="o-name">{label(s)}</span>
                {#if s.hidden}<span class="o-tag">{$t('chan_scope_hidden_mark')}</span>{/if}
              </button>
              <button type="button" class="icon" disabled={busy || i === 0} onclick={() => move(i, -1)} aria-label={`${$t('chan_scope_up')}: ${label(s)}`} title={$t('chan_scope_up')}><Icon name="arrow-up" size={16} /></button>
              <button type="button" class="icon" disabled={busy || i === scopes.length - 1} onclick={() => move(i, 1)} aria-label={`${$t('chan_scope_down')}: ${label(s)}`} title={$t('chan_scope_down')}><Icon name="arrow-down" size={16} /></button>
            </li>
          {/each}
        </ol>
      </section>
    {/if}

    <div class="main">
      <label class="field-wrap">
        <span class="label">{$t('chan_scope_title')}</span>
        <input class="field" type="text" bind:value={title} maxlength={SCOPE_TITLE_MAX} placeholder={everyone ? $t('chan_scope_everyone') : $t('chan_scope_title_ph')}
          aria-invalid={tried && !!problem} disabled={busy} />
      </label>
      {#if tried && problem}<div class="error-msg" role="alert">{$t(PROBLEM[problem] as 'chan_scope_err_title')}</div>{/if}
      <label class="field-wrap">
        <span class="label">{$t('chan_scope_note')}</span>
        <input class="field" type="text" bind:value={note} maxlength={NOTE_MAX} placeholder={$t('chan_scope_note_ph')} disabled={busy} />
      </label>

      <section class="mark">
        <span class="label">{$t('chan_scope_badge')}</span>
        <div class="m-card" style:--t-color={draftColor} style:--t-ink={draftInk}>
          <div class="icons" role="radiogroup" aria-label={$t('chan_scope_badge_icon')}>
            {#each SECTION_ICONS as it (it.key)}
              {@const on = icon === it.key}
              <button type="button" class="ico" class:on role="radio" aria-checked={on} disabled={busy}
                aria-label={$t(`chan_icon_${it.key.replace(/-/g, '_')}` as 'chan_icon_star')} title={$t(`chan_icon_${it.key.replace(/-/g, '_')}` as 'chan_icon_star')}
                onclick={() => (icon = it.key)}><Icon name={it.icon} size={20} /></button>
            {/each}
          </div>

          <div class="m-part">
            <span class="label">{$t('chan_scope_color')}</span>
            <div class="swatches" role="radiogroup" aria-label={$t('chan_scope_color')}>
              {#each SECTION_COLORS as name, i (name)}
                <button type="button" class="sw" class:on={color === i} role="radio" aria-checked={color === i} style:--sw="var(--sec-{name})" disabled={busy}
                  aria-label={$t(`chan_color_${name}` as 'chan_color_violet')} title={$t(`chan_color_${name}` as 'chan_color_violet')} onclick={() => (color = i)}></button>
              {/each}
            </div>
          </div>

          <div class="m-part">
            <span class="label">{$t('chan_scope_badge_photo')}</span>
            <div class="bg-row">
              <div class="bg-box" class:none={!photoSrc} class:framed={hasPhoto}>
                {#if photoSrc}<img src={photoSrc} alt="" />{:else}<Icon name="image" size={24} />{/if}
              </div>
              <div class="bg-acts">
                <span class="bg-btns">
                  <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={pickPhoto}><Icon name="camera" size={18} />{$t(hasPhoto ? 'chan_scope_photo_change' : 'chan_scope_photo_pick')}</button>
                  {#if hasPhoto}<button type="button" class="btn btn-ghost btn-sm drop" disabled={busy} onclick={dropPhoto}><Icon name="x" size={18} />{$t('chan_editor_remove')}</button>{/if}
                </span>
                {#if pendingPhoto}<span class="hint">{pendingPhoto.split(/[\\/]/).pop()}</span>{/if}
                <span class="res"><span class="mono">960×640 px</span><span class="hint">{$t('chan_scope_photo_hint')}</span></span>
              </div>
            </div>
          </div>
        </div>
      </section>

      {#if manager && !everyone}
        <div class="hide-box" class:on={hidden}>
          <div class="hide-text">
            <span class="hide-title"><Icon name="eye-off" size={16} />{$t('chan_scope_hidden')}</span>
            <span class="hint">{$t('chan_scope_hidden_text')}</span>
            {#if lateHide}<span class="late" role="status"><Icon name="alert-triangle" size={14} />{$t('chan_scope_hidden_late')}</span>{/if}
          </div>
          <button type="button" class="toggle" class:on={hidden} role="switch" aria-checked={hidden} aria-label={$t('chan_scope_hidden')} disabled={busy}
            onclick={() => (hidden = !hidden)}></button>
        </div>
      {/if}

      {#if isNew}
        <div class="choice">
          <span class="label term">{$t('chan_scope_scheme')}<TermHint term={$t('chan_scope_scheme')}>
            {#each ['sd', 'flat'] as k (k)}<span class="pl"><b>{cap($t(`chan_scope_scheme_${k}` as 'chan_scope_scheme_sd'))}</b> — {$t(`chan_scope_scheme_${k}_hint` as 'chan_scope_scheme_sd_hint')}</span>{/each}
          </TermHint></span>
          <div class="chips" role="radiogroup" aria-label={$t('chan_scope_scheme')}>
            {#each [['sd-v1', 'sd'], ['flat-v1', 'flat']] as [v, k] (v)}
              <button type="button" class="chip" class:active={scheme === v} role="radio" aria-checked={scheme === v} disabled={busy}
                onclick={() => (scheme = v as ScopeScheme)}>{cap($t(`chan_scope_scheme_${k}` as 'chan_scope_scheme_sd'))}</button>
            {/each}
          </div>
        </div>
        <div class="choice">
          <span class="label term">{$t('chan_rule_history')}<TermHint term={$t('chan_rule_history')}>
            {#each ['open', 'from_join'] as v (v)}<span class="pl"><b>{$t(`chan_history_${v}` as 'chan_history_open')}</b> — {$t(`chan_history_${v}_hint` as 'chan_history_open_hint')}</span>{/each}
          </TermHint></span>
          <div class="chips" role="radiogroup" aria-label={$t('chan_rule_history')}>
            {#each ['open', 'from_join'] as v (v)}
              <button type="button" class="chip" class:active={history === v} role="radio" aria-checked={history === v} disabled={busy}
                onclick={() => (history = v as ScopeHistory)}>{$t(`chan_history_${v}` as 'chan_history_open')}</button>
            {/each}
          </div>
        </div>
        <p class="hint">{$t('chan_scope_fixed')}</p>
      {/if}
      {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
    </div>
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('chan_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy || (tried && !!problem) || (!isNew && !dirty)} aria-busy={busy} onclick={save}>
      {#if busy}<span class="spinner light" aria-hidden="true"></span>{/if}{$t(isNew ? 'chan_add' : 'chan_save')}
    </button>
  {/snippet}
</Dialog>

<style>
  /* The spinner of a primary button, on its fill. */
  .spinner.light { border-color: rgba(255, 255, 255, 0.35); border-top-color: #fff; }
  /*
    A computer: what it looks like and the order on the left, the form on the right.
    A narrow window, a tablet and a phone: one column — what it looks like first, the form, the order last.
  */
  .cols {
    display: grid; grid-template-columns: 272px minmax(0, 1fr); grid-template-rows: auto 1fr; grid-template-areas: 'look main' 'order main';
    column-gap: var(--sp-6); row-gap: var(--sp-5); min-width: 0;
  }
  .look { grid-area: look; }
  .order-box { grid-area: order; }
  .look, .order-box { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .main { grid-area: main; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-4); padding-left: var(--sp-6); border-left: 1px solid var(--divider); }
  .look-row { display: flex; flex-direction: column; gap: var(--sp-2); }
  @media (max-width: 1000px) {
    .cols { grid-template-columns: minmax(0, 1fr); grid-template-rows: none; grid-template-areas: 'look' 'main' 'order'; row-gap: var(--sp-4); }
    .main { padding-left: 0; border-left: none; }
    .look-row { display: grid; grid-template-columns: 240px minmax(0, 1fr); align-items: start; gap: var(--sp-4); }
    .preview { height: 120px; }
  }
  .cols.phone { grid-template-columns: minmax(0, 1fr); grid-template-rows: none; grid-template-areas: 'look' 'main' 'order'; row-gap: var(--sp-4); }
  .cols.phone .main { padding-left: 0; border-left: none; }
  .cols.phone .look-row { display: grid; grid-template-columns: 280px minmax(0, 1fr); align-items: start; gap: var(--sp-4); }
  .cols.phone .preview { height: 140px; }
  @media (max-width: 600px) {
    .cols.phone .look-row, .look-row { display: flex; flex-direction: column; align-items: stretch; gap: var(--sp-2); }
  }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }
  /* A term with its «?»: the row its note opens under. */
  .term { position: relative; display: flex; align-items: center; }
  .pl { display: block; }

  /* What it looks like: the tile (a picture under it is framed in the colour) and the button of the head. */
  .preview {
    position: relative; overflow: hidden; height: 140px; box-sizing: border-box; padding: var(--sp-3); border-radius: var(--radius-head);
    display: flex; flex-direction: column; justify-content: space-between;
    border: 1px solid color-mix(in srgb, var(--t-color) 35%, var(--border)); background: color-mix(in srgb, var(--t-color) 12%, var(--surface-2));
  }
  .preview.fresh { border-color: var(--accent-border); }
  .preview.framed { border: 2px solid var(--t-color); }
  .preview.hidden { border-style: dashed; }
  .p-bg { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; display: block; }
  .p-shade { position: absolute; inset: 0; background: linear-gradient(180deg, transparent 40%, rgba(0, 0, 0, 0.45)); }
  .p-top, .p-words { position: relative; }
  .p-top { display: flex; align-items: flex-start; justify-content: space-between; }
  .p-words { display: flex; flex-direction: column; gap: var(--sp-1); min-width: 0; }
  .p-name { font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .p-sub { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .preview.pic .p-name { color: #fff; }
  .preview.pic .p-sub { color: rgba(255, 255, 255, 0.82); }
  .hidden-mark {
    display: inline-flex; align-items: center; gap: var(--sp-1); height: 22px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    background: var(--danger-bg); color: var(--danger-text); font-size: var(--fs-xs); font-weight: var(--fw-semibold);
  }
  .p-head {
    display: flex; align-items: center; gap: var(--sp-2); height: var(--control-h-lg); box-sizing: border-box; padding: 0 var(--sp-3); border-radius: var(--radius-md);
    background: var(--surface-2); border: 1px solid var(--border); font-size: var(--fs-sm); color: var(--text-2);
  }
  .p-head-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .p-stripe { width: 44px; height: 3px; border-radius: var(--radius-pill); background: var(--t-color); flex-shrink: 0; }

  /* The order: one card of rows, the current one in the accent. */
  .order {
    list-style: none; margin: 0; padding: var(--sp-1); display: flex; flex-direction: column;
    border-radius: var(--radius-lg); background: var(--surface-2); border: 1px solid var(--border);
  }
  .o-row { position: relative; display: flex; align-items: center; gap: var(--sp-1); min-height: var(--control-h-lg); padding-right: var(--sp-1); border-radius: var(--radius-md); }
  .o-row + .o-row::before { content: ''; position: absolute; top: 0; left: var(--sp-2); right: var(--sp-2); height: 1px; background: var(--divider); }
  .o-row.on { background: var(--accent-bg); }
  .o-row.on::before, .o-row.on + .o-row::before { display: none; }
  .o-pick {
    flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-2); height: var(--control-h-lg); padding: 0 var(--sp-2); border: none; background: none;
    font: inherit; font-size: var(--fs-base); color: var(--text); text-align: left; cursor: pointer;
  }
  .o-row.on .o-name { font-weight: var(--fw-bold); }
  .o-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .o-tag { font-size: var(--fs-xs); color: var(--text-3); flex-shrink: 0; }
  .icon {
    flex-shrink: 0; width: var(--control-h-sm); height: var(--control-h-sm); border: none; background: none; color: var(--text-2); cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; padding: 0; border-radius: var(--radius-sm);
  }
  .icon:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .icon:disabled { opacity: 0.3; cursor: default; }

  /* The fields. */
  .field-wrap { display: flex; flex-direction: column; gap: var(--sp-2); }
  .field {
    width: 100%; height: var(--control-h-lg); box-sizing: border-box; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 0 var(--sp-3);
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .field[aria-invalid='true'] { border-color: var(--danger-border); }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.4; }

  /* The mark: the icon always, the one colour (its tile, and the frame of a background), the background if any. */
  .mark { display: flex; flex-direction: column; gap: var(--sp-2); }
  .m-card {
    display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-3); border-radius: var(--radius-lg);
    background: var(--surface-2); border: 1px solid var(--border);
  }
  .m-part { display: flex; flex-direction: column; gap: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--divider); }
  .icons { display: grid; grid-template-columns: repeat(10, minmax(0, 1fr)); gap: var(--sp-2); }
  .ico {
    aspect-ratio: 1; min-height: 36px; display: inline-flex; align-items: center; justify-content: center; padding: 0; cursor: pointer;
    border-radius: var(--radius-md); border: none; background: var(--surface-3); color: var(--text-2);
  }
  .ico:hover:not(:disabled) { color: var(--text); background: var(--surface-hover); }
  .ico.on, .ico.on:hover:not(:disabled) { background: var(--t-color); color: var(--t-ink); box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px var(--t-color); }
  .ico:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  /* Outside the mark of the chosen one, so both are seen. */
  .ico.on:focus-visible { outline-offset: 6px; }
  .swatches { display: flex; flex-wrap: wrap; gap: var(--sp-3); }
  .sw { width: 34px; height: 34px; border-radius: 50%; border: none; padding: 0; cursor: pointer; background: var(--sw); }
  .sw.on { box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px var(--text); }
  .sw:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 4px; }
  .bg-row { display: flex; align-items: center; gap: var(--sp-4); }
  /* The picture of the section in the 3:2 of a tile (960×640): cut to fill, the colour as its frame. */
  .bg-box {
    position: relative; overflow: hidden; flex-shrink: 0; width: 192px; aspect-ratio: 3 / 2; box-sizing: border-box; border-radius: var(--radius-lg);
    border: 2px solid var(--t-color);
  }
  .bg-box img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .bg-box.none { border-style: dashed; border-color: var(--border-2); background: var(--surface-3); display: grid; place-items: center; color: var(--text-3); }
  .bg-box.none.framed { border-color: var(--t-color); }
  .bg-acts { display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-2); min-width: 0; }
  .bg-btns { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .bg-btns .btn { gap: var(--sp-2); }
  .res { display: flex; flex-direction: column; gap: var(--sp-1); }
  .res .mono { font-size: var(--fs-sm); font-weight: var(--fw-medium); color: var(--text); }
  .drop { color: var(--text-2); }

  /* Hidden. */
  .hide-box {
    display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-lg);
    background: var(--surface-2); border: 1px solid var(--border);
  }
  .hide-box.on { background: var(--danger-bg); border-color: var(--danger-border); }
  .hide-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .hide-title { display: inline-flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .hide-box.on .hide-title :global(svg) { color: var(--danger-text); }
  .hide-text .hint { line-height: 1.45; }
  .late { display: inline-flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-xs); line-height: 1.45; color: var(--warn-text); }
  .late :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .choice { display: flex; flex-direction: column; gap: var(--sp-2); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  /* A phone (not a tablet's sheet): five icons a row, the background the whole width. */
  @media (max-width: 600px) {
    .cols.phone .icons { grid-template-columns: repeat(5, minmax(0, 1fr)); }
    .cols.phone .ico { border-radius: var(--radius-lg); }
    .cols.phone .swatches { gap: var(--sp-2); }
    .cols.phone .sw { width: 30px; height: 30px; }
    .cols.phone .bg-row { flex-direction: column; align-items: stretch; }
    .cols.phone .bg-box { width: 100%; }
  }
  @media (pointer: coarse) {
    .field { height: 48px; font-size: var(--fs-md); }
    .chip { min-height: 40px; padding-inline: var(--sp-4); }
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A section of a channel, new (`scope_add`, communities-wire §14.1) or the
  owner's settings of one (`scope_set`, §14.7): its title, the line on its
  tile, its mark (an icon of the set or a photo), its colour, and whether it
  is hidden — then its title and look are sealed in its own feed, and those
  it is closed to do not learn even its name. Beside them, how the tile and
  the button of the head look, and the order of the sections (each move
  goes at once). How a new section's keys go and how much of its past a
  newcomer reads stay under «Подробнее»; neither changes later.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { confirmStore, isTauriHost, onPhone } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { isOwner } from '../permissions';
  import type { ChannelDetail } from '../api';
  import SectionBadge from './SectionBadge.svelte';
  import { EVERYONE, levelsOf } from './sections';
  import { SCOPE_TITLE_MAX, checkScope, type ScopeProblem } from './editor';
  import { SECTION_COLORS, SECTION_ICONS, moved, ordered, sectionColor, sectionInk } from './look';
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
  const owner = $derived(isOwner(detail));

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
  let more = $state(false);
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
    scheme = 'sd-v1'; history = 'open'; more = false; tried = false; error = ''; busy = false;
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

  /** What the owner changed of the section, as `channel_scope_set` takes it. */
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
  const draftName = $derived(title.trim() || (everyone ? $t('chan_scope_everyone') : $t('chan_scope_title')));
</script>

<Dialog bind:open title={isNew ? $t('chan_scope_new') : $t('chan_scope_edit')} width={isNew ? 'min(760px, calc(100vw - 24px))' : 'min(860px, calc(100vw - 24px))'}>
  <div class="cols" class:phone>
    <aside class="side">
      <span class="label">{$t('chan_scope_look')}</span>
      <div class="preview" class:hidden style:--t-color={draftColor}>
        <span class="p-top">
          <SectionBadge scope={draft} communityId={cid} size={40} noPhoto={mode === 'icon'} />
          {#if hidden}<span class="hidden-mark"><Icon name="eye-off" size={13} />{$t('chan_scope_hidden_mark')}</span>{/if}
        </span>
        <span class="p-words"><span class="p-name">{draftName}</span>{#if note.trim()}<span class="p-sub">{note.trim()}</span>{/if}</span>
      </div>
      <div class="p-head" style:--t-color={draftColor}>
        <SectionBadge scope={draft} communityId={cid} size={20} noPhoto={mode === 'icon'} />
        <span class="p-head-text">{$t('chan_scope_head_button')}</span>
        <span class="p-stripe"></span>
      </div>

      {#if !isNew && owner && scopes.length > 1}
        <span class="label order-label">{$t('chan_scope_order')}</span>
        <ol class="order">
          {#each scopes as s, i (s.scope_id)}
            <li class="o-row" class:on={s.scope_id === target}>
              <button type="button" class="o-pick" onclick={() => switchTo(s)} disabled={busy} aria-current={s.scope_id === target ? 'true' : undefined}>
                <SectionBadge scope={s} communityId={cid} size={20} />
                <span class="o-name">{label(s)}</span>
                {#if s.hidden}<span class="o-tag">{$t('chan_scope_hidden_mark')}</span>{/if}
              </button>
              <button type="button" class="icon" disabled={busy || i === 0} onclick={() => move(i, -1)} aria-label={`${$t('chan_scope_up')}: ${label(s)}`} title={$t('chan_scope_up')}><Icon name="arrow-up" size={14} /></button>
              <button type="button" class="icon" disabled={busy || i === scopes.length - 1} onclick={() => move(i, 1)} aria-label={`${$t('chan_scope_down')}: ${label(s)}`} title={$t('chan_scope_down')}><Icon name="arrow-down" size={14} /></button>
            </li>
          {/each}
        </ol>
      {/if}
    </aside>

    <div class="main">
      <label class="field-wrap">
        <span class="label">{$t('chan_scope_title')}</span>
        <input class="field" type="text" bind:value={title} maxlength={SCOPE_TITLE_MAX} placeholder={everyone ? $t('chan_scope_everyone') : $t('chan_scope_title_ph')}
          aria-invalid={tried && !!problem} disabled={busy} />
      </label>
      {#if isNew}<p class="hint">{$t(hidden ? 'chan_scope_hidden_text' : 'chan_scope_new_hint')}</p>{/if}
      {#if tried && problem}<div class="error-msg" role="alert">{$t(PROBLEM[problem] as 'chan_scope_err_title')}</div>{/if}
      <label class="field-wrap">
        <span class="label">{$t('chan_scope_note')}</span>
        <input class="field" type="text" bind:value={note} maxlength={NOTE_MAX} placeholder={$t('chan_scope_note_ph')} disabled={busy} />
      </label>

      <div class="row-head">
        <span class="label">{$t('chan_scope_badge')}</span>
        <div class="seg" role="radiogroup" aria-label={$t('chan_scope_badge')}>
          <button type="button" class="seg-btn" class:active={mode === 'icon'} role="radio" aria-checked={mode === 'icon'} disabled={busy} onclick={() => (mode = 'icon')}>{$t('chan_scope_badge_icon')}</button>
          <button type="button" class="seg-btn" class:active={mode === 'photo'} role="radio" aria-checked={mode === 'photo'} disabled={busy} onclick={() => (mode = 'photo')}>{$t('chan_scope_badge_photo')}</button>
        </div>
      </div>
      {#if mode === 'icon'}
        <div class="icons" role="radiogroup" aria-label={$t('chan_scope_badge_icon')} style:--t-color={draftColor} style:--t-ink={draftInk}>
          {#each SECTION_ICONS as it (it.key)}
            {@const on = icon === it.key}
            <button type="button" class="ico" class:on role="radio" aria-checked={on} disabled={busy}
              aria-label={$t(`chan_icon_${it.key.replace(/-/g, '_')}` as 'chan_icon_star')} title={$t(`chan_icon_${it.key.replace(/-/g, '_')}` as 'chan_icon_star')}
              onclick={() => (icon = it.key)}><Icon name={it.icon} size={20} /></button>
          {/each}
        </div>
      {:else}
        <div class="photo-row">
          <SectionBadge scope={draft} communityId={cid} size={56} />
          <div class="photo-acts">
            <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={pickPhoto}><Icon name="camera" size={13} />{$t(was?.photo || pendingPhoto ? 'chan_scope_photo_change' : 'chan_scope_photo_pick')}</button>
            {#if pendingPhoto}<span class="hint">{pendingPhoto.split(/[\\/]/).pop()}</span>{/if}
            <span class="hint">{$t('chan_scope_photo_hint')}</span>
          </div>
        </div>
      {/if}

      <span class="label">{$t('chan_scope_color')}</span>
      <div class="swatches" role="radiogroup" aria-label={$t('chan_scope_color')}>
        {#each SECTION_COLORS as name, i (name)}
          <button type="button" class="sw" class:on={color === i} role="radio" aria-checked={color === i} style:--sw="var(--sec-{name})" disabled={busy}
            aria-label={$t(`chan_color_${name}` as 'chan_color_violet')} title={$t(`chan_color_${name}` as 'chan_color_violet')} onclick={() => (color = i)}></button>
        {/each}
      </div>

      {#if owner && !everyone}
        <div class="hide-box" class:on={hidden}>
          <div class="hide-text">
            <span class="hide-title"><Icon name="eye-off" size={16} />{$t('chan_scope_hidden')}</span>
            <span class="hint">{$t('chan_scope_hidden_text')}</span>
            {#if lateHide}<span class="late" role="status"><Icon name="alert-triangle" size={13} />{$t('chan_scope_hidden_late')}</span>{/if}
          </div>
          <button type="button" class="toggle" class:on={hidden} role="switch" aria-checked={hidden} aria-label={$t('chan_scope_hidden')} disabled={busy}
            onclick={() => (hidden = !hidden)}></button>
        </div>
      {/if}

      {#if isNew}
        <button type="button" class="more" aria-expanded={more} onclick={() => (more = !more)}>
          {$t('chan_scope_more')}<Icon name={more ? 'chevron-up' : 'chevron-down'} size={13} />
        </button>
        {#if more}
          <div class="choice">
            <span class="label">{$t('chan_scope_scheme')}</span>
            <div class="chips" role="radiogroup" aria-label={$t('chan_scope_scheme')}>
              {#each [['sd-v1', 'sd'], ['flat-v1', 'flat']] as [v, k] (v)}
                <button type="button" class="chip" class:active={scheme === v} role="radio" aria-checked={scheme === v} disabled={busy}
                  onclick={() => (scheme = v as ScopeScheme)}>{cap($t(`chan_scope_scheme_${k}` as 'chan_scope_scheme_sd'))}</button>
              {/each}
            </div>
            <span class="hint">{$t(scheme === 'flat-v1' ? 'chan_scope_scheme_flat_hint' : 'chan_scope_scheme_sd_hint')}</span>
          </div>
          <div class="choice">
            <span class="label">{$t('chan_rule_history')}</span>
            <div class="chips" role="radiogroup" aria-label={$t('chan_rule_history')}>
              {#each ['open', 'from_join'] as v (v)}
                <button type="button" class="chip" class:active={history === v} role="radio" aria-checked={history === v} disabled={busy}
                  onclick={() => (history = v as ScopeHistory)}>{$t(`chan_history_${v}` as 'chan_history_open')}</button>
              {/each}
            </div>
            <span class="hint">{$t(`chan_history_${history}_hint` as 'chan_history_open_hint')}</span>
          </div>
          <p class="hint">{$t('chan_scope_fixed')}</p>
        {/if}
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
  .cols { display: flex; gap: var(--sp-5); min-width: 0; }
  .side { width: 260px; flex-shrink: 0; display: flex; flex-direction: column; gap: var(--sp-3); padding-right: var(--sp-5); border-right: 1px solid var(--divider); }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-3); }
  /* On a phone the form first, then how it looks and the order. */
  .cols.phone { flex-direction: column; gap: var(--sp-4); }
  .cols.phone .side { order: 2; width: auto; padding-right: 0; border-right: none; }
  .label { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); }
  .order-label { margin-top: var(--sp-2); }
  .preview {
    height: 130px; box-sizing: border-box; padding: 14px; border-radius: 16px; display: flex; flex-direction: column; justify-content: space-between;
    border: 1px solid color-mix(in srgb, var(--t-color) 35%, transparent); background: color-mix(in srgb, var(--t-color) 12%, var(--pane-strong));
  }
  .preview.hidden { border-style: dashed; }
  .p-top { display: flex; align-items: flex-start; justify-content: space-between; }
  .p-words { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .p-name { font-size: var(--fs-md); font-weight: var(--fw-semibold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .p-sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hidden-mark { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); font-weight: var(--fw-medium); color: var(--danger-text); }
  .p-head { display: flex; align-items: center; gap: var(--sp-2); height: 36px; padding: 0 10px; border-radius: var(--radius-sm); background: var(--surface-2); font-size: var(--fs-xs); color: var(--text-2); }
  .p-head-text { flex: 1; min-width: 0; }
  .p-stripe { width: 40px; height: 3px; border-radius: 2px; background: var(--t-color); }
  .order { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .o-row { display: flex; align-items: center; gap: 2px; border-radius: var(--radius-sm); border: 1px solid var(--divider); background: var(--surface-2); padding-right: 2px; }
  .o-row.on { border-color: var(--accent-border); background: var(--accent-tint); }
  .o-pick { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-2); padding: 7px 8px; border: none; background: none; font: inherit; font-size: var(--fs-xs); color: var(--text); text-align: left; cursor: pointer; }
  .o-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .o-tag { font-size: var(--fs-2xs); color: var(--text-3); flex-shrink: 0; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .icon:disabled { opacity: 0.35; cursor: default; }
  .field-wrap { display: flex; flex-direction: column; gap: 4px; }
  .field {
    width: 100%; box-sizing: border-box; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 10px;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .field[aria-invalid='true'] { border-color: var(--danger-border); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .row-head { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); margin-top: 2px; }
  .icons { display: grid; grid-template-columns: repeat(10, minmax(0, 1fr)); gap: 6px; }
  .cols.phone .icons { grid-template-columns: repeat(5, minmax(0, 1fr)); }
  .ico {
    aspect-ratio: 1; min-height: 36px; display: inline-flex; align-items: center; justify-content: center; padding: 0; cursor: pointer;
    border-radius: 11px; border: 1px solid var(--border); background: var(--surface-2); color: var(--text-2);
  }
  .ico:hover:not(:disabled) { color: var(--text); border-color: var(--border-2); }
  .ico.on { background: var(--t-color); border-color: var(--t-color); color: var(--t-ink); }
  .photo-row { display: flex; align-items: center; gap: var(--sp-3); }
  .photo-acts { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; min-width: 0; }
  .swatches { display: flex; flex-wrap: wrap; gap: 10px; }
  .sw { width: 30px; height: 30px; border-radius: 50%; border: none; padding: 0; cursor: pointer; background: var(--sw); }
  .sw.on { box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px var(--text); }
  .sw:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 4px; }
  .hide-box {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3); margin-top: 2px; border-radius: var(--radius-md);
    background: var(--surface-2); border: 1px solid var(--border);
  }
  .hide-box.on { background: var(--danger-bg); border-color: var(--danger-border); }
  .hide-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .hide-title { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .hide-box.on .hide-title :global(svg) { color: var(--danger-text); }
  .late { display: inline-flex; align-items: flex-start; gap: 6px; font-size: var(--fs-xs); line-height: 1.45; color: var(--warn-text); }
  .late :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .more {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 4px; padding: 2px 0; border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2);
  }
  .choice { display: flex; flex-direction: column; gap: 6px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  @media (pointer: coarse) {
    .field { font-size: var(--fs-md); }
    .chip { min-height: 40px; padding-inline: 14px; }
    .icon { padding: 10px; }
    .sw { width: 36px; height: 36px; }
  }
</style>

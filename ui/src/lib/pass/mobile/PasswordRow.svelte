<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One password in the phone's list, in three lines as a 2FA code's row (design 16):
  the title and the first TOTP code at the right (a tap copies it, «+N» if there
  are more), the username, then the labels the whole width from the text to «⋮»,
  in a row that wraps, «+N» past three. A tap anywhere else on the row opens the
  password; «⋮» or a long press asks for its actions (`onmenu`).
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { longpress } from '$lib/core/mobile/longpress';
  import { AVATAR_COLORS } from '$lib/core/mobile/nav-colors';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import type { PasswordEntry } from '$lib/pass/types';

  interface Props {
    entry: PasswordEntry;
    /**
     * Labels and the names of linked entities, in the order the row shows them:
     * a label in the user's colour, an entity neutral (no colour); the mark tells the kind.
     */
    chips: { label: string; kind: string; color?: string }[];
    /** The refresh window of the first TOTP code. */
    period?: number;
    onopen: () => void;
    onmenu: () => void;
  }

  let { entry, chips, period = 30, onopen, onmenu }: Props = $props();

  /** Chips past this many become one counter. */
  const MAX_CHIPS = 3;

  function avatarColor(s: string): string {
    let h = 0;
    for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return AVATAR_COLORS[h % AVATAR_COLORS.length];
  }

  const sub = $derived(entry.username || entry.url || '');
  const totp = $derived(entry.totp_ids[0]);
</script>

<div class="list-row row" {@attach longpress(onmenu)}>
  <button type="button" class="list-row-main" onclick={onopen}>
    <span class="m-avatar" style:--c={avatarColor(entry.title)}>{entry.title.charAt(0).toUpperCase()}</span>
    <span class="meta">
      <span class="name">{entry.title}</span>
      {#if sub}<span class="sub">{sub}</span>{/if}
    </span>
  </button>
  {#if totp}
    <span class="code">
      <TotpLiveCode entryId={totp} {period} copiedLabel={$t('totp_copy')} compact />
      {#if entry.totp_ids.length > 1}<span class="badge">+{entry.totp_ids.length - 1}</span>{/if}
    </span>
  {/if}
  <button type="button" class="icon-btn row-more" onclick={onmenu} aria-label={$t('nav_more')}><Icon name="more-vertical" size={22} /></button>
  {#if chips.length}
    <span class="tagline">
      {#each chips.slice(0, MAX_CHIPS) as chip, i (i)}
        <span class="badge tinted" style:--chip={chip.color}><ChipMark kind={chip.kind} />{chip.label}</span>
      {/each}
      {#if chips.length > MAX_CHIPS}
        <span class="badge">{$t('pw_more_tags', { n: String(chips.length - MAX_CHIPS) })}</span>
      {/if}
    </span>
  {/if}
</div>

<style>
  /* The common row of a record (base.css .list-row, 64): the avatar of 48, the code at the end; the labels wrap
     to a line of their own under them. The main button's zone is the whole row (a tap on the labels opens too);
     the code and «⋮» stand over it. */
  .row { cursor: default; flex-wrap: wrap; row-gap: var(--sp-1); }
  .row > .list-row-main { position: static; margin: 0; padding: 0; }
  .row > .list-row-main::before { inset: 0; }
  .row > .code, .row > .row-more { position: relative; z-index: 1; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-0-5); }
  .name { font-size: var(--fs-body); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* From the text's left edge (past the avatar of 48) to «⋮», not narrowed by the code's column. */
  .tagline {
    flex: 1 0 100%; min-width: 0; display: flex; flex-wrap: wrap; gap: var(--sp-1);
    padding: 0 calc(40px + var(--sp-1) - var(--sp-3)) 0 calc(48px + var(--sp-3));
    pointer-events: none;
  }
  .tagline .badge { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  /* The first code, and «+N» more as a neutral badge (as on the computer). */
  .code { flex-shrink: 0; display: inline-flex; align-items: center; gap: var(--sp-1); }
</style>

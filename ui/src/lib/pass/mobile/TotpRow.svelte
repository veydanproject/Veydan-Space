<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One code in the phone's 2FA list, in three lines as a password's row (design 16):
  the name and the live code at the right, the account, then the chips the whole
  width from the text to «⋮», in a row that wraps, «+N» past three. A tap on the
  row opens the entry, a tap on the code copies it; «⋮» or a long press asks for
  its actions (`onmenu`).
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { longpress } from '$lib/core/mobile/longpress';
  import { AVATAR_COLORS } from '$lib/core/mobile/nav-colors';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import type { TotpCode, TotpEntry } from '$lib/pass/types';

  interface Props {
    entry: TotpEntry;
    /** The code of now, from the list's one fetch of every code. */
    code: TotpCode | null;
    /** Profiles, workspaces and labels, in the order the row shows them. */
    chips: { key: string; label: string; kind: string; color?: string }[];
    onopen: () => void;
    oncopy: () => void;
    onmenu: () => void;
  }

  let { entry, code, chips, onopen, oncopy, onmenu }: Props = $props();

  /** Chips past this many become one counter. */
  const MAX_CHIPS = 3;

  function avatarColor(s: string): string {
    let h = 0;
    for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return AVATAR_COLORS[h % AVATAR_COLORS.length];
  }

  const label = $derived(entry.issuer || entry.name);
</script>

<div class="list-row row" {@attach longpress(onmenu)}>
  <button type="button" class="list-row-main" onclick={onopen}>
    <span class="m-avatar" style:--c={avatarColor(label)}>{label.charAt(0).toUpperCase()}</span>
    <span class="meta">
      <span class="name">{label}</span>
      {#if entry.issuer}<span class="account">{entry.name}</span>{/if}
    </span>
  </button>
  <button type="button" class="code" onclick={oncopy} aria-label={$t('totp_copy')}>
    <TotpLiveCode entryId={entry.id} period={entry.period} given={code} size="lg" />
  </button>
  <button type="button" class="icon-btn row-more" onclick={onmenu} aria-label={$t('nav_more')}><Icon name="more-vertical" size={22} /></button>
  {#if chips.length}
    <span class="tagline">
      {#each chips.slice(0, MAX_CHIPS) as chip (chip.key)}
        <span class="badge tinted" style:--chip={chip.color || undefined}><ChipMark kind={chip.kind} />{chip.label}</span>
      {/each}
      {#if chips.length > MAX_CHIPS}
        <span class="badge">{$t('pw_more_tags', { n: String(chips.length - MAX_CHIPS) })}</span>
      {/if}
    </span>
  {/if}
</div>

<style>
  /* The common row of a record (base.css .list-row, 64): the avatar of 48, the code at the end; the chips wrap
     to a line of their own under them. The main button's zone is the whole row (a tap on the chips opens too);
     the code and «⋮» stand over it. */
  .row { cursor: default; flex-wrap: wrap; row-gap: var(--sp-1); }
  .row > .list-row-main { position: static; margin: 0; padding: 0; }
  .row > .list-row-main::before { inset: 0; }
  .row > .code, .row > .row-more { position: relative; z-index: 1; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-0-5); }
  .name { font-size: var(--fs-body); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .account { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .code { flex-shrink: 0; display: inline-flex; padding: 0; border: 0; background: none; color: inherit; font: inherit; cursor: pointer; }
  /* From the text's left edge (past the avatar of 48) to «⋮», not narrowed by the code's column. */
  .tagline {
    flex: 1 0 100%; min-width: 0; display: flex; flex-wrap: wrap; gap: var(--sp-1);
    padding: 0 calc(40px + var(--sp-1) - var(--sp-3)) 0 calc(48px + var(--sp-3));
    pointer-events: none;
  }
  .tagline .badge { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
</style>

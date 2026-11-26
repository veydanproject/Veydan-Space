<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One password in the phone's list, in two tiers: avatar, title (wrapping, never
  cut) and username on the full width; under them, from the text's edge, the
  first TOTP code (a tap copies it) and the labels. The right side stays empty,
  so the floating «+» covers nothing. A tap on the row opens the password; a
  long press asks for its actions (`onmenu`).
-->
<script lang="ts">
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
    /** The password open beside the list (a wide window). */
    active?: boolean;
    onopen: () => void;
    onmenu: () => void;
  }

  let { entry, chips, period = 30, active = false, onopen, onmenu }: Props = $props();

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

<div class="m-card row" class:active {@attach longpress(onmenu)}>
  <button type="button" class="open" onclick={onopen} aria-current={active ? 'page' : undefined}>
    <span class="m-avatar" style:--c={avatarColor(entry.title)}>{entry.title.charAt(0).toUpperCase()}</span>
    <span class="meta">
      <span class="name">{entry.title}</span>
      {#if sub}<span class="sub">{sub}</span>{/if}
    </span>
  </button>
  <!-- The code is a button of its own, so it can't sit inside .open: a tier under it. -->
  {#if totp}
    <span class="code">
      <TotpLiveCode entryId={totp} {period} copiedLabel={$t('totp_copy')} compact />
    </span>
  {/if}
  {#if chips.length}
    <!-- Under the code, so outside .open too; a tap opens like the rest of the row (the keyboard has .open). -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="tagline" onclick={onopen}>
      {#each chips.slice(0, MAX_CHIPS) as chip, i (i)}
        <span class="m-chip small tinted" style:--chip={chip.color}><ChipMark kind={chip.kind} />{chip.label}</span>
      {/each}
      {#if chips.length > MAX_CHIPS}
        <span class="m-chip small neutral">{$t('pw_more_tags', { n: String(chips.length - MAX_CHIPS) })}</span>
      {/if}
    </span>
  {/if}
</div>

<style>
  /* A flat row of the list (the design kit), not a card. */
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    min-height: 64px;
    padding-right: var(--sp-2);
    background: transparent;
    border-color: transparent;
    box-shadow: none;
    border-radius: var(--radius-lg);
  }
  .row:active { background: var(--surface-hover); }
  .row.active { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-border); }
  .open {
    flex: 1 1 100%;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-2) var(--sp-3) var(--sp-4);
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
  }
  .row:active { background: var(--surface-2); }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .name { font-size: var(--fs-body); font-weight: var(--fw-bold); white-space: normal; overflow-wrap: anywhere; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* The second tier starts at the text's edge: .open's left padding, the avatar 36 (.m-avatar) and the gap. */
  .code,
  .tagline { flex: 1 1 100%; min-width: 0; padding-left: calc(var(--sp-4) + 36px + var(--sp-3)); }
  .code { flex-shrink: 0; display: flex; }
  /* Only here: the digits on the text's edge; the computer's list keeps its padding. */
  .code :global(.live.compact) { padding: 0; }
  .tagline { display: flex; flex-wrap: wrap; gap: 4px; padding-right: var(--sp-2); padding-bottom: var(--sp-3); }
  .tagline .m-chip { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
</style>

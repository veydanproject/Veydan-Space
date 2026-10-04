<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One password in the phone's list, in the style of the 2FA list: avatar,
  title and username on the full width, labels on a line of their own, the
  first TOTP code at the right (a tap copies it). A tap on the row opens the
  password; a long press asks for its actions (`onmenu`).
-->
<script lang="ts">
  import { t } from '$lib/core/mobile/i18n';
  import { longpress } from '$lib/core/mobile/longpress';
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import type { PasswordEntry } from '$lib/pass/types';

  interface Props {
    entry: PasswordEntry;
    /** Labels and the names of linked entities, in the order the row shows them. */
    chips: string[];
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
    return NAV_COLORS[h % NAV_COLORS.length];
  }

  const sub = $derived(entry.username || entry.url || '');
  const totp = $derived(entry.totp_ids[0]);
</script>

<div class="m-card row" {@attach longpress(onmenu)}>
  <button type="button" class="open" onclick={onopen}>
    <span class="m-avatar" style:--c={avatarColor(entry.title)}>{entry.title.charAt(0).toUpperCase()}</span>
    <span class="meta">
      <span class="name">{entry.title}</span>
      {#if sub}<span class="sub">{sub}</span>{/if}
      {#if chips.length}
        <span class="tagline">
          {#each chips.slice(0, MAX_CHIPS) as chip, i (i)}
            <span class="m-chip small neutral">{chip}</span>
          {/each}
          {#if chips.length > MAX_CHIPS}
            <span class="m-chip small neutral">{$t('pw_more_tags', { n: String(chips.length - MAX_CHIPS) })}</span>
          {/if}
        </span>
      {/if}
    </span>
  </button>
  {#if totp}
    <span class="code">
      <TotpLiveCode entryId={totp} {period} copiedLabel={$t('totp_copy')} compact />
    </span>
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    min-height: 64px;
    padding-right: var(--sp-2);
  }
  .open {
    flex: 1;
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
  .name { font-size: 15px; font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: 13px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tagline { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .tagline .m-chip { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .code { flex-shrink: 0; }
</style>

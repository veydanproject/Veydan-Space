<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Live TOTP code with a countdown ring; tap copies. Shared by desktop cards, mobile sheets and the phone's 2FA page
     (there the row copies: the code is given and drawn without a button of its own, size lg). -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/pass/api';
  import { copySecret } from '$lib/pass/clipboard';
  import Icon from '$lib/core/Icon.svelte';
  import type { TotpCode } from '$lib/pass/types';

  interface Props {
    entryId: string;
    /** Refresh window in seconds; learned from the first code when omitted */
    period?: number;
    /** The button's tooltip */
    copiedLabel?: string;
    /** Code and ring only, no border or copy icon; fits inside a list row */
    compact?: boolean;
    /** The code from a caller that fetches every code at once: nothing is fetched here, nothing copies (the row does). */
    given?: TotpCode | null;
    /** lg: the digits 20 and the ring 24 of the phone's 2FA page; md: 16 and 22. */
    size?: 'md' | 'lg';
  }

  let { entryId, period = 30, copiedLabel = '', compact = false, given, size = 'md' }: Props = $props();

  const still = $derived(given !== undefined);
  const RADIUS = $derived(size === 'lg' ? 12 : 10);
  const CIRC = $derived(2 * Math.PI * RADIUS);
  const RING = $derived(size === 'lg' ? 24 : 22);

  let fetched = $state<TotpCode | null>(null);
  const code = $derived(still ? (given ?? null) : fetched);

  const offset = $derived(code ? CIRC * (1 - Math.min(1, code.seconds_left / period)) : CIRC);
  // The last seconds tint both; otherwise the ring is the accent (a graphic, 3:1) and the digits are text (--accent-text, 4.5:1 in either theme).
  const late = $derived(!code || code.seconds_left <= 5 ? 'var(--danger-text)' : code.seconds_left <= 10 ? 'var(--warn-text)' : null);
  const color = $derived(late ?? 'var(--accent)');
  const textColor = $derived(late ?? 'var(--accent-text)');

  function formatCode(c: string): string {
    const half = Math.ceil(c.length / 2);
    return `${c.slice(0, half)} ${c.slice(half)}`;
  }

  async function refresh() {
    try {
      fetched = await api.totp.generateCode(entryId);
    } catch {
      /* keep the previous code while the next tick retries */
    }
  }

  onMount(() => {
    if (still) return;
    void refresh();
    const tick = setInterval(refresh, 1000);
    return () => clearInterval(tick);
  });

  /** Synchronous clipboard write inside the tap keeps the user-gesture context on every platform. */
  function copy(e: MouseEvent) {
    e.stopPropagation();
    if (!code) return;
    void copySecret(code.code).catch(() => {});
  }
</script>

{#snippet face()}
  <span class="code" style:color={textColor}>
    {code ? formatCode(code.code) : '••• •••'}
  </span>
  <svg class="ring" viewBox="0 0 28 28" width={RING} height={RING} aria-hidden="true">
    <circle cx="14" cy="14" r={RADIUS} class="track" />
    <circle cx="14" cy="14" r={RADIUS} class="progress" style="stroke: {color}; stroke-dasharray: {CIRC}; stroke-dashoffset: {offset}" />
  </svg>
{/snippet}

{#if still}
  <span class="live compact" class:lg={size === 'lg'}>{@render face()}</span>
{:else}
  <button type="button" class="live" class:compact class:lg={size === 'lg'} onclick={copy} title={copiedLabel}>
    {@render face()}
    {#if !compact}<Icon name="copy" size={13} />{/if}
  </button>
{/if}

<style>
  .live {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-height: 36px;
    padding: 0 var(--sp-2-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    background: transparent;
    color: var(--text-2);
    font: inherit;
    cursor: pointer;
  }
  .live:hover { border-color: var(--border-2); }
  .live.compact { border: 0; padding: 0 var(--sp-1); min-height: 44px; gap: var(--sp-1-5); }
  .live.compact .code { font-size: var(--fs-md); }
  .live.lg .code { font-size: var(--fs-xl); }
  span.live { cursor: inherit; }
  .code {
    font-family: var(--font-mono);
    font-size: var(--fs-body);
    font-weight: var(--fw-bold);
    white-space: nowrap;
  }
  .ring { transform: rotate(-90deg); flex-shrink: 0; }
  .ring circle { fill: none; stroke-width: 3; }
  .track { stroke: var(--border); }
  .progress { transition: stroke-dashoffset 0.9s linear; }
</style>

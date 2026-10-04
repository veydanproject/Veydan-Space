<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Live TOTP code with a countdown ring; tap copies. Shared by desktop cards and mobile sheets. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/pass/api';
  import Icon from '$lib/core/Icon.svelte';
  import type { TotpCode } from '$lib/pass/types';

  interface Props {
    entryId: string;
    /** Refresh window in seconds; learned from the first code when omitted */
    period?: number;
    /** Copied-state label shown briefly after a tap */
    copiedLabel?: string;
    /** Code and ring only, no border or copy icon; fits inside a list row */
    compact?: boolean;
  }

  let { entryId, period = 30, copiedLabel = '', compact = false }: Props = $props();

  const RADIUS = 10;
  const CIRC = 2 * Math.PI * RADIUS;

  let code = $state<TotpCode | null>(null);
  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout>;

  const offset = $derived(code ? CIRC * (1 - Math.min(1, code.seconds_left / period)) : CIRC);
  const color = $derived(
    !code || code.seconds_left <= 5 ? 'var(--danger-text)' : code.seconds_left <= 10 ? 'var(--warn-text)' : 'var(--accent)',
  );

  function formatCode(c: string): string {
    const half = Math.ceil(c.length / 2);
    return `${c.slice(0, half)} ${c.slice(half)}`;
  }

  async function refresh() {
    try {
      code = await api.totp.generateCode(entryId);
    } catch {
      /* keep the previous code while the next tick retries */
    }
  }

  onMount(() => {
    void refresh();
    const tick = setInterval(refresh, 1000);
    return () => {
      clearInterval(tick);
      clearTimeout(copyTimer);
    };
  });

  /** Synchronous clipboard write inside the tap keeps the user-gesture context on every platform. */
  function copy(e: MouseEvent) {
    e.stopPropagation();
    if (!code) return;
    void navigator.clipboard.writeText(code.code).catch(() => {});
    copied = false;
    requestAnimationFrame(() => (copied = true));
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = false), 1600);
  }
</script>

<button type="button" class="live" class:copied class:compact onclick={copy} title={copiedLabel}>
  <span class="code" class:pop={copied} style:color={copied ? 'var(--success-text)' : color}>
    {code ? formatCode(code.code) : '••• •••'}
  </span>
  <svg class="ring" viewBox="0 0 28 28" width="22" height="22" aria-hidden="true">
    <circle cx="14" cy="14" r={RADIUS} class="track" />
    <circle cx="14" cy="14" r={RADIUS} class="progress" style="stroke: {color}; stroke-dasharray: {CIRC}; stroke-dashoffset: {offset}" />
  </svg>
  {#if !compact}<Icon name={copied ? 'check' : 'copy'} size={13} />{/if}
</button>

<style>
  .live {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    min-height: 36px;
    padding: 0 0.6rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-2);
    font: inherit;
    cursor: pointer;
  }
  .live:hover { border-color: var(--border-2); }
  .live.copied { border-color: var(--success-text); }
  .live.compact { border: 0; padding: 0 0.25rem; min-height: 44px; gap: 0.4rem; }
  .live.compact .code { font-size: 16px; }
  .code {
    font-family: var(--font-mono);
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.06em;
    white-space: nowrap;
  }
  .code.pop { animation: code-pop 0.4s ease; }
  @keyframes code-pop {
    0% { transform: scale(1); }
    40% { transform: scale(1.12); }
    100% { transform: scale(1); }
  }
  .ring { transform: rotate(-90deg); flex-shrink: 0; }
  .ring circle { fill: none; stroke-width: 3; }
  .track { stroke: var(--border); }
  .progress { transition: stroke-dashoffset 0.9s linear; }
</style>

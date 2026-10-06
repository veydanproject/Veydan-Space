<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The reactions under a message: one chip per emoji, with how many put it.
  Mine have the accent border; a tap puts mine or takes it back.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { MessengerMessage } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';

  interface Props {
    message: MessengerMessage;
    /** A refusal, for the window to explain. */
    onerror?: (e: unknown) => void;
  }
  let { message: m, onerror }: Props = $props();

  let busy = $state(false);

  async function toggle(emoji: string) {
    if (busy) return;
    busy = true;
    try { await chatStore.react(m.id, emoji); }
    catch (e) { onerror?.(e); }
    finally { busy = false; }
  }
</script>

<div class="reactions msg-font">
  {#each m.reactions as r (r.emoji)}
    <button class="chip" class:mine={r.mine} disabled={busy} title={$t('msg_react_title')} aria-pressed={r.mine}
      onclick={() => toggle(r.emoji)}>
      <span class="emoji">{r.emoji}</span><span class="count">{r.count}</span>
    </button>
  {/each}
</div>

<style>
  .reactions { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip {
    display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; min-height: 24px;
    border: 1px solid var(--border); border-radius: var(--radius-pill); background: var(--surface);
    color: var(--text-2); font: inherit; font-size: var(--fs-xs); line-height: 1; cursor: pointer;
  }
  .chip:hover:not(:disabled) { background: var(--surface-3); }
  .chip:disabled { cursor: default; }
  .chip.mine { border-color: var(--accent); background: var(--accent-tint); color: var(--accent-text-2); }
  .emoji { font-size: 15px; }
  .count { font-variant-numeric: tabular-nums; font-weight: var(--fw-semibold); }
  @media (pointer: coarse) { .chip { min-height: 30px; padding: 3px 10px; } .emoji { font-size: 17px; } }
</style>

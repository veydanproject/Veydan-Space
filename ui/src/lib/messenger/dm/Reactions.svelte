<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The reactions under a message (design kit, M12): one chip per emoji, with
  how many put it. Mine are in the accent; a tap puts mine or takes it back,
  my emoji pops once and the count rolls. «Добавить реакцию» (a computer)
  opens the quick reactions over it. Three different reactions of mine at most.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { MessengerMessage } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { reactionAllowed } from './quick-reactions';

  interface Props {
    message: MessengerMessage;
    /** A refusal, for the window to explain. */
    onerror?: (e: unknown) => void;
    /** «Добавить реакцию»: the quick reactions over this chip; absent, no chip. */
    onadd?: (anchor: HTMLElement) => void;
  }
  let { message: m, onerror, onadd }: Props = $props();

  let busy = $state(false);
  let popped = $state('');

  async function toggle(emoji: string, mine: boolean) {
    if (busy || (!mine && !reactionAllowed(m, emoji))) return;
    busy = true;
    try {
      await chatStore.react(m.id, emoji);
      if (!mine) {
        popped = emoji;
        setTimeout(() => { if (popped === emoji) popped = ''; }, 220);
      }
    }
    catch (e) { onerror?.(e); }
    finally { busy = false; }
  }
</script>

<div class="reactions msg-font">
  {#each m.reactions as r (r.emoji)}
    <button class="chip" class:mine={r.mine} class:pop={popped === r.emoji} disabled={busy} title={$t('msg_react_title')} aria-pressed={r.mine}
      onclick={() => toggle(r.emoji, r.mine)}>
      <span class="emoji">{r.emoji}</span>{#key r.count}<span class="count">{r.count}</span>{/key}
    </button>
  {/each}
  {#if onadd}
    <button class="chip add" onclick={(e) => onadd?.(e.currentTarget)} title={$t('msg_react_title')} aria-label={$t('msg_react_title')}>
      <Icon name="smile-plus" size={18} />
    </button>
  {/if}
</div>

<style>
  .reactions { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip {
    position: relative; display: inline-flex; align-items: center; gap: var(--sp-2); height: var(--control-h-sm); padding: 0 var(--sp-3);
    border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2);
    color: var(--text-2); font: inherit; line-height: 1; cursor: pointer;
    transition: background var(--dur-fast), border-color var(--dur-fast), color var(--dur-fast);
  }
  .chip:hover:not(:disabled) { background: var(--surface-hover); }
  .chip:disabled { cursor: default; }
  .chip.mine { border-color: var(--accent-border); background: var(--accent-bg); color: var(--accent-text); }
  .emoji { font-size: var(--fs-lg); line-height: 1; }
  .count { font-size: var(--fs-base); font-weight: var(--fw-semibold); font-variant-numeric: tabular-nums; animation: roll 120ms var(--ease); }
  .chip.pop .emoji { animation: pop 200ms cubic-bezier(0.2, 0.9, 0.3, 1.15); }
  .chip.add { width: var(--control-h-lg); justify-content: center; padding: 0; border-style: dashed; background: none; color: var(--text-soft); }
  @keyframes pop { 50% { transform: scale(1.2); } }
  @keyframes roll { from { transform: translateY(60%); opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .count, .chip.pop .emoji { animation: none; } }
  @media (pointer: coarse) { .chip::after { content: ''; position: absolute; inset: -9px 0; } }
</style>

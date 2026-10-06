<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The head of a message's menu: six reactions in reach, the emoji I use
  most first, and "more" for the whole picker in their place. Emoji that are
  already mine on the message are marked; picking one again takes it back.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import EmojiPicker from '../shared/emoji/EmojiPicker.svelte';
  import { usageStore } from '../shared/emoji/usageStore.svelte';
  import type { MessengerMessage } from '../api';
  import { quickSet } from './quick-reactions';

  interface Props {
    message: MessengerMessage;
    /** The whole picker instead of the strip. */
    more: boolean;
    onmore: () => void;
    onreact: (emoji: string) => void;
  }
  let { message: m, more, onmore, onreact }: Props = $props();

  const quick = $derived(quickSet(usageStore.popular));
  const mine = $derived(new Set(m.reactions.filter((r) => r.mine).map((r) => r.emoji)));
</script>

{#if more}
  <div class="more-picker"><EmojiPicker variant="inline" onpick={onreact} /></div>
{:else}
  <div class="strip msg-font" role="group" aria-label={$t('msg_react_title')}>
    {#each quick as e (e)}
      <button class="quick" class:mine={mine.has(e)} aria-pressed={mine.has(e)} onclick={() => onreact(e)}>{e}</button>
    {/each}
    <button class="quick other" onclick={onmore} title={$t('msg_react_more')} aria-label={$t('msg_react_more')}>
      <Icon name="plus" size={16} />
    </button>
  </div>
{/if}

<style>
  .strip { display: flex; align-items: center; gap: 2px; white-space: normal; }
  .quick {
    flex: 1 0 auto; display: inline-flex; align-items: center; justify-content: center;
    width: 34px; height: 34px; padding: 0; border: 1px solid transparent; border-radius: 50%;
    background: none; font: inherit; font-size: 20px; line-height: 1; cursor: pointer; color: var(--text-2);
  }
  .quick:hover { background: var(--surface-hover, var(--surface-3)); }
  .quick.mine { border-color: var(--accent); background: var(--accent-tint); }
  .quick.other { font-size: inherit; }
  .more-picker { width: 300px; max-width: 100%; white-space: normal; }
  @media (pointer: coarse) {
    .strip { justify-content: space-between; }
    .quick { width: 44px; height: 44px; font-size: 26px; flex: 0 0 auto; }
    .more-picker { width: 100%; }
  }
</style>

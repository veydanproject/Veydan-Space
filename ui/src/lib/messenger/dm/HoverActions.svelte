<!--
  @component
  The actions of a hovered message on a computer, beside its bubble: «Ответить»,
  the reaction, «Переслать» and «Ещё», round buttons 32 with no plate under them
  (mockups c1.js), the same for words, pictures, files and voice. The one
  whose menu or picker is open stays in accent while it is.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    onreply?: () => void;
    actions: { react?: (anchor: HTMLElement) => void; forward?: () => void; more: (anchor: HTMLElement) => void };
    /** The message's menu or picker is open. */
    selected?: boolean;
  }
  let { onreply, actions, selected = false }: Props = $props();

  /** The button that opened what is open now (only the look of it). */
  let opened = $state<'react' | 'more' | null>(null);
  $effect(() => { if (!selected) opened = null; });
</script>

<div class="acts" class:shown={selected}>
  {#if onreply}<button class="act" onclick={onreply} title={$t('msg_message_reply')} aria-label={$t('msg_message_reply')}><Icon name="reply" size={18} /></button>{/if}
  {#if actions.react}<button class="act" class:open={selected && opened === 'react'} onclick={(e) => { opened = 'react'; actions.react?.(e.currentTarget); }} title={$t('msg_react_title')} aria-label={$t('msg_react_title')} aria-haspopup="true"><Icon name="smile-plus" size={18} /></button>{/if}
  {#if actions.forward}<button class="act" onclick={actions.forward} title={$t('msg_fwd_forward')} aria-label={$t('msg_fwd_forward')}><Icon name="forward" size={18} /></button>{/if}
  <button class="act" class:open={selected && opened === 'more'} onclick={(e) => { opened = 'more'; actions.more(e.currentTarget); }} title={$t('msg_message_more')} aria-label={$t('msg_message_more')} aria-haspopup="menu"><Icon name="more-vertical" size={18} /></button>
</div>

<style>
  .acts { display: flex; gap: 2px; flex-shrink: 0; opacity: 0; }
  :global(.line:hover) .acts, :global(.line:focus-within) .acts, .acts.shown { opacity: 1; }
  /* Muted round buttons, each with its own fill under the pointer only. */
  .act {
    display: inline-flex; align-items: center; justify-content: center; width: 32px; height: 32px; padding: 0;
    border: none; border-radius: 50%; background: none; color: var(--text-2); cursor: pointer;
  }
  .act:hover { background: var(--surface-hover); color: var(--text); }
  .act.open { background: var(--accent-tint); color: var(--accent-text); }
  .act:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  @media (hover: none), (pointer: coarse) { .acts { display: none; } }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The buttons under a bot's message (sdk/docs/bridge.md, «Buttons under a message» D5, D6):
  in the bubble under the words, a line over them, the time under them.

  A press does one thing: `messenger_press` with the button's `d`, nothing
  else (no dialog, no screen, no clipboard). The pressed button waits for
  the bot with a spinner, the others are dimmed, none takes a second press
  until the bot's edit comes or 30 s pass. No button is a link in this
  version (keyboard.ts leaves one out). Labels are text, never HTML.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { toast } from '$lib/core/ui/toast.svelte';
  import { messengerApi, messengerError, type MessengerMessage } from '../api';
  import { keyboardOf, laterPress, waitingPress, PRESS_WAIT_S, type KeyButton, type Press } from './keyboard';

  interface Props { message: MessengerMessage }
  let { message: m }: Props = $props();

  // Never under my own message, nor under one taken back.
  const blocks = $derived(m.direction === 'in' && !m.deleted ? keyboardOf(m.kb) : []);

  /** My press in this window, before the runtime's `pressed` says it. */
  let mine = $state<Press | null>(null);
  /** The command has not answered yet. */
  let sending = $state(false);
  /** Bumped when a wait runs out: the wait is read again. */
  let tick = $state(0);
  const waiting = $derived.by(() => {
    void tick;
    if (sending) return mine;
    return waitingPress(laterPress(m.pressed, mine), blocks, m.edited_at, Date.now() / 1000);
  });
  $effect(() => {
    const w = waiting;
    if (!w || sending) return;
    const left = w.at + PRESS_WAIT_S - Date.now() / 1000;
    const timer = setTimeout(() => tick++, Math.max(0, left) * 1000 + 50);
    return () => clearTimeout(timer);
  });

  async function press(b: KeyButton) {
    if (waiting || sending) return;
    mine = { d: b.d, at: Date.now() / 1000 };
    sending = true;
    try {
      await messengerApi.press(m.id, b.d);
    } catch (e) {
      const why = messengerError(e);
      // Pressed a moment ago (another window of mine): it waits as if pressed here.
      if (/\bpress_pending\b/.test(why)) return;
      mine = null;
      // The runtime's words go to the log; the toast says it in the UI's language.
      console.warn('messenger_press refused:', why);
      toast.error($t('msg_kb_press_failed'));
    } finally {
      sending = false;
    }
  }
</script>

{#snippet key(b: KeyButton, chip: boolean)}
  {@const busy = !!waiting && waiting.d === b.d}
  <button type="button" class="btn key" class:chip-key={chip} class:btn-primary={b.style === 'primary'} class:btn-danger-soft={b.style === 'danger'}
    class:quiet={b.style === 'quiet'} class:busy class:dim={!!waiting && !busy}
    aria-disabled={waiting ? 'true' : undefined} aria-busy={busy ? 'true' : undefined}
    title={b.label}
    onclick={() => press(b)}>
    {#if busy}<span class="ring" aria-hidden="true"></span>{/if}
    <span class="label">{b.label}</span>
  </button>
{/snippet}

{#if blocks.length}
  <div class="kb" class:waiting={!!waiting}>
    {#each blocks as block, i (i)}
      {#if block.type === 'chips'}
        <div class="kb-chips">{#each block.buttons as b, j (j)}{@render key(b, true)}{/each}</div>
      {:else}
        <div class="kb-row">{#each block.buttons as b, j (j)}{@render key(b, false)}{/each}</div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  /* A card in the bubble: a line between it and the words, its buttons the card's width. */
  .kb { display: flex; flex-direction: column; gap: var(--sp-2); margin-top: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--border); user-select: none; -webkit-user-select: none; }
  /* One button the row's width; several of equal widths. */
  .kb-row { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(0, 1fr); gap: var(--sp-2); }
  /* Short labels (a choice of a language) flow as chips and wrap where the card ends. */
  .kb-chips { display: flex; flex-wrap: wrap; gap: var(--sp-2); }

  .key {
    min-width: 0; min-height: var(--touch, var(--control-h)); padding: 0 var(--sp-3); gap: var(--sp-2);
    border-radius: var(--radius-md); font-size: var(--fs-base); font-weight: var(--fw-semibold);
  }
  .key.chip-key { width: fit-content; max-width: 100%; min-height: var(--control-h-sm); border-radius: var(--radius-pill); font-size: var(--fs-sm); }
  @media (pointer: coarse) { .key.chip-key { min-height: 40px; } }
  /* The quiet button: the accent's quiet plate (in a bubble, the bubble's tone) and the accent's text. */
  .key.quiet { background: var(--accent-bg); color: var(--accent-text); border: none; }
  .key.quiet:hover { background: var(--bubble-tone-hover, var(--surface-hover)); }
  /* No shadow under the filled one in a bubble: it lies on it. */
  .key.btn-primary { box-shadow: none; }
  .key:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }

  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* Waiting for the bot: the pressed one spins, the others are dimmed; none takes a press. */
  .kb.waiting .key { pointer-events: none; }
  .key[aria-disabled='true']:active { transform: none; }
  .key.dim { opacity: 0.45; }
  .ring {
    width: 16px; height: 16px; flex: none; box-sizing: border-box; border-radius: 50%;
    border: 2px solid color-mix(in srgb, currentColor 30%, transparent); border-top-color: currentColor;
    animation: kb-spin 0.7s linear infinite;
  }
  @keyframes kb-spin { to { transform: rotate(360deg); } }
  .key { transition: background var(--dur-fast) var(--ease), opacity var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease); }
  @media (prefers-reduced-motion: reduce) {
    .key { transition: none; }
    .ring { animation: none; }
  }
</style>

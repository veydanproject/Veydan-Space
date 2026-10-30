<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The sheet of the paperclip on a phone: the panel of the chosen item above, the strip of items below. It knows no item: they come from the registry (seams/attach). -->
<script lang="ts">
  import { onDestroy, untrack, type Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { attachActions, type AttachContext, type AttachRegistration, type AttachResult } from '../seams/attach.svelte';
  import { holdBack } from './sheetBack';
  import { ResultGate, finishPanel, runAction, type Ticket } from './resultGate';

  interface Props {
    open: boolean;
    ctx: AttachContext;
    /** What was chosen; the sheet is already closed. */
    onresult: (result: AttachResult) => void;
    /** An item that failed (its text is the composer's to say). */
    onerror?: (error: unknown) => void;
  }
  let { open = $bindable(), ctx, onresult, onerror }: Props = $props();

  // A ticket is good only while the very registration it was taken from stands.
  const gate = new ResultGate((r) => attachActions.alive(r));
  // The items are read as the sheet opens: the clipboard may have changed since.
  let regs = $state.raw<AttachRegistration[]>([]);
  let activeId = $state<string | null>(null);
  $effect(() => {
    if (!open) return;
    untrack(() => {
      gate.open(ctx.chatId);
      regs = attachActions.for(ctx);
      activeId = regs.find((r) => r.action.panel)?.action.id ?? null;
    });
  });
  $effect(() => {
    const chat = ctx.chatId;
    untrack(() => gate.moveTo(chat));
  });
  onDestroy(() => gate.destroy());
  // A module that stops while the sheet is open takes its tab back with it.
  const live = $derived(regs.filter((r) => attachActions.alive(r)));
  const active = $derived(live.find((r) => r.action.id === activeId && r.action.panel) ?? null);

  // The panel on show answers with this ticket only; a new ticket is a new panel.
  let ticket = $state.raw<Ticket | null>(null);
  // A bar the panel on show put in place of the strip (its choice); it goes with the panel.
  let panelBar = $state.raw<Snippet | null>(null);
  $effect(() => {
    const r = open ? active : null;
    ctx.chatId;
    untrack(() => {
      gate.show(r?.action.id ?? null);
      ticket = r ? gate.panel(r) : null;
      panelBar = null;
    });
  });
  const barOf = (t: Ticket) => (b: Snippet | null) => {
    if (ticket === t) panelBar = b;
  };

  const close = () => {
    gate.close();
    open = false;
  };
  $effect(() => {
    if (!open) return;
    // Untracked: taking a place in the history writes the page's state, which it also reads.
    return untrack(() => holdBack(close));
  });
  // Closed from outside (the backdrop, the ×): what a panel gives later is not taken.
  $effect(() => {
    if (!open) untrack(() => gate.close());
  });

  function finish(t: Ticket, result: AttachResult | null) {
    // The answer is taken before the sheet closes: closing spends the ticket.
    if (finishPanel(gate, t, result, { onresult, onerror })) close();
  }

  async function choose(r: AttachRegistration) {
    if (r.action.panel) {
      activeId = r.action.id;
      return;
    }
    // `run` outlives the closing of the sheet, so its ticket is taken before.
    const done = runAction(gate, r, ctx, { onresult, onerror });
    close();
    await done;
  }
</script>

<!-- No title and no ×: the handle, a swipe down, Back and the dim close it; the name stays for a screen reader. -->
<Dialog bind:open label={$t('msg_attach_sheet')} expandable>
  <div class="attach-sheet">
    <!-- One height for every tab, so the sheet does not jump; the grid of 1b takes the same place. -->
    {#if active?.action.panel && ticket}
      {@const Panel = active.action.panel}
      {@const tk = ticket}
      <div class="panel">
        {#key tk}
          <Panel {ctx} done={(r) => finish(tk, r)} bar={barOf(tk)} />
        {/key}
      </div>
    {/if}
    {#if panelBar && active}
      <!-- The panel's own bar (a choice in progress) stands where the strip stands, on the same plate. -->
      <div class="strip m-bar">{@render panelBar()}</div>
    {:else}
    <!-- The app's floating bar: the items share its width while they fit, then it scrolls and the last one is cut by the edge. -->
    <div class="strip m-bar m-bar--pick" role="tablist">
      {#each live as r (r.action.id)}
        {@const a = r.action}
        <button class="m-bar-item tab" role="tab" aria-selected={a.id === activeId} class:active={a.id === activeId} onclick={() => choose(r)}>
          <span class="m-bar-ico">
            <Icon name={a.icon} size={22} />
            {#if a.badge === 'new'}<span class="m-bar-badge accent">{$t('msg_attach_new')}</span>{/if}
          </span>
          <span class="m-bar-label">{$t(a.label)}</span>
        </button>
      {/each}
    </div>
    {/if}
  </div>
</Dialog>

<style>
  /* The plate stands as the navigation does: 8 from the screen's sides, --nav-gap above the system's bar. */
  :global(.m-sheet:has(.attach-sheet)) { padding-inline: var(--sp-2); padding-bottom: calc(var(--nav-gap) + var(--sab)); }
  .attach-sheet { display: flex; flex-direction: column; flex: 1; min-height: 0; gap: var(--sp-2); }
  .panel {
    flex: 1 0 auto; height: 40vh; min-height: 0; overflow-y: auto;
    display: flex; flex-direction: column; padding: 0 var(--sp-2);
  }
  /* Pulled up to the whole screen, the panel takes all of it above the strip (the whole gallery). */
  :global(.m-sheet.full) .panel { flex: 1 1 0; height: auto; }
  .strip {
    display: flex; flex-shrink: 0; min-height: var(--nav-h); padding: var(--sp-1) var(--sp-2);
    overflow-x: auto; scrollbar-width: none; touch-action: pan-x; container-type: inline-size;
  }
  .strip::-webkit-scrollbar { display: none; }
  /* Up to four share the width; a fifth makes it 4½ wide, so the cut one says the strip goes on. */
  .tab { flex: 1 0 calc(100cqw / 4.5); cursor: pointer; }
</style>

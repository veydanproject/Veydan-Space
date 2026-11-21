<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The buttons under a step: a row at the right on a computer. On a phone they go to the foot of the screen (CloudScreen's .page-foot), pinned under the scrolling body, side by side; where there is no such foot, full-width and stacked with the main one on top. The main button is written last. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { isMobile } from '$lib/core/platform';

  let { children, start = false }: { children: Snippet; start?: boolean } = $props();

  let inFoot = $state(false);

  /** A phone: the buttons move to their screen's foot, if it has one. Out of their <form> a submit button no longer
   *  submits it, so a click on one (drawn now or later) asks the form it left to submit, as the button would have. */
  function toFoot(node: HTMLElement) {
    const foot = node.closest('.m-page')?.querySelector<HTMLElement>(':scope > .cloud-foot');
    if (!foot) return;
    const form = node.closest('form');
    const submit = (e: MouseEvent) => {
      const btn = (e.target as Element).closest('button');
      if (!form || !btn || btn.type !== 'submit' || btn.form) return;
      e.preventDefault();
      form.requestSubmit();
    };
    foot.appendChild(node);
    node.addEventListener('click', submit);
    inFoot = true;
    return { destroy: () => { node.removeEventListener('click', submit); node.remove(); } };
  }
</script>

{#if isMobile}
  <div class="actions m" class:in-foot={inFoot} use:toFoot>{@render children()}</div>
{:else}
  <div class="actions" class:start>{@render children()}</div>
{/if}

<style>
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; justify-content: flex-end; align-items: center; }
  .actions.start { justify-content: flex-start; }
  .actions.m { flex-direction: column-reverse; align-items: stretch; flex-wrap: nowrap; }
  .actions.m :global(.btn) { width: 100%; justify-content: center; min-height: 48px; font-size: var(--fs-body); }
  /* In the foot its buttons are the foot's own, as `.page-foot > .btn` (mobile.css): halves side by side. */
  .actions.in-foot { display: contents; }
  .actions.in-foot :global(.btn) { flex: 1 1 0; width: auto; min-width: fit-content; font-size: var(--fs-base); }
</style>

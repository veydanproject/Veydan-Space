<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Where a flow of a few steps stands: the steps by name, the passed ones checked. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';

  let { steps, current }: { steps: string[]; current: number } = $props();
</script>

<ol class="steps" aria-label={steps[current]}>
  {#each steps as step, i (i)}
    <li class:done={i < current} class:now={i === current} aria-current={i === current ? 'step' : undefined}>
      <span class="dot">{#if i < current}<Icon name="check" size={12} strokeWidth={3} />{:else}{i + 1}{/if}</span>
      <span class="name">{step}</span>
    </li>
  {/each}
</ol>

<style>
  .steps { list-style: none; margin: 0; padding: 0; display: flex; align-items: center; gap: var(--sp-2); }
  li { display: flex; align-items: center; gap: var(--sp-2); color: var(--text-3); font-size: var(--fs-sm); font-weight: var(--fw-semibold); min-width: 0; }
  li:not(:last-child) { flex: 1; }
  li:not(:last-child)::after { content: ''; flex: 1; height: 1px; background: var(--border-2); min-width: var(--sp-3); }
  .dot {
    display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 22px; border-radius: 50%; flex-shrink: 0;
    border: 1px solid var(--border-2); background: var(--surface-3); font-size: var(--fs-xs); color: var(--text-2);
  }
  .name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .now { color: var(--text); }
  .now .dot { background: var(--accent-bg); border-color: var(--accent-border); color: var(--accent-text); }
  .done .dot { background: var(--success-bg); border-color: var(--success-border); color: var(--success-text); }
  .done::after { background: var(--success-border); }
</style>

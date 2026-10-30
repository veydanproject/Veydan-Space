<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One choice of a few (how one joins, what kind a group is): a tile of 36,
  the title, a hint and the check of the chosen one. The whole row is the
  `button role="radio" aria-checked`; the caller puts the rows into a
  `role="radiogroup"` with its label and says what a press does.
  `surface`: `dialog` (the default, `--surface-3`) or `panel` (inside a panel
  on `--surface`: `--surface-2`).
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    icon: string;
    title: string;
    hint?: string;
    checked: boolean;
    onclick: () => void;
    disabled?: boolean;
    surface?: 'dialog' | 'panel';
  }
  let { icon, title, hint, checked, onclick, disabled = false, surface = 'dialog' }: Props = $props();
</script>

<button type="button" class="choice {surface}" class:on={checked} role="radio" aria-checked={checked} {disabled} {onclick}>
  <span class="tile"><Icon name={icon} size={18} /></span>
  <span class="words">
    <span class="title">{title}</span>
    {#if hint}<span class="hint">{hint}</span>{/if}
  </span>
  {#if checked}<span class="check"><Icon name="check" size={18} /></span>{/if}
</button>

<style>
  .choice {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 60px; padding: var(--sp-2) var(--sp-3);
    text-align: left; cursor: pointer; font: inherit; color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-3);
    transition: border-color var(--dur-fast), background var(--dur-fast);
  }
  .panel { background: var(--surface-2); }
  .choice:hover:not(.on):not(:disabled) { border-color: var(--border-2); }
  .choice:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .choice.on { border-color: var(--accent-border); background: var(--accent-bg); }
  .tile {
    flex-shrink: 0; width: 36px; height: 36px; border-radius: var(--radius-sm); display: grid; place-items: center;
    background: var(--surface-2); color: var(--text-2);
  }
  .panel .tile { background: var(--surface-3); }
  .on .tile { background: transparent; color: var(--accent-text); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .title { font-size: var(--fs-base); font-weight: var(--fw-bold); }
  .hint { font-size: var(--fs-sm); color: var(--text-2); line-height: 1.4; }
  .check { flex-shrink: 0; display: inline-flex; color: var(--accent-text); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A yes / no setting as a card: its title, a hint, the switch at the right.
  The whole card is the `button role="switch"`; the switch inside is only
  drawn (`.toggle` of base.css, `aria-hidden`). `onclick` does what pressing
  it does; without it the press flips `on` (bindable).
  `surface`: `strong` (the default: a dialog, `--pane-strong`) or `panel`
  (inside a pane or a panel on `--surface`: `--surface-2` with `--border`).
-->
<script lang="ts">
  interface Props {
    title: string;
    hint?: string;
    on?: boolean;
    onclick?: () => void;
    disabled?: boolean;
    /** The accessible name, when it is not the title and the hint together. */
    label?: string;
    surface?: 'strong' | 'panel';
  }
  let { title, hint, on = $bindable(false), onclick, disabled = false, label, surface = 'strong' }: Props = $props();

  function press() {
    if (onclick) onclick();
    else on = !on;
  }
</script>

<button type="button" class="toggle-row {surface}" role="switch" aria-checked={on} aria-label={label} {disabled} onclick={press}>
  <span class="info">
    <span class="title">{title}</span>
    {#if hint}<span class="hint">{hint}</span>{/if}
  </span>
  <span class="toggle" class:on aria-hidden="true"></span>
</button>

<style>
  .toggle-row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px; padding: var(--sp-3) var(--sp-4);
    font: inherit; color: var(--text); text-align: left; cursor: pointer;
    border: 1px solid var(--pane-strong-border); border-radius: var(--radius-lg); background: var(--pane-strong);
  }
  .panel { border-color: var(--border); background: var(--surface-2); }
  .toggle-row:hover:not(:disabled) { border-color: var(--border-2); }
  .toggle-row:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .toggle-row:disabled { cursor: default; opacity: 0.6; }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); font-size: var(--fs-base); line-height: 1.4; }
  .title { font-weight: var(--fw-semibold); }
  .hint { font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
</style>

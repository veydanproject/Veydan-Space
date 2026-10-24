<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A colour among presets or an own one: the label's row may carry an action (the theme's reset);
     each swatch is a 44px target, the chosen one ringed and checked. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    label: string;
    value: string;
    presets: string[];
    onchange: (hex: string) => void;
    action?: Snippet;
  }

  let { label, value, presets, onchange, action }: Props = $props();

  function norm(hex: string) {
    return hex.toLowerCase();
  }

  function isPreset(hex: string) {
    return presets.some((p) => norm(p) === norm(hex));
  }
</script>

<div class="color-field">
  <div class="color-field-head">
    <span class="color-field-label">{label}</span>
    {@render action?.()}
  </div>
  <div class="color-picker" role="group" aria-label={label}>
    {#each presets as c}
      {@const on = norm(value) === norm(c)}
      <button
        type="button"
        class="color-swatch"
        class:selected={on}
        aria-label={c}
        aria-pressed={on}
        onclick={() => onchange(c)}
      >
        <span class="dot" style="background: {c}">{#if on}<Icon name="check" size={16} />{/if}</span>
      </button>
    {/each}
    <label class="color-swatch" class:selected={!isPreset(value)}>
      <input
        type="color"
        value={value}
        aria-label={label}
        oninput={(e) => onchange(e.currentTarget.value)}
      />
      {#if isPreset(value)}
        <span class="dot dot-custom"></span>
      {:else}
        <span class="dot" style="background: {value}"><Icon name="check" size={16} /></span>
      {/if}
    </label>
  </div>
</div>

<style>
  .color-field {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .color-field-head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 32px;
  }
  .color-field-label {
    flex: 1;
    font-size: var(--fs-md);
    color: var(--text);
    font-weight: var(--fw-semibold);
  }
  .color-picker {
    display: flex;
    gap: var(--sp-1);
    flex-wrap: wrap;
    align-items: center;
    /* The 44px targets stand out of the row: the dots line up with the label. */
    margin-left: -7px;
  }
  .color-swatch {
    position: relative;
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: none;
    padding: 0;
    cursor: pointer;
    flex-shrink: 0;
  }
  .dot {
    width: 30px;
    height: 30px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: #fff;
    /* A ring of its own: a preset of the card's colour is still a visible circle. */
    box-shadow: inset 0 0 0 1px var(--border-2);
    transition: transform var(--dur-fast), box-shadow var(--dur-fast);
    pointer-events: none;
  }
  /* The white check stays legible on a light swatch: the knob's shadow of the theme. */
  .dot :global(svg) { filter: drop-shadow(var(--shadow-knob)); }
  .color-swatch:hover .dot { transform: scale(1.08); }
  .color-swatch.selected .dot { outline: 2px solid var(--text); outline-offset: 2px; }
  .color-swatch:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .color-swatch:has(input:focus-visible) { outline: 2px solid var(--accent-text); outline-offset: -2px; }

  .dot-custom {
    background: conic-gradient(
      #f43f5e, #f97316, #eab308, #22c55e, #06b6d4, #6366f1, #ec4899, #f43f5e
    );
  }
  .color-swatch input[type="color"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
    border: none;
    padding: 0;
    margin: 0;
  }
</style>

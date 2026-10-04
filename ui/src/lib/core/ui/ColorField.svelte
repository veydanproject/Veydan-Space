<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  interface Props {
    label: string;
    value: string;
    presets: string[];
    onchange: (hex: string) => void;
  }

  let { label, value, presets, onchange }: Props = $props();

  function norm(hex: string) {
    return hex.toLowerCase();
  }

  function isPreset(hex: string) {
    return presets.some((p) => norm(p) === norm(hex));
  }
</script>

<div class="color-field">
  <span class="color-field-label">{label}</span>
  <div class="color-picker" role="group" aria-label={label}>
    {#each presets as c}
      <button
        type="button"
        class="color-swatch"
        class:selected={norm(value) === norm(c)}
        style="background: {c}"
        aria-label={c}
        aria-pressed={norm(value) === norm(c)}
        onclick={() => onchange(c)}
      ></button>
    {/each}
    <label class="color-swatch color-swatch-custom" class:selected={!isPreset(value)}>
      <input
        type="color"
        value={value}
        aria-label={label}
        oninput={(e) => onchange(e.currentTarget.value)}
      />
      {#if !isPreset(value)}
        <span class="custom-dot" style="background:{value}"></span>
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
  .color-field-label {
    font-size: var(--fs-sm);
    color: var(--text);
    font-weight: var(--fw-semibold);
  }
  .color-picker {
    display: flex;
    gap: var(--sp-2);
    flex-wrap: wrap;
    align-items: center;
  }
  .color-swatch {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid transparent;
    /* A ring of its own: a preset of the card's colour is still a visible circle. */
    box-shadow: inset 0 0 0 1px var(--border-2);
    cursor: pointer;
    transition: transform var(--dur-fast), border-color var(--dur-fast);
    padding: 0;
    flex-shrink: 0;
  }
  .color-swatch:hover { transform: scale(1.15); }
  .color-swatch.selected { border-color: var(--text); transform: scale(1.15); }

  .color-swatch-custom {
    position: relative;
    background: conic-gradient(
      #f43f5e, #f97316, #eab308, #22c55e, #06b6d4, #6366f1, #ec4899, #f43f5e
    );
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .color-swatch-custom input[type="color"] {
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
  .color-swatch-custom .custom-dot {
    position: absolute;
    inset: 3px;
    border-radius: 50%;
    border: 1.5px solid rgba(255, 255, 255, 0.6);
    pointer-events: none;
  }
</style>

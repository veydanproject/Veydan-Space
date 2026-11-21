<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The one colour picker of what the user colours (a tag, a folder, a workspace, a smart view,
     a column, a 2FA label): the seven of PALETTE and «Custom color» on --hue-wheel. A cell is 38
     (44 under a finger), its dot 30; the chosen one is ringed and checked (design 10). -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { PALETTE, PALETTE_NAMES } from '$lib/core/palette';

  interface Props {
    value?: string;
    id?: string;
    /** The name of the group for a screen reader. */
    label?: string;
    onchange?: (hex: string) => void;
  }

  let { value = $bindable(), id, label, onchange }: Props = $props();

  const norm = (hex: string) => hex.toLowerCase();
  const own = $derived(!PALETTE.some((p) => norm(p) === norm(value ?? '')));

  function pick(hex: string) {
    value = hex;
    onchange?.(hex);
  }
</script>

<div class="color-swatches" {id} role="group" aria-label={label}>
  {#each PALETTE as c, i (c)}
    {@const on = norm(value ?? '') === norm(c)}
    <button
      type="button"
      class="color-swatch"
      class:selected={on}
      aria-label={$t(`kanban_color_${PALETTE_NAMES[i]}`)}
      aria-pressed={on}
      onclick={() => pick(c)}
    >
      <span class="dot" style:background={c}>{#if on}<Icon name="check" size={16} />{/if}</span>
    </button>
  {/each}
  <label class="color-swatch" class:selected={own} title={$t('workspaces_color_custom')}>
    <input
      type="color"
      value={own ? value : PALETTE[0]}
      aria-label={$t('workspaces_color_custom')}
      oninput={(e) => pick(e.currentTarget.value)}
    />
    {#if own}
      <span class="dot" style:background={value}><Icon name="check" size={16} /></span>
    {:else}
      <span class="dot dot-custom"></span>
    {/if}
  </label>
</div>

<style>
  .color-swatches {
    --swatch-cell: 38px;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    /* The cells stand out of the row only up to the chosen one's ring (the dot's 30 and 4 around
       it): the ring meets the content's edge and is never cut by a sheet that clips (Q2 ROO-1441),
       and eight cells of 44 still fit a phone's sheet. */
    margin-inline: calc((38px - var(--swatch-cell)) / 2);
  }
  @media (pointer: coarse) {
    .color-swatches { --swatch-cell: 44px; }
  }
  .color-swatch {
    position: relative;
    width: var(--swatch-cell);
    height: var(--swatch-cell);
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
    /* A ring of its own: a colour close to the ground is still a visible circle. */
    box-shadow: inset 0 0 0 1px var(--border-2);
    transition: transform var(--dur-fast);
    pointer-events: none;
  }
  /* The white check stays legible on a light colour: the knob's shadow of the theme. */
  .dot :global(svg) { filter: drop-shadow(var(--shadow-knob)); }
  .color-swatch:hover .dot { transform: scale(1.08); }
  .color-swatch.selected .dot { outline: 2px solid var(--text); outline-offset: 2px; }
  .color-swatch:focus-visible,
  .color-swatch:has(input:focus-visible) { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .dot-custom { background: var(--hue-wheel); }
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

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';

  export interface SelectOption {
    label: string;
    value: string | null | undefined;
    disabled?: boolean;
  }

  interface Props {
    options: SelectOption[];
    value: string | null | undefined;
    placeholder?: string;
    id?: string;
    onchange?: (value: string | null | undefined) => void;
  }

  let { options, value = $bindable(), placeholder = '—', id = '', onchange }: Props = $props();

  let open = $state(false);
  let triggerEl = $state<HTMLButtonElement | null>(null);
  let dropdownEl = $state<HTMLDivElement | null>(null);
  let dropdownStyle = $state('');

  let selectedLabel = $derived(
    options.find(o => o.value === value)?.label ?? placeholder
  );

  function toggle() {
    if (open) { open = false; return; }
    positionDropdown();
    open = true;
  }

  function select(opt: SelectOption) {
    if (opt.disabled) return;
    value = opt.value;
    onchange?.(opt.value);
    open = false;
  }

  function positionDropdown() {
    if (!triggerEl) return;
    const rect = triggerEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const dropH = Math.min(options.length * 32 + 8, 280);
    const dropW = rect.width;

    let top = rect.bottom + 4;
    let left = rect.left;

    // Clamp right edge
    if (left + dropW > vw - 4) left = vw - dropW - 4;
    if (left < 4) left = 4;

    // Open upward if not enough space below
    if (top + dropH > vh - 4) top = rect.top - dropH - 4;
    if (top < 4) top = 4;

    dropdownStyle = `top:${top}px;left:${left}px;width:${dropW}px;max-height:${Math.min(dropH, vh - top - 4)}px`;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') open = false;
  }

  function onOutsideClick(e: MouseEvent) {
    const t = e.target as Node;
    if (triggerEl && !triggerEl.contains(t) && dropdownEl && !dropdownEl.contains(t)) {
      open = false;
    }
  }

  onMount(() => {
    document.addEventListener('mousedown', onOutsideClick, true);
    document.addEventListener('keydown', onKeydown);
  });
  onDestroy(() => {
    document.removeEventListener('mousedown', onOutsideClick, true);
    document.removeEventListener('keydown', onKeydown);
  });
</script>

<button
  bind:this={triggerEl}
  type="button"
  {id}
  class="trigger"
  class:open
  onclick={toggle}
  aria-haspopup="listbox"
  aria-expanded={open}
>
  <span class="trigger-label">{selectedLabel}</span>
  <svg class="chevron" class:rotated={open} width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>
</button>

{#if open}
  <div
    bind:this={dropdownEl}
    class="dropdown"
    style={dropdownStyle}
    role="listbox"
  >
    {#each options as opt}
      <button
        type="button"
        class="option"
        class:selected={opt.value === value}
        class:separator={opt.disabled}
        role="option"
        aria-selected={opt.value === value}
        aria-disabled={opt.disabled}
        onclick={() => select(opt)}
      >
        {opt.label}
      </button>
    {/each}
  </div>
{/if}

<style>
  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: var(--control-h-lg);
    padding: var(--sp-2) var(--sp-3);
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    color: var(--text);
    font-size: var(--fs-base);
    font-family: inherit;
    cursor: pointer;
    text-align: left;
    gap: var(--sp-2);
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .trigger:focus, .trigger.open {
    outline: none;
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--accent-bg);
  }
  .trigger-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .chevron {
    flex-shrink: 0;
    color: var(--text-2);
    transition: transform 0.15s;
  }
  .chevron.rotated { transform: rotate(180deg); }

  .dropdown {
    position: fixed;
    z-index: var(--z-popover);
    background: var(--surface-drawer);
    border: 1px solid var(--border-2);
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
    overflow-y: auto;
    padding: 4px;
  }
  .dropdown::-webkit-scrollbar { width: 4px; }
  .dropdown::-webkit-scrollbar-track { background: transparent; }
  .dropdown::-webkit-scrollbar-thumb { background: var(--border-2); border-radius: 2px; }

  .option {
    display: block;
    width: 100%;
    padding: 0.45rem 0.65rem;
    border-radius: var(--radius-sm);
    background: transparent;
    border: none;
    color: var(--text);
    font-size: var(--fs-base);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: background 0.1s;
  }
  .option:hover { background: var(--surface-2); }
  .option.selected { background: var(--accent-bg); color: var(--accent-text); }
  .option.separator {
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: default;
    padding: 0.2rem 0.65rem;
  }
  .option.separator:hover { background: transparent; }
</style>

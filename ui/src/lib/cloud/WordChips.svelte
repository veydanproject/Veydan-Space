<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Words a person reads aloud or types on another device: numbered when their order matters, large when they are compared by eye. -->
<script lang="ts">
  let { words, numbered = false, large = false }: { words: string[]; numbered?: boolean; large?: boolean } = $props();
</script>

<ol class="words mono" class:large class:numbered>
  {#each words as word, i (i)}
    <li>{#if numbered}<span class="num">{i + 1}</span>{/if}{word}</li>
  {/each}
</ol>

<style>
  .words { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap: var(--sp-2); }
  li {
    display: flex; align-items: baseline; gap: var(--sp-2); padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-sm); background: var(--surface-3); border: 1px solid var(--border);
    font-size: var(--fs-base); color: var(--text); user-select: text; -webkit-user-select: text;
  }
  .num { color: var(--text-3); font-size: var(--fs-xs); min-width: 1.1em; text-align: right; user-select: none; -webkit-user-select: none; }
  .large { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .large li { justify-content: center; font-size: var(--fs-lg); font-weight: var(--fw-bold); padding: var(--sp-3); background: var(--accent-tint); border-color: var(--accent-tint-border); }
  @media (min-width: 520px) { .large { grid-template-columns: repeat(4, minmax(0, 1fr)); } }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The search of a list, opened by the lens of its head: the focus on its showing; Esc or the cross clears the
     words, in an empty field closes it. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  let { value = $bindable(''), placeholder, onclose }: { value?: string; placeholder: string; onclose: () => void } = $props();

  function clear() {
    if (value) value = '';
    else onclose();
  }
</script>

<div class="search">
  <Icon name="search" size={16} />
  <input type="text" bind:value {placeholder} spellcheck="false" aria-label={placeholder}
    onkeydown={(e) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); clear(); } }}
    {@attach (el: HTMLInputElement) => el.focus()} />
  <button class="clear" onclick={clear} title={$t('common_clear')} aria-label={$t('common_clear')}><Icon name="x" size={12} /></button>
</div>

<style>
  .search {
    display: flex; align-items: center; gap: 8px; min-height: 40px; margin: 0 var(--sp-3) var(--sp-2); padding: 0 var(--sp-4); flex-shrink: 0;
    background: var(--surface-3); border: 1px solid transparent; border-radius: var(--radius-pill); color: var(--text-3);
  }
  .search:focus-within { border-color: var(--accent-border); }
  /* The pill is the field: its border is the one focus mark. */
  .search input { flex: 1; min-width: 0; border: none; border-radius: 0; background: none; outline: none; box-shadow: none; font: inherit; font-size: var(--fs-base); color: var(--text); padding: 7px 0; }
  .clear {
    border: none; background: none; color: var(--text-soft); cursor: pointer; display: inline-flex; align-items: center; justify-content: center;
    width: 24px; height: 24px; padding: 0; border-radius: var(--radius);
  }
  .clear:hover { color: var(--text); background: var(--surface-hover); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A person of the cloud (design 09): the first letter of the name, white on the accent's gradient, in a circle of
     36 (a list), 48 (the account's row) or 56 (the account's head). `ghost` is the quiet one in the accent's tint;
     without a name it shows the person's icon (no account yet). -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';

  let { name = '', size = 36, ghost = false }: { name?: string; size?: 36 | 48 | 56; ghost?: boolean } = $props();

  const letter = $derived([...name.trim()][0]?.toUpperCase() ?? '');
</script>

<span class="avatar" class:ghost class:big={size > 36} style:--ava={`${size}px`} aria-hidden="true">
  {#if letter}{letter}{:else}<Icon name="user" size={size > 36 ? 22 : 18} />{/if}
</span>

<style>
  .avatar {
    display: inline-flex; align-items: center; justify-content: center; width: var(--ava); height: var(--ava); border-radius: 50%; flex-shrink: 0;
    background: var(--accent-grad); color: #fff; font-size: var(--fs-base); font-weight: var(--fw-extrabold);
  }
  .big { font-size: var(--fs-xl); }
  .ghost { background: var(--accent-tint); color: var(--accent-text); }
</style>

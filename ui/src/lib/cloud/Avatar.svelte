<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A person in a list of the cloud: the first letter of the name on a round ground. -->
<script lang="ts">
  let { name }: { name: string } = $props();

  const letter = $derived(([...name.trim()][0] ?? '?').toUpperCase());
</script>

<span class="avatar" aria-hidden="true">{letter}</span>

<style>
  .avatar {
    display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; border-radius: 50%; flex-shrink: 0;
    background: var(--accent-tint); border: 1px solid var(--accent-tint-border); color: var(--accent-text);
    font-size: var(--fs-base); font-weight: var(--fw-bold);
  }
</style>

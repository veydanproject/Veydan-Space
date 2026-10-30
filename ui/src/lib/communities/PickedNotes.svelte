<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What the tray of a post, a comment or the editor of a post tells of its
  files, as a chat's composer does: a phone's photo its decoder could not
  read goes as a file (`unsupported_format`), said by its name.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { Picked } from './picked.svelte';

  let { picked }: { picked: Picked } = $props();
</script>

{#each picked.unsupported as name, i (i)}
  <p class="picked-note" role="status">{name}: {$t('msg_attach_as_file')}</p>
{/each}

<style>
  .picked-note { margin: 0; font-size: var(--fs-xs); color: var(--text-2); overflow-wrap: anywhere; }
</style>

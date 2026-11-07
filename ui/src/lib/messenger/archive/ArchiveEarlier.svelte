<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Under the oldest restored message of an open chat, while the older months of this chat's scope come back from the archive (messenger-archive-plan.md section 5). The row of "loading" the chat already has, with the same icon. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { archiveStore } from './archiveStore.svelte';
  import { scopeOfChat } from './archive';

  let { chatId }: { chatId: string } = $props();
  const shown = $derived(archiveStore.isRestoring(scopeOfChat(chatId)));
</script>

{#if shown}
  <div class="earlier-restoring" role="status"><Icon name="loader" size={14} />{$t('msg_archive_earlier')}</div>
{/if}

<style>
  .earlier-restoring { display: flex; align-items: center; justify-content: center; gap: var(--sp-2); padding: var(--sp-2); font-size: var(--fs-sm); color: var(--text-3); }
</style>

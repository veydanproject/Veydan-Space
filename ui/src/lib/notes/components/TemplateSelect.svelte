<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import { templateNotes } from '$lib/notes/filter';
  import { t } from '$lib/core/i18n';

  interface Props {
    /** Selected template note id, empty for none */
    value: string;
    class?: string;
    onchange?: () => void;
  }

  let { value = $bindable(''), class: cls = '', onchange }: Props = $props();

  const templates = $derived(templateNotes(notesStore.list, notesStore.folders));
</script>

<select class={cls} bind:value {onchange} title={$t('note_template_label')}>
  <option value="">{$t('note_template_none')}</option>
  {#each templates as n (n.id)}
    <option value={n.id}>{n.title}</option>
  {/each}
</select>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import ColorSwatches from '$lib/core/ui/ColorSwatches.svelte';

  interface Props {
    open: boolean;
    title: string;
    name: string;
    color: string;
    placeholder: string;
    saving?: boolean;
    onclose: () => void;
    onsave: (name: string, color: string) => void;
  }

  let { open, title, name = $bindable(), color = $bindable(), placeholder, saving = false, onclose, onsave }: Props = $props();
</script>

<BottomSheet {open} {title} {onclose}>
  <input type="text" bind:value={name} {placeholder} onkeydown={(e) => { if (e.key === 'Enter' && name.trim()) onsave(name.trim(), color); }} />
  <ColorSwatches bind:value={color} label={title} />
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={!name.trim() || saving} onclick={() => onsave(name.trim(), color)}>
      {$t('common_save')}
    </button>
  {/snippet}
</BottomSheet>

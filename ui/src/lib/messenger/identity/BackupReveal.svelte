<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import CopyField from './CopyField.svelte';

  interface Props {
    npub: string;
    ncryptsec: string;
    onDone: () => void;
  }
  let { npub, ncryptsec, onDone }: Props = $props();

  let confirmed = $state(false);
</script>

<div class="reveal">
  <div class="card-title"><Icon name="key" size={16} /> {$t('msg_id_backup_title')}</div>
  <p class="warn">{$t('msg_id_backup_warning')}</p>
  <CopyField label={$t('msg_id_npub')} value={npub} />
  <CopyField label={$t('msg_id_ncryptsec')} value={ncryptsec} mono />
  <label class="confirm">
    <input type="checkbox" bind:checked={confirmed} />
    <span>{$t('msg_id_backup_confirm')}</span>
  </label>
  <div class="actions">
    <button class="btn btn-primary" disabled={!confirmed} onclick={onDone}>{$t('msg_id_backup_done')}</button>
  </div>
</div>

<style>
  .reveal { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .warn {
    margin: 0; font-size: var(--fs-sm); color: var(--text-body);
    background: color-mix(in srgb, var(--color-warning) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-warning) 40%, transparent);
    border-radius: var(--radius-sm); padding: var(--sp-2) var(--sp-3);
  }
  .confirm { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-body); cursor: pointer; }
  .confirm input { margin-top: 3px; }
  .actions { display: flex; justify-content: flex-end; }
</style>

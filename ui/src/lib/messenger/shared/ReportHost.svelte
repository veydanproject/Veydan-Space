<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The form of a report (reportStore.open). Mount once per page, beside
  ConfirmHost; the dialog is a sheet on the phone by itself.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { REPORT_COMMENT_MAX, REPORT_REASONS, REPORT_SUBS, reportStore as s } from './report.svelte';

  let open = $derived(s.target !== null);
</script>

<Dialog {open} title={s.target?.target === 'group' ? $t('msg_report_group_title', { name: s.target.name }) : $t('msg_report_title')} width="min(440px, calc(100vw - 24px))" onclose={() => s.close()}>
  {#if s.target}
    <div class="body">
      <div class="list" role="radiogroup" aria-label={$t('msg_report_title')}>
        {#each REPORT_REASONS as r (r)}
          <label class="radio">
            <input type="radio" name="report-reason" value={r} checked={s.reason === r} disabled={s.busy} onchange={() => (s.reason = r)} />
            <span>{$t(`msg_report_reason_${r}` as 'msg_report_reason_spam')}</span>
          </label>
        {/each}
      </div>
      {#if s.reason === 'child_harm'}
        <div class="list sub" role="radiogroup" aria-label={$t('msg_report_reason_child_harm')}>
          {#each REPORT_SUBS as k (k)}
            <label class="radio">
              <input type="radio" name="report-sub" value={k} checked={s.sub === k} disabled={s.busy} onchange={() => (s.sub = k)} />
              <span>{$t(`msg_report_sub_${k}` as 'msg_report_sub_sexual')}</span>
            </label>
          {/each}
        </div>
      {/if}

      <label class="field">
        <span class="label">{$t('msg_report_comment')}</span>
        <textarea rows="3" maxlength={REPORT_COMMENT_MAX} bind:value={s.comment} disabled={s.busy}></textarea>
        <span class="count">{s.comment.length} / {REPORT_COMMENT_MAX}</span>
      </label>

      <label class="check">
        <input type="checkbox" bind:checked={s.attach} disabled={s.busy} />
        <span>{$t(s.target.target === 'group' ? 'msg_report_group_attach' : 'msg_report_attach')}</span>
      </label>
      {#if s.target.canBlock}
        <label class="check">
          <input type="checkbox" bind:checked={s.block} disabled={s.busy} />
          <span>{$t('msg_report_block', { name: s.target.name })}</span>
        </label>
      {/if}

      {#if s.target.target === 'group'}<p class="note">{$t('msg_report_group_note')}</p>{/if}
      <p class="note">{$t('msg_report_note')}</p>
    </div>
  {/if}
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={s.busy} onclick={() => s.close()}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-danger" disabled={!s.ready} onclick={() => s.send()}>
      {#if s.busy}<span class="spinner" aria-hidden="true"></span>{/if}{$t('msg_report_send')}
    </button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .list { display: flex; flex-direction: column; gap: 2px; }
  .list.sub { padding-inline-start: var(--sp-5); border-inline-start: 2px solid var(--border); margin-inline-start: 7px; }
  .radio, .check { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-body); cursor: pointer; user-select: none; padding-block: 4px; }
  .check { align-items: flex-start; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .label { font-size: var(--fs-xs); color: var(--text-3); }
  textarea { resize: vertical; min-height: 64px; font: inherit; font-size: var(--fs-sm); color: var(--text-body); background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px; }
  .count { align-self: flex-end; font-size: var(--fs-2xs); color: var(--text-3); }
  .note { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  @media (pointer: coarse) { .radio, .check { min-height: 40px; } }
</style>

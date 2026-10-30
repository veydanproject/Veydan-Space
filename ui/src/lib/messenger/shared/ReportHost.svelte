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

  /** What the report is on, in the title: a person, a group, a channel, or a message of a channel. */
  const title = $derived.by(() => {
    const tg = s.target;
    if (!tg) return '';
    if (tg.target === 'group') return $t('msg_report_group_title', { name: tg.name });
    if (tg.target === 'channel') return $t('msg_report_channel_title', { name: tg.name });
    if (tg.context.t === 'channel' && tg.context.post_id) return tg.item === 'comment' ? $t('msg_report_comment_title') : $t('msg_report_post_title');
    return $t('msg_report_title');
  });

  /** Nothing of a chat goes with a report from a channel: the note says what does. */
  const channelNote = $derived.by(() => {
    const tg = s.target;
    if (!tg || tg.context.t !== 'channel') return null;
    if (tg.target === 'channel') return $t('msg_report_channel_note');
    if (!tg.context.post_id) return $t('msg_report_channel_person_note');
    return tg.item === 'comment' ? $t('msg_report_comment_note') : $t('msg_report_post_note');
  });
</script>

<Dialog {open} {title} width="min(440px, calc(100vw - 24px))" onclose={() => s.close()}>
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

      <!-- A channel's post is named by the report itself: there are no chat messages to attach. -->
      {#if s.target.context.t !== 'channel' || s.target.canBlock}
      <div class="checks">
      {#if s.target.context.t !== 'channel'}
        <label class="check">
          <input type="checkbox" bind:checked={s.attach} disabled={s.busy} />
          <span>{$t(s.target.target === 'group' ? 'msg_report_group_attach' : 'msg_report_attach')}</span>
        </label>
      {/if}
      {#if s.target.canBlock}
        <label class="check">
          <input type="checkbox" bind:checked={s.block} disabled={s.busy} />
          <span>{$t('msg_report_block', { name: s.target.name })}</span>
        </label>
      {/if}
      </div>
      {/if}

      {#if s.target.target === 'group'}<p class="note">{$t('msg_report_group_note')}</p>{/if}
      <p class="note">{channelNote ?? $t('msg_report_note')}</p>
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
  /* The reasons: one card of rows (design book, «Пожаловаться»). */
  .list {
    display: flex; flex-direction: column; padding: var(--sp-1);
    border: 1px solid var(--border); border-radius: var(--radius-lg); background: var(--surface-2);
  }
  .list.sub { margin-top: calc(-1 * var(--sp-2)); margin-inline-start: var(--sp-6); }
  .radio, .check { display: flex; gap: var(--sp-3); font-size: var(--fs-base); color: var(--text-body); cursor: pointer; user-select: none; }
  .radio { align-items: center; min-height: 40px; padding: 0 var(--sp-3); border-radius: var(--radius); }
  .radio:hover { background: var(--surface-hover); color: var(--text); }
  .radio:has(input:checked) { color: var(--text); }
  .radio:has(input:focus-visible) { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .radio input:focus-visible { outline: none; }
  .checks { display: flex; flex-direction: column; gap: var(--sp-3); }
  .check { align-items: flex-start; line-height: 1.45; }
  .check input { margin-top: 2px; }
  .field { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); }
  textarea {
    resize: vertical; min-height: 80px; font: inherit; font-size: var(--fs-base); line-height: 1.45; color: var(--text);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--sp-3);
  }
  textarea:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 0; }
  .count { align-self: flex-end; margin-top: calc(-1 * var(--sp-1)); font-size: var(--fs-xs); color: var(--text-3); font-variant-numeric: tabular-nums; }
  .note { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  @media (pointer: coarse) { .radio { min-height: 44px; } .check { min-height: 40px; } }
</style>

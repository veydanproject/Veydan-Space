<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A reader asks for a level (communities-wire §14.4): the levels that give
  more than theirs do, each with what it would open and let them do, and
  a note to the moderators. Sent, the request is said as such until the
  channel changes; a level given opens its sections in the feed.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import ChoiceRow from '$lib/core/ui/ChoiceRow.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import type { ChannelDetail } from '../api';
  import { levelsOf } from './sections';
  import { adds, requestable, waitingLevels } from './request';

  interface Props {
    open: boolean;
    detail: ChannelDetail;
    /** The level offered first (a locked section's); none, the first. */
    preselect?: number | null;
  }
  let { open = $bindable(), detail, preselect = null }: Props = $props();

  const NOTE_MAX = 500;
  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const lv = $derived(levelsOf(detail));
  const options = $derived(requestable(lv.levels, lv.my_levels, lv.scopes));
  const asked = $derived(waitingLevels(channelStore.levelAsked, cid, detail.my_level_requests));

  let picked = $state<number | null>(null);
  let note = $state('');
  let busy = $state(false);
  let error = $state('');

  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      const free = options.filter((l) => !asked.includes(l.id));
      picked = free.find((l) => l.id === preselect)?.id ?? free[0]?.id ?? null;
      note = ''; error = ''; busy = false;
    }
    wasOpen = open;
  });

  async function send() {
    const level = options.find((l) => l.id === picked);
    if (!level) return;
    busy = true; error = '';
    try {
      await channelStore.askLevel(cid, level.id, note.trim() || undefined);
      // «My levels» says it, waiting for the answer: no toast besides.
      open = false;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title={$t('chan_level_request_title')} width="min(460px, calc(100vw - 24px))">
  <div class="body">
    <p class="lead">{$t('chan_levels_explain')}</p>
    <!-- One value of a few: ChoiceRow, as the channel's join rule; a level asked for already is out of the choice. -->
    <div class="options" role="radiogroup" aria-label={$t('chan_level')}>
      {#each options as l (l.id)}
        {@const a = adds(l, lv.levels, lv.my_levels, lv.scopes)}
        {@const sent = asked.includes(l.id)}
        <ChoiceRow icon="layers" title={l.name} checked={picked === l.id} disabled={busy || sent} onclick={() => (picked = l.id)}>
          {#snippet more()}
            {#if sent}<span class="badge badge-ok">{$t('chan_level_request_sent')}</span>{/if}
            {#if a.scopes.length}<span class="sub">{$t('chan_level_request_adds', { list: a.scopes.map((s) => `«${s.title}»`).join(', ') })}</span>{/if}
            {#if a.rights.length}<span class="sub">{$t('chan_level_request_rights', { list: a.rights.map((r) => $t(`chan_right_${r}_short` as 'chan_right_comment_short')).join(', ') })}</span>{/if}
          {/snippet}
        </ChoiceRow>
      {/each}
    </div>
    <label class="field-wrap">
      <span class="form-label">{$t('chan_level_request_note')}</span>
      <textarea class="field" rows="3" bind:value={note} maxlength={NOTE_MAX} disabled={busy}></textarea>
    </label>
    <p class="hint">{$t('chan_level_request_hint')}</p>
    {#if error}<Notice tone="danger">{error}</Notice>{/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy || picked === null} onclick={send}><Icon name="send" size={18} />{$t('chan_level_request_send')}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-4); }
  .lead { margin: 0; font-size: var(--fs-base); color: var(--text-2); line-height: var(--lh-base); }
  .hint { margin: calc(-1 * var(--sp-2)) 0 0; line-height: var(--lh-snug); }
  .options { display: flex; flex-direction: column; gap: var(--sp-2); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); line-height: var(--lh-snug); }
  .badge { align-self: flex-start; }
  .field-wrap { display: flex; flex-direction: column; gap: var(--sp-2); }
  .field {
    width: 100%; box-sizing: border-box; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: var(--sp-3); resize: vertical;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  @media (pointer: coarse) { .field { font-size: var(--fs-md); } }
</style>

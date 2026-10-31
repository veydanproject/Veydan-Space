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
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
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
      open = false;
      toast.success($t('chan_level_request_sent_text', { name: level.name }));
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
    <ul class="options" role="radiogroup" aria-label={$t('chan_level')}>
      {#each options as l (l.id)}
        {@const a = adds(l, lv.levels, lv.my_levels, lv.scopes)}
        {@const sent = asked.includes(l.id)}
        <li>
          <label class="option" class:active={picked === l.id} class:sent>
            <input type="radio" name="level" value={l.id} checked={picked === l.id} disabled={busy || sent} onchange={() => (picked = l.id)} />
            <span class="text">
              <span class="name"><Icon name="layers" size={14} />{l.name}{#if sent}<span class="tag">{$t('chan_level_request_sent')}</span>{/if}</span>
              {#if a.scopes.length}<span class="sub">{$t('chan_level_request_adds', { list: a.scopes.map((s) => `«${s.title}»`).join(', ') })}</span>{/if}
              {#if a.rights.length}<span class="sub">{$t('chan_level_request_rights', { list: a.rights.map((r) => $t(`chan_right_${r}_short` as 'chan_right_comment_short')).join(', ') })}</span>{/if}
            </span>
          </label>
        </li>
      {/each}
    </ul>
    <label class="field-wrap">
      <span class="label">{$t('chan_level_request_note')}</span>
      <textarea class="field" rows="3" bind:value={note} maxlength={NOTE_MAX} disabled={busy}></textarea>
    </label>
    <p class="hint">{$t('chan_level_request_hint')}</p>
    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('chan_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy || picked === null} onclick={send}><Icon name="send" size={18} />{$t('chan_level_request_send')}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-4); }
  .lead { margin: 0; font-size: var(--fs-base); color: var(--text-2); line-height: 1.5; }
  .hint { margin: calc(-1 * var(--sp-2)) 0 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  /* The levels: rows of a choice, the chosen one in the accent. */
  .options { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .option {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3); cursor: pointer; user-select: none;
    border: 1px solid var(--border); border-radius: var(--radius-lg); background: var(--surface-2);
    transition: background var(--dur-fast), border-color var(--dur-fast);
  }
  @media (hover: hover) { .option:hover:not(.active):not(.sent) { background: var(--surface-hover); } }
  .option:has(input:focus-visible) { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .option.active { border-color: var(--accent-border); background: var(--accent-bg); }
  .option.sent { cursor: default; opacity: 0.75; }
  .option input { margin-top: 2px; flex-shrink: 0; }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { display: flex; align-items: center; gap: var(--sp-1); font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); overflow-wrap: anywhere; }
  .sub { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; }
  .tag { font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 0 var(--sp-2); border-radius: var(--radius-pill); background: var(--success-bg); color: var(--success-text, var(--success)); }
  .field-wrap { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); }
  .field {
    width: 100%; box-sizing: border-box; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: var(--sp-3); resize: vertical;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  @media (pointer: coarse) { .field { font-size: var(--fs-md); } }
</style>

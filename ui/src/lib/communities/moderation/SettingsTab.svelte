<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/SettingsTab.svelte at 8ce09c39, then changed for channels. -->

<!--
  Who posts in a channel and when, whether readers comment, and whose
  posts and comments wait for a moderator. A manager's: the change is an
  operation of the channel's journal, which every device applies the same
  way.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t, locale, localeTag } from '$lib/core/i18n';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { moderationError } from '../errors';
  import { channelsApi, type ChannelDetail, type ModSettings } from '../api';
  import { DAYS, deviceZone, scheduleValid, validTime, weekdayNames, zoneList } from './schedule';

  type Premod = 'off' | 'newcomers' | 'all';

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const PREMODS: Premod[] = ['off', 'newcomers', 'all'];

  // What the panel showed is where the form starts; the runtime's answer replaces it.
  const start = untrack(() => detail);
  let comments = $state(start.comments !== 'off');
  let premod = $state<Premod>(start.premod === 'all' || start.premod === 'newcomers' ? start.premod : 'off');
  let scheduled = $state(!!start.schedule);
  let days = $state<number[]>(start.schedule?.days ?? [1, 2, 3, 4, 5]);
  let from = $state(start.schedule?.from ?? '09:00');
  let to = $state(start.schedule?.to ?? '21:00');
  let tz = $state(start.schedule?.tz ?? deviceZone());
  let busy = $state(false);
  let error = $state('');
  let saved = $state(false);

  function fill(s: ModSettings) {
    comments = s.comments !== 'off';
    premod = s.premod === 'all' || s.premod === 'newcomers' ? s.premod : 'off';
    scheduled = !!s.schedule?.days.length;
    if (s.schedule?.days.length) { days = [...s.schedule.days]; from = s.schedule.from; to = s.schedule.to; tz = s.schedule.tz; }
  }

  $effect(() => {
    const id = detail.channel.id;
    untrack(() => {
      channelsApi.moderation.settings(id).then(fill).catch((e) => (error = moderationError(e, tr)));
    });
  });

  const names = $derived(weekdayNames(localeTag($locale)));
  const zones = $derived(zoneList(deviceZone(), tz).map((z) => ({ value: z, label: z.replaceAll('_', ' ') })));
  // No days takes the schedule away (ModSettings.schedule).
  const draft = $derived<ModSettings>({
    comments: comments ? 'on' : 'off', premod,
    schedule: scheduled ? { tz, days: [...days].sort((a, b) => a - b), from, to } : { tz, days: [], from, to },
  });
  const valid = $derived(!scheduled || !draft.schedule || scheduleValid(draft.schedule));

  function toggleDay(d: number) {
    days = days.includes(d) ? days.filter((x) => x !== d) : [...days, d];
    saved = false;
  }

  async function save() {
    if (!valid) return;
    error = ''; saved = false; busy = true;
    try {
      await channelsApi.moderation.settingsSet(detail.channel.id, draft);
      saved = true;
      await channelStore.refresh(detail.channel.id);
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = false;
    }
  }
</script>

<div class="settings">
  <section class="block">
    <!-- Posts are by authors and the team only, readers comment (stage 1, 2026-10-22): a rule, not a choice. -->
    <p class="hint rule">{$t('chan_post_mode_hint')}</p>
    <div class="option">
      <span class="label">{$t('chan_rule_comments')}</span>
      <button class="toggle" class:on={comments} role="switch" aria-checked={comments} aria-label={$t('chan_rule_comments')} disabled={busy}
        onclick={() => { comments = !comments; saved = false; }}></button>
    </div>
    <p class="hint">{$t(comments ? 'chan_comments_on_hint' : 'chan_comments_off_hint')}</p>
  </section>

  <section class="block">
    <div class="option">
      <span class="label">{$t('chan_rule_schedule')}</span>
      <button class="toggle" class:on={scheduled} role="switch" aria-checked={scheduled} aria-label={$t('chan_rule_schedule')} disabled={busy}
        onclick={() => { scheduled = !scheduled; saved = false; }}></button>
    </div>
    <p class="hint">{$t('chan_schedule_hint')}</p>
    {#if scheduled}
      <div class="days">
        {#each DAYS as d (d)}
          <button type="button" class="chip" class:active={days.includes(d)} aria-pressed={days.includes(d)} disabled={busy} onclick={() => toggleDay(d)}>{names[d - 1]}</button>
        {/each}
      </div>
      <div class="times">
        <label class="time">
          <span>{$t('chan_schedule_from')}</span>
          <input class="field" type="time" bind:value={from} class:bad={!validTime(from)} disabled={busy} oninput={() => (saved = false)} />
        </label>
        <label class="time">
          <span>{$t('chan_schedule_to')}</span>
          <input class="field" type="time" bind:value={to} class:bad={!validTime(to)} disabled={busy} oninput={() => (saved = false)} />
        </label>
      </div>
      <div class="zone">
        <span class="sub">{$t('chan_schedule_tz')}</span>
        <CustomSelect options={zones} bind:value={tz} onchange={() => (saved = false)} />
      </div>
      {#if !valid}<p class="hint bad-text">{$t('chan_schedule_invalid')}</p>{/if}
    {/if}
  </section>

  <section class="block">
    <div class="label">{$t('chan_rule_premod')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('chan_rule_premod')}>
      {#each PREMODS as v (v)}
        <button class="seg-btn" class:active={premod === v} role="radio" aria-checked={premod === v} disabled={busy}
          onclick={() => { premod = v; saved = false; }}>{$t(`chan_premod_${v}` as 'chan_premod_off')}</button>
      {/each}
    </div>
    <p class="hint">{$t(`chan_premod_hint_${premod}` as 'chan_premod_hint_off')}</p>
  </section>

  {#if error}<div class="error-msg">{error}</div>{/if}
  <div class="actions">
    <button class="btn btn-primary btn-sm" disabled={busy || !valid} onclick={save}>{$t('chan_save')}</button>
    {#if saved}<span class="saved">{$t('chan_saved')}</span>{/if}
  </div>
</div>

<style>
  .settings { display: flex; flex-direction: column; gap: var(--sp-5); }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .hint.rule { color: var(--text-2); }
  .bad-text { color: var(--danger-text); }
  .option { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); }
  /* A phone is narrower than three choices in one line: each wraps its words. */
  .seg { align-self: flex-start; max-width: 100%; }
  .seg-btn { flex: 1 1 auto; min-width: 0; height: auto; min-height: 34px; padding-block: 6px; white-space: normal; text-align: center; justify-content: center; line-height: 1.25; }
  .days { display: flex; flex-wrap: wrap; gap: 6px; }
  .times { display: flex; gap: var(--sp-3); flex-wrap: wrap; }
  .time { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-2); flex: 1; min-width: 110px; }
  .zone { display: flex; flex-direction: column; gap: 4px; }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 7px 10px;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .field.bad { border-color: var(--danger-border); }
  .actions { display: flex; align-items: center; gap: var(--sp-3); }
  .saved { font-size: var(--fs-xs); color: var(--success-text, var(--success)); }
  @media (pointer: coarse) {
    .field { font-size: var(--fs-md); min-height: 44px; }
    .chip { min-height: 40px; padding-inline: 12px; }
  }
</style>

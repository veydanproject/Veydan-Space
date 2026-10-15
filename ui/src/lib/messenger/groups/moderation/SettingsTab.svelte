<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Who may write in a group and when, and whose messages wait for a
  moderator. A manager's: the change is an operation of the group's log,
  which every member's device applies the same way.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t, locale, localeTag } from '$lib/core/i18n';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import { groupStore } from '../groupStore.svelte';
  import { moderationError } from '../errors';
  import { messengerApi, type GroupModSettings, type GroupPostMode, type GroupPremod, type MessengerGroup } from '../../api';
  import { DAYS, deviceZone, scheduleValid, validTime, weekdayNames, zoneHint, zoneList } from './schedule';

  interface Props {
    group: MessengerGroup;
  }
  let { group }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  const POST_MODES: GroupPostMode[] = ['all', 'managers'];
  const PREMODS: GroupPremod[] = ['off', 'newcomers', 'all'];

  // What the panel showed is where the form starts; the runtime's answer replaces it.
  const start = untrack(() => group);
  let postMode = $state<GroupPostMode>(start.post_mode ?? 'all');
  let premod = $state<GroupPremod>(start.premod ?? 'off');
  let scheduled = $state(!!start.schedule);
  let days = $state<number[]>(start.schedule?.days ?? [1, 2, 3, 4, 5]);
  let from = $state(start.schedule?.from ?? '09:00');
  let to = $state(start.schedule?.to ?? '21:00');
  let tz = $state(start.schedule?.tz ?? deviceZone());
  let busy = $state(false);
  let error = $state('');
  let saved = $state(false);

  function fill(s: GroupModSettings) {
    postMode = s.post_mode === 'managers' ? 'managers' : 'all';
    premod = s.premod === 'all' || s.premod === 'newcomers' ? s.premod : 'off';
    scheduled = !!s.schedule;
    if (s.schedule) { days = [...s.schedule.days]; from = s.schedule.from; to = s.schedule.to; tz = s.schedule.tz; }
  }

  $effect(() => {
    const id = group.id;
    untrack(() => {
      messengerApi.moderation.groupModSettings(id).then(fill).catch((e) => (error = moderationError(e, tr)));
    });
  });

  const names = $derived(weekdayNames(localeTag($locale)));
  const zones = $derived(zoneList(deviceZone(), tz).map((z) => ({ value: z, label: z.replaceAll('_', ' ') })));
  const draft = $derived<GroupModSettings>({
    post_mode: postMode, premod, schedule: scheduled ? { tz, days: [...days].sort((a, b) => a - b), from, to } : undefined,
  });
  const valid = $derived(!draft.schedule || scheduleValid(draft.schedule));
  /** The zone the group has now is one the core does not know: said here, where a manager picks another. */
  const unknownZone = $derived(zoneHint(group));

  function toggleDay(d: number) {
    days = days.includes(d) ? days.filter((x) => x !== d) : [...days, d];
    saved = false;
  }

  async function save() {
    if (!valid) return;
    error = ''; saved = false; busy = true;
    try {
      await messengerApi.moderation.groupModSettingsSet(group.id, draft);
      saved = true;
      await groupStore.refresh(group.id);
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = false;
    }
  }
</script>

<div class="settings">
  <section class="block">
    <div class="label">{$t('msg_mod_post_mode')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_mod_post_mode')}>
      {#each POST_MODES as v (v)}
        <button class="seg-btn" class:active={postMode === v} role="radio" aria-checked={postMode === v} disabled={busy}
          onclick={() => { postMode = v; saved = false; }}>{$t(`msg_mod_post_mode_${v}` as 'msg_mod_post_mode_all')}</button>
      {/each}
    </div>
    <p class="hint">{$t('msg_mod_post_mode_hint')}</p>
  </section>

  <section class="block">
    <div class="option">
      <span class="label">{$t('msg_mod_schedule')}</span>
      <button class="toggle" class:on={scheduled} role="switch" aria-checked={scheduled} aria-label={$t('msg_mod_schedule')} disabled={busy}
        onclick={() => { scheduled = !scheduled; saved = false; }}></button>
    </div>
    <p class="hint">{$t('msg_mod_schedule_hint')}</p>
    {#if scheduled}
      <div class="days">
        {#each DAYS as d (d)}
          <button type="button" class="chip" class:active={days.includes(d)} aria-pressed={days.includes(d)} disabled={busy} onclick={() => toggleDay(d)}>{names[d - 1]}</button>
        {/each}
      </div>
      <div class="times">
        <label class="time">
          <span>{$t('msg_mod_schedule_from')}</span>
          <input class="field" type="time" bind:value={from} class:bad={!validTime(from)} disabled={busy} oninput={() => (saved = false)} />
        </label>
        <label class="time">
          <span>{$t('msg_mod_schedule_to')}</span>
          <input class="field" type="time" bind:value={to} class:bad={!validTime(to)} disabled={busy} oninput={() => (saved = false)} />
        </label>
      </div>
      <div class="zone">
        <span class="sub">{$t('msg_mod_schedule_tz')}</span>
        <CustomSelect options={zones} bind:value={tz} onchange={() => (saved = false)} />
      </div>
      {#if !valid}<p class="hint bad-text">{$t('msg_mod_schedule_invalid')}</p>{/if}
      {#if unknownZone}<p class="hint bad-text">{$t('msg_mod_schedule_tz_unknown', { tz: unknownZone })}</p>{/if}
    {/if}
  </section>

  <section class="block">
    <div class="label">{$t('msg_mod_premod')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_mod_premod')}>
      {#each PREMODS as v (v)}
        <button class="seg-btn" class:active={premod === v} role="radio" aria-checked={premod === v} disabled={busy}
          onclick={() => { premod = v; saved = false; }}>{$t(`msg_mod_premod_${v}` as 'msg_mod_premod_off')}</button>
      {/each}
    </div>
    <p class="hint">{$t(`msg_mod_premod_hint_${premod}` as 'msg_mod_premod_hint_off')}</p>
  </section>

  {#if error}<div class="error-msg">{error}</div>{/if}
  <div class="actions">
    <button class="btn btn-primary btn-sm" disabled={busy || !valid} onclick={save}>{$t('msg_mod_save')}</button>
    {#if saved}<span class="saved">{$t('msg_mod_saved')}</span>{/if}
  </div>
</div>

<style>
  .settings { display: flex; flex-direction: column; gap: var(--sp-5); }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
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
    .field { font-size: 16px; min-height: 44px; }
    .chip { min-height: 40px; padding-inline: 12px; }
  }
</style>

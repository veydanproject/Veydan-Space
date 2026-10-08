<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The strip at the top of a group's chat while a call is on in it: "A call
  is on · 3 participants", the faces of who is in, and Join. A call of a
  group does not ring: this is how a member finds it. While I am in it,
  the strip says so and leads back to the room. A refusal shows under it
  for a moment, also when the call it was about went and the strip with
  it ("The call is already over").

  A screen reader hears which call is on when that changes, never the
  clock that runs in the strip while I am in the call.
-->
<script lang="ts">
  import { locale, t, countKey } from '$lib/core/i18n';
  import Avatar from '../contacts/Avatar.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { onPhone } from '../shared/phone';
  import { messengerStore } from '../store.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { bannerKey, groupStatusText, peopleIn } from './group';
  import { groupCallStore } from './groupCallStore.svelte';
  import { openGroupRoom } from './room';
  import { callErrorText } from './words';

  let { groupId }: { groupId: string } = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);
  const phone = onPhone();

  const mine = $derived(groupCallStore.inGroup(groupId));
  const live = $derived(mine ? null : groupCallStore.announcedIn(groupId));
  const people = $derived(mine ? peopleIn(mine) : (live?.participants.length ?? 0));
  const faces = $derived(live ? live.participants.slice(0, 4) : []);
  const more = $derived(live ? Math.max(0, live.participants.length - faces.length) : 0);
  const who = $derived(live ? live.participants.map((p) => nameStore.label(p)).join(', ') : '');
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);
  const reason = $derived(
    callStore.loaded && !callStore.available ? $t('msg_call_unavailable')
      : callStore.call || groupCallStore.call ? $t('msg_call_err_busy')
      : !sessionActive ? $t('msg_call_err_locked')
      : '',
  );
  const status = $derived(mine ? groupStatusText(mine, null, groupCallStore.now, tr) : '');
  const count = $derived(people > 0 ? $t(countKey('msg_gcall_people', people, $locale), { n: String(people) }) : '');
  const what = $derived(mine || live ? tr(bannerKey(!!mine, (mine ?? live)?.media)) : '');

  let refusal = $state('');
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function join() {
    if (reason) return;
    const { view, refusal: why } = await groupCallStore.join(groupId);
    if (view) { openGroupRoom(phone); return; }
    if (!why) return;
    refusal = callErrorText(why, tr);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (refusal = ''), 5000);
  }

  $effect(() => () => { if (timer) clearTimeout(timer); });
</script>

<span class="sr" aria-live="polite">{what}</span>
{#if mine || live}
  <div class="banner" class:mine={!!mine}>
    <span class="dot" aria-hidden="true"><CallIcon name={(mine ?? live)?.media === 'video' ? 'video' : 'phone'} size={14} /></span>
    <span class="words">
      <span class="what">{what}</span>
      {#if count}<span class="count">{count}</span>{/if}
      {#if mine && status}<span class="clock">{status}</span>{/if}
    </span>
    {#if faces.length}
      <span class="faces" title={who}>
        {#each faces as pk (pk)}
          <span class="face"><Avatar url={nameStore.picture(pk)} label={nameStore.label(pk)} seed={pk} size={22} /></span>
        {/each}
        {#if more}<span class="face more">+{more}</span>{/if}
      </span>
    {/if}
    {#if mine}
      <button class="btn-join soft" onclick={() => openGroupRoom(phone)}>{$t('msg_gcall_open_room')}</button>
    {:else}
      <button class="btn-join" disabled={!!reason || groupCallStore.busy} title={reason || undefined} onclick={join}>
        <CallIcon name="phone" size={13} />{$t('msg_gcall_join')}
      </button>
    {/if}
  </div>
{/if}
{#if refusal}<div class="refusal" role="alert">{refusal}</div>{/if}

<style>
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .banner {
    display: flex; align-items: center; gap: var(--sp-2); padding: 7px var(--sp-3) 7px var(--sp-4);
    background: var(--success-bg); border-bottom: 1px solid var(--success-border); color: var(--success-text);
    font-size: var(--fs-xs); min-width: 0;
  }
  .dot {
    width: 26px; height: 26px; border-radius: 50%; flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center;
    background: var(--success); color: #fff; position: relative;
  }
  .banner:not(.mine) .dot::after {
    content: ''; position: absolute; inset: -3px; border-radius: 50%; border: 2px solid var(--success);
    animation: pulse 2s ease-out infinite;
  }
  .words { display: flex; align-items: baseline; gap: 6px; flex-wrap: wrap; min-width: 0; flex: 1; }
  .what { font-weight: var(--fw-bold); white-space: nowrap; }
  .count { color: var(--text-2); white-space: nowrap; }
  .clock { font-variant-numeric: tabular-nums; color: var(--text-2); }
  .faces { display: inline-flex; align-items: center; flex-shrink: 0; }
  .face { margin-left: -6px; border-radius: 50%; box-shadow: 0 0 0 2px var(--success-bg); display: inline-flex; }
  .face:first-child { margin-left: 0; }
  .face.more {
    width: 22px; height: 22px; align-items: center; justify-content: center; font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    background: var(--surface-3); color: var(--text-2);
  }
  .btn-join {
    display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0; padding: 5px 12px; border-radius: var(--radius-pill);
    border: none; cursor: pointer; font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-bold); background: var(--success); color: #fff;
  }
  .btn-join.soft { background: var(--surface); color: var(--success-text); border: 1px solid var(--success-border); }
  .btn-join:disabled { opacity: 0.5; cursor: default; }
  @media (hover: hover) { .btn-join:hover:not(:disabled) { filter: brightness(1.08); } }
  @media (pointer: coarse) { .btn-join { padding: 7px 14px; } }
  .refusal {
    padding: 6px var(--sp-4); font-size: var(--fs-xs); border-bottom: 1px solid var(--danger-border);
    background: var(--danger-bg); color: var(--danger-text);
  }
  @keyframes pulse { from { transform: scale(1); opacity: 0.7; } to { transform: scale(1.45); opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .banner .dot::after { animation: none; opacity: 0; } }
</style>

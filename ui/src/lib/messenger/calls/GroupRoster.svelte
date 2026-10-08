<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Who is in the room of a group call: each seat with its name and whether
  its identity was confirmed (its word of identity, signed by a member's
  key, checked here), whether its pictures come, whether it speaks; my
  own microphone when it is off (another's is not known here yet). A seat not
  confirmed yet is nameless and not heard. The count says the room's
  limit when the node gave one.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Avatar from '../contacts/Avatar.svelte';
  import type { GroupCallView } from '../api';
  import { nameStore } from '../groups/names.svelte';
  import CallIcon from './CallIcon.svelte';
  import { groupCallStore } from './groupCallStore.svelte';
  import { seatMuted, seatSendsVideo, seatsInOrder } from './group';

  interface Props {
    call: GroupCallView;
    /** Dark: over the room's videos. */
    dark?: boolean;
  }
  let { call, dark = false }: Props = $props();

  const seats = $derived(seatsInOrder(call.participants));
  const count = $derived(
    call.max_participants > 0
      ? $t('msg_gcall_roster_count_of', { n: String(seats.length), max: String(call.max_participants) })
      : String(seats.length),
  );
</script>

<section class="roster" class:dark aria-label={$t('msg_gcall_roster')}>
  <header class="head"><span>{$t('msg_gcall_roster')}</span><span class="count">{count}</span></header>
  <ul>
    {#each seats as p (p.id)}
      {@const name = p.me ? $t('msg_gcall_you_named', { name: p.npub ? nameStore.label(p.npub) : '' }) : p.verified && p.npub ? nameStore.label(p.npub) : $t('msg_gcall_unverified')}
      <li class:unverified={!p.verified}>
        <Avatar url={p.npub ? nameStore.picture(p.npub) : null} label={p.npub ? nameStore.label(p.npub) : name} seed={p.npub ?? `seat:${p.id}`} size={30} />
        <span class="who">
          <span class="name">{name}</span>
          <span class="state">
            {#if p.verified}
              <span class="ok"><CallIcon name="shield-check" size={11} />{$t(p.me ? 'msg_gcall_this_device' : 'msg_gcall_verified')}</span>
            {:else}
              <span class="wait"><CallIcon name="shield-question" size={11} />{$t('msg_gcall_checking')}</span>
            {/if}
          </span>
        </span>
        <span class="marks">
          {#if p.speaking && p.verified}<span class="talk" title={$t('msg_gcall_speaking')}><CallIcon name="speaking" size={13} /></span>{/if}
          {#if seatSendsVideo(p, call, (seat) => !!groupCallStore.showing[seat])}<span class="mark" title={$t('msg_call_camera')}><CallIcon name="video" size={13} /></span>{/if}
          {#if seatMuted(p, call)}<span class="mark off" title={$t('msg_call_muted')}><CallIcon name="mic-off" size={13} /></span>{/if}
        </span>
      </li>
    {/each}
  </ul>
  <p class="note"><CallIcon name="shield-check" size={11} />{$t('msg_gcall_verified_hint')}</p>
</section>

<style>
  .roster { display: flex; flex-direction: column; min-height: 0; color: var(--text); }
  .head {
    display: flex; align-items: baseline; justify-content: space-between; gap: 8px; padding: 10px 12px 6px;
    font-size: var(--fs-xs); font-weight: var(--fw-bold); text-transform: uppercase; letter-spacing: 0.4px; color: var(--text-3);
  }
  .count { font-variant-numeric: tabular-nums; text-transform: none; letter-spacing: 0; }
  ul { list-style: none; margin: 0; padding: 0 6px; overflow-y: auto; flex: 1; min-height: 0; }
  li { display: flex; align-items: center; gap: 10px; padding: 6px; border-radius: var(--radius-sm); }
  li.unverified { opacity: 0.7; }
  .who { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { font-size: var(--fs-2xs); }
  .ok, .wait { display: inline-flex; align-items: center; gap: 3px; }
  .ok { color: var(--success-text); }
  .wait { color: var(--warn-text); }
  .marks { display: inline-flex; gap: 4px; flex-shrink: 0; color: var(--text-3); }
  .mark, .talk { display: inline-flex; }
  .mark.off { color: var(--danger-text); }
  .talk { color: var(--success-text); }
  .note { display: flex; gap: 5px; align-items: flex-start; margin: 0; padding: 8px 12px 10px; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; }
  .note :global(svg) { margin-top: 1px; }

  .dark { color: #fff; }
  .dark .head, .dark .note, .dark .marks { color: rgba(255, 255, 255, 0.6); }
  .dark .ok { color: #7ee2a8; }
  .dark .wait { color: #ffd38a; }
  .dark .talk { color: #7ee2a8; }
  .dark .mark.off { color: #ffb4a8; }
</style>

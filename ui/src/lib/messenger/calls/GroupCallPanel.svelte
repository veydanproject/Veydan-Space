<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The room of a group call folded on a computer: a capsule in the middle
  of the window's top bar, as a call of two has, with the group, the clock
  and how many are in, the microphone, the room unfolded, the chat, Leave.
-->
<script lang="ts">
  import { t, locale, countKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { groupStore } from '../groups/groupStore.svelte';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { groupStatusText, peopleIn } from './group';
  import { groupCallStore } from './groupCallStore.svelte';

  interface Props {
    onchat: (chatId: string) => void;
  }
  let { onchat }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(groupCallStore.shown ? null : groupCallStore.call);
  const group = $derived(call ? groupStore.groups[call.group_id] ?? null : null);
  const name = $derived(group?.name || (call ? chatStore.chats.find((c) => c.id === call.chat_id)?.title : '') || '');
  const face = $derived({ name, picture: group?.picture || null, seed: call?.group_id ?? '' });
  const status = $derived(groupStatusText(call, null, groupCallStore.now, tr));
  const people = $derived(call ? peopleIn(call) : 0);
  const talking = $derived(call?.participants.some((p) => p.speaking && p.verified && !p.me) ?? false);
</script>

{#if call}
  <div class="card" role="group" aria-label={name}>
    <CallFace peer={face} size={30} level={talking ? 0.6 : 0} />
    <button class="who" onclick={() => (groupCallStore.shown = true)} title={$t('msg_gcall_unfold')}>
      <span class="name">{name}</span>
      <span class="status">
        <span class="phase" class:clock={call.phase === 'in_room'}>{status}</span>
        <span class="people"><Icon name="users" size={11} />{$t(countKey('msg_gcall_people', people, $locale), { n: String(people) })}</span>
        {#if call.muted}<span class="muted-mark"><CallIcon name="mic-off" size={11} /></span>{/if}
      </span>
    </button>
    <div class="controls">
      <button class="ctl" class:on={call.muted} disabled={groupCallStore.busy} onclick={() => groupCallStore.toggleMute()}
        title={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-label={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-pressed={call.muted}>
        <CallIcon name={call.muted ? 'mic-off' : 'mic'} size={15} />
      </button>
      <button class="ctl" onclick={() => (groupCallStore.shown = true)} title={$t('msg_gcall_unfold')} aria-label={$t('msg_gcall_unfold')}>
        <Icon name="layout-grid" size={15} />
      </button>
      <button class="ctl" onclick={() => onchat(call.chat_id)} title={$t('msg_call_open_chat')} aria-label={$t('msg_call_open_chat')}>
        <Icon name="message-circle" size={15} />
      </button>
      <button class="ctl end" disabled={groupCallStore.ending} onclick={() => groupCallStore.leave()} title={$t('msg_gcall_leave')} aria-label={$t('msg_gcall_leave')}>
        <CallIcon name="hangup" size={16} />
      </button>
    </div>
  </div>
{/if}

<style>
  .card {
    position: fixed; top: calc(var(--overlay-inset, 0px) + 5px); left: 50%; z-index: 72; transform: translateX(-50%);
    width: max-content; max-width: min(620px, calc(100vw - 24px));
    display: flex; align-items: center; gap: var(--sp-2);
    padding: 4px 4px 4px 5px; border-radius: var(--radius-pill);
    background: var(--surface); color: var(--text); border: 1px solid var(--border-strong); box-shadow: var(--shadow-lg);
    animation: drop 240ms cubic-bezier(0.2, 0.9, 0.3, 1.15);
  }
  .who {
    display: flex; align-items: center; gap: var(--sp-2); min-width: 0; flex: 1; padding: 0 var(--sp-1); white-space: nowrap;
    border: none; background: none; color: inherit; font: inherit; cursor: pointer; text-align: left;
  }
  .name { font-weight: var(--fw-bold); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; max-width: 180px; }
  .status { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-2); }
  .phase.clock { font-variant-numeric: tabular-nums; color: var(--success-text); font-weight: var(--fw-semibold); }
  .people { display: inline-flex; align-items: center; gap: 3px; }
  .muted-mark {
    display: inline-flex; padding: 2px 5px; border-radius: var(--radius-pill);
    background: var(--warn-bg); border: 1px solid var(--warn-border); color: var(--warn-text);
  }
  .controls { display: flex; gap: 4px; flex-shrink: 0; }
  .ctl {
    width: 32px; height: 32px; border-radius: 50%; border: 1px solid var(--border); cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; background: var(--surface-2); color: var(--text);
  }
  .ctl:hover:not(:disabled) { background: var(--surface-3); }
  .ctl.on { background: var(--warn-bg); border-color: var(--warn-border); color: var(--warn-text); }
  .ctl.end { width: 40px; border-radius: var(--radius-pill); background: var(--danger); border-color: transparent; color: #fff; }
  .ctl.end:hover:not(:disabled) { background: var(--danger); filter: brightness(1.1); }
  .ctl:disabled { opacity: 0.6; cursor: default; }
  .ctl:focus-visible, .who:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @keyframes drop {
    from { opacity: 0; transform: translate(-50%, -10px) scale(0.96); }
    to { opacity: 1; transform: translate(-50%, 0) scale(1); }
  }
</style>

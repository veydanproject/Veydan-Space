<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The room of a group call on a phone, the whole screen: the group, the
  clock and how many are in at the top, the node the call goes through;
  the seats (GroupStage) in the middle, my own among them with what my
  camera sees; the microphone, the camera (and, while it is on, the
  other camera), the loudspeaker (once the phone says which routes it
  has) and Leave at the bottom. The camera's frames go into the room
  through the phone's shell (call_android.rs), never through this page.
  Who is in (GroupRoster) slides up from the bottom.
  After the room is over it says how until the page takes it away.
-->
<script lang="ts">
  import { t, locale, countKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { groupStore } from '../groups/groupStore.svelte';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { groupStatusText, nodeHost, peopleIn } from './group';
  import { groupCallStore } from './groupCallStore.svelte';
  import GroupRoster from './GroupRoster.svelte';
  import GroupStage from './GroupStage.svelte';
  import { callErrorText } from './words';

  interface Props {
    /** Leaves the screen; the call goes on. */
    onleave: () => void;
  }
  let { onleave }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(groupCallStore.call);
  const over = $derived(call ? null : groupCallStore.over);
  const shown = $derived(call ?? over?.call ?? null);
  const group = $derived(shown ? groupStore.groups[shown.group_id] ?? null : null);
  const name = $derived(group?.name || (shown ? chatStore.chats.find((c) => c.id === shown.chat_id)?.title : '') || '');
  const face = $derived({ name, picture: group?.picture || null, seed: shown?.group_id ?? '' });
  const status = $derived(groupStatusText(call, over?.how ?? null, groupCallStore.now, tr));
  const people = $derived(call ? peopleIn(call) : 0);
  const count = $derived(people ? $t(countKey('msg_gcall_people', people, $locale), { n: String(people) }) : '');
  const error = $derived(groupCallStore.error && call ? callErrorText(groupCallStore.error, tr) : '');
  const node = $derived(call?.node ? nodeHost(call.node) : '');
  const speaker = $derived(groupCallStore.routes?.current === 'speaker');
  const hasSpeaker = $derived(!!groupCallStore.routes?.available.includes('speaker'));
  const cameraOn = $derived(!!call?.video_local && !groupCallStore.screen);
  /** Four buttons and more do not fit a narrow phone at full size (as on the screen of a call of two). */
  const many = $derived(3 + (cameraOn ? 1 : 0) + (hasSpeaker ? 1 : 0) >= 4);

  // Once the sound is the room's (the phone's shell starts with the room's
  // first word), the phone is asked where it can go; once per room.
  let asked = '';
  $effect(() => {
    const c = call;
    if (!c || asked === c.call_id) return;
    asked = c.call_id;
    groupCallStore.loadRoutes();
  });

  let roster = $state(false);
  $effect(() => { if (!call) roster = false; });
</script>

{#if shown}
  <div class="screen">
    <span class="sr" aria-live="polite">{call ? '' : status}</span>
    <header class="top">
      <button class="round-btn" onclick={onleave} aria-label={$t('msg_back')}><Icon name="chevron-down" size={24} /></button>
      <span class="title">
        <span class="name">{name}</span>
        <span class="sub"><span class:clock={call?.phase === 'in_room'}>{status}</span>{#if count}<span>· {count}</span>{/if}</span>
      </span>
      {#if call}
        <button class="round-btn" class:lit={roster} onclick={() => (roster = !roster)} aria-label={$t('msg_gcall_roster')} aria-pressed={roster}>
          <Icon name="users" size={20} />
        </button>
      {/if}
    </header>
    {#if call}
      <div class="chips">
        {#if node}<span class="chip"><CallIcon name="server" size={11} />{$t('msg_gcall_via_node', { node })}</span>{/if}
        <span class="chip quiet"><Icon name="lock" size={11} />{$t('msg_call_e2e')}</span>
        {#if call.muted}<span class="chip warn"><CallIcon name="mic-off" size={11} />{$t('msg_call_muted')}</span>{/if}
      </div>
    {/if}

    <div class="stage">
      {#if call}
        <GroupStage {call} phone />
      {:else}
        <div class="over"><CallFace peer={face} size={112} /><span>{status}</span></div>
      {/if}
    </div>

    {#if error}<p class="error">{error}</p>{/if}

    {#if call}
      <div class="controls" class:many>
        <div class="action">
          <button class="round soft" class:on={call.muted} disabled={groupCallStore.busy} onclick={() => groupCallStore.toggleMute()}
            aria-pressed={call.muted} aria-label={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')}>
            <CallIcon name={call.muted ? 'mic-off' : 'mic'} size={26} />
          </button>
          <span>{$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')}</span>
        </div>
        <div class="action">
          <button class="round soft" class:lit={cameraOn} disabled={groupCallStore.busy} onclick={() => groupCallStore.toggleCamera()}
            aria-pressed={cameraOn} aria-label={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')}>
            <CallIcon name={cameraOn ? 'video' : 'video-off'} size={26} />
          </button>
          <span>{$t('msg_call_camera')}</span>
        </div>
        {#if cameraOn}
          <div class="action">
            <button class="round soft" disabled={groupCallStore.busy} onclick={() => groupCallStore.switchCamera()} aria-label={$t('msg_call_camera_switch')}>
              <Icon name="switch-camera" size={26} />
            </button>
            <span>{$t('msg_call_camera_flip')}</span>
          </div>
        {/if}
        {#if hasSpeaker}
          <div class="action">
            <button class="round soft" class:on={speaker} disabled={groupCallStore.busy} onclick={() => groupCallStore.toggleSpeaker()}
              aria-pressed={speaker} aria-label={$t('msg_call_speaker')}>
              <CallIcon name="speaker" size={26} />
            </button>
            <span>{$t('msg_call_speaker')}</span>
          </div>
        {/if}
        <div class="action">
          <button class="round decline" disabled={groupCallStore.ending} onclick={() => groupCallStore.leave()} aria-label={$t('msg_gcall_leave')}>
            <CallIcon name="hangup" size={30} />
          </button>
          <span>{$t('msg_gcall_leave')}</span>
        </div>
      </div>
    {/if}

    {#if roster && call}
      <button class="scrim" aria-label={$t('common_close')} onclick={() => (roster = false)}></button>
      <div class="sheet"><GroupRoster {call} /></div>
    {/if}
  </div>
{/if}

<style>
  .screen {
    position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 6px;
    padding: calc(var(--sat, 0px) + 4px) 0 calc(var(--sab, 0px) + var(--sp-5));
    background: #0b0d10; color: #fff;
  }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .top { display: flex; align-items: center; gap: 6px; min-height: 48px; padding: 0 8px; }
  .round-btn {
    border: none; background: none; color: inherit; width: 44px; height: 44px; border-radius: 50%; cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
  }
  .round-btn.lit { background: rgba(255, 255, 255, 0.16); }
  .title { display: flex; flex-direction: column; min-width: 0; flex: 1; line-height: 1.25; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-extrabold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { display: flex; gap: 5px; font-size: var(--fs-xs); color: rgba(255, 255, 255, 0.72); white-space: nowrap; }
  .clock { font-variant-numeric: tabular-nums; color: #7ee2a8; font-weight: var(--fw-semibold); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 16px; }
  .chip {
    display: inline-flex; align-items: center; gap: 4px; padding: 2px 9px; border-radius: var(--radius-pill);
    background: rgba(255, 255, 255, 0.1); color: rgba(255, 255, 255, 0.85); font-size: var(--fs-2xs); white-space: nowrap;
  }
  .chip.quiet { color: rgba(255, 255, 255, 0.65); }
  .chip.warn { background: var(--warn-bg); color: var(--warn-text); }
  .stage { position: relative; flex: 1; min-height: 0; }
  .over { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; font-size: var(--fs-md); color: rgba(255, 255, 255, 0.85); }
  .error {
    align-self: center; margin: 0 16px; padding: 6px 12px; border-radius: var(--radius-sm); text-align: center; font-size: var(--fs-sm);
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text);
  }
  .controls { display: flex; justify-content: center; gap: clamp(18px, 7vw, 64px); padding-top: 8px; }
  .controls.many { gap: clamp(6px, 2.5vw, 22px); }
  .action { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: rgba(255, 255, 255, 0.88); }
  .many .action { font-size: var(--fs-xs); width: 62px; text-align: center; }
  .round {
    width: 66px; height: 66px; border-radius: 50%; border: none; cursor: pointer; color: #fff;
    display: inline-flex; align-items: center; justify-content: center;
    transition: transform var(--dur-fast) var(--ease), background var(--dur-fast) var(--ease);
  }
  .many .round { width: 58px; height: 58px; }
  .round:active:not(:disabled) { transform: scale(0.94); }
  .round:disabled { opacity: 0.55; }
  .decline { background: var(--danger); box-shadow: var(--shadow-danger); }
  .soft { background: rgba(255, 255, 255, 0.16); border: 1px solid rgba(255, 255, 255, 0.22); }
  .soft.on { background: #fff; color: #111; }
  .soft.lit { background: color-mix(in srgb, var(--accent) 55%, rgba(0, 0, 0, 0.3)); border-color: transparent; }
  .scrim { position: absolute; inset: 0; z-index: 10; border: none; background: rgba(0, 0, 0, 0.45); cursor: pointer; }
  .sheet {
    position: absolute; z-index: 11; left: 0; right: 0; bottom: 0; max-height: 70%; display: flex; flex-direction: column;
    padding-bottom: calc(var(--sab, 0px) + 8px); border-radius: 18px 18px 0 0; background: var(--surface); color: var(--text);
    box-shadow: var(--shadow-lg); animation: up 220ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes up { from { transform: translateY(40px); opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .sheet { animation: none; } }
</style>

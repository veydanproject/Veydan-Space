<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A call on a phone, the whole screen: the peer's face, the phase or the
  clock, the way the sound goes; Answer and Decline while it rings (and
  Answer without video for a video call), the microphone, the camera and
  the end while it talks. After the end it says how the call ended until
  the page takes it away.

  While either side's video goes, the videos fill the screen (VideoStage)
  and the words and buttons lie over them.

  The route of the sound is the call service's on the phone: the
  loudspeaker button is there once the service says which routes it has.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import CallFace from './CallFace.svelte';
  import { callStore } from './callStore.svelte';
  import { callPeer } from './peer';
  import { liveText, statusText } from './status';
  import VideoStage from './VideoStage.svelte';
  import { callErrorText } from './words';

  interface Props {
    /** Leaves the screen; the call goes on. */
    onleave: () => void;
    /** «Открыть чат»: leaves the screen for the call's chat; the call goes on. */
    onchat?: () => void;
  }
  let { onleave, onchat }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(callStore.call);
  const shown = $derived(call ?? callStore.ended?.call ?? null);
  const peer = $derived(shown ? callPeer(shown) : null);
  const status = $derived(statusText(call, callStore.ended, callStore.now, tr));
  /** What a screen reader is told: the phase as it changes, never the clock (it would be read out every second). */
  const live = $derived(liveText(call, callStore.ended, tr));
  const talking = $derived(call?.phase === 'active');
  const ringing = $derived(call?.phase === 'incoming' || call?.phase === 'outgoing');
  const rtt = $derived(talking ? callStore.stats?.rtt_ms : undefined);
  const error = $derived(callStore.error && call ? callErrorText(callStore.error, tr) : '');
  const speaker = $derived(callStore.routes?.current === 'speaker');
  const video = $derived(callStore.video);
  const cameraOn = $derived(!!call?.video_local && !call.video_screen);
  const hasSpeaker = $derived(!!callStore.routes?.available.includes('speaker'));
  /** The call rings while I sit in the room of a group call: it cannot be taken here, only refused. */
  const busy = $derived(call?.phase === 'incoming' && callStore.busyWithGroup);
  /** How many buttons stand in the row while it talks: four close up, five shrink on a narrow phone (as in a group's room). */
  const buttons = $derived(3 + (cameraOn ? 1 : 0) + (hasSpeaker ? 1 : 0));

  // Once the sound is the call's (it is not while a call only rings here),
  // the phone is asked where it can go; once per call.
  let asked = '';
  $effect(() => {
    const c = call;
    if (!c || c.phase === 'incoming' || asked === c.call_id) return;
    asked = c.call_id;
    callStore.loadRoutes();
  });
</script>

{#if shown && peer}
  <div class="screen" class:ended={!call} class:video>
    <span class="sr" aria-live="polite">{live}</span>
    {#if video && call}
      <div class="stage"><VideoStage {call} {peer} {status} phone level={callStore.level} /></div>
    {/if}

    <header class="top">
      <button class="round-btn" onclick={onleave} aria-label={$t('common_back')}><Icon name="chevron-down" size={24} /></button>
      <span class="e2e"><Icon name="lock-check" size={12} />{$t('msg_call_e2e')}</span>
      <span class="spacer"></span>
      {#if onchat}<button class="icon-btn" onclick={onchat} aria-label={$t('msg_call_open_chat')}><Icon name="message-circle" size={22} /></button>{/if}
    </header>

    {#if video}
      <div class="caption">
        <span class="cap-name">{peer.name}</span>
        <span class="cap-status" class:clock={talking}>{status}</span>
        {#if (talking && call?.via) || call?.muted}
          <span class="cap-chips">
            {#if talking && call?.via}
              <span class="call-chip small" class:relay={call.via === 'relay'}>
                <Icon name={call.via === 'relay' ? 'route' : 'arrows-exchange'} size={11} />{$t(call.via === 'relay' ? 'msg_call_via_relay' : 'msg_call_via_direct')}
              </span>
            {/if}
            {#if rtt != null}<span class="call-chip small quiet">{$t('msg_call_rtt', { ms: String(rtt) })}</span>{/if}
            {#if call?.muted}<span class="call-chip warn small"><Icon name="mic-off" size={11} />{$t('msg_call_muted')}</span>{/if}
          </span>
        {/if}
      </div>
      <div class="grow"></div>
      {#if error}<p class="error error-msg over">{error}</p>{/if}
    {:else}
      <div class="middle">
        <CallFace {peer} size={132} {ringing} level={talking ? callStore.level : 0} />
        <h1 class="name">{peer.name}</h1>
        <div class="status" class:clock={talking}>{status}</div>
        <div class="chips">
          {#if call?.via}
            <span class="call-chip" class:relay={call.via === 'relay'}>
              <Icon name={call.via === 'relay' ? 'route' : 'arrows-exchange'} size={13} />{$t(call.via === 'relay' ? 'msg_call_via_relay' : 'msg_call_via_direct')}
            </span>
          {/if}
          {#if rtt != null}<span class="call-chip quiet">{$t('msg_call_rtt', { ms: String(rtt) })}</span>{/if}
          {#if call?.muted}<span class="call-chip warn"><Icon name="mic-off" size={13} />{$t('msg_call_muted')}</span>{/if}
          {#if busy}<span class="call-chip warn"><Icon name="phone-x" size={13} />{$t('msg_call_busy_in_group')}</span>{/if}
        </div>
        {#if error}<p class="error error-msg">{error}</p>{/if}
      </div>
    {/if}

    <div class="controls" class:many={call?.phase !== 'incoming' && buttons >= 4} class:crowd={call?.phase !== 'incoming' && buttons >= 5}>
      {#if call?.phase === 'incoming'}
        <div class="action">
          <button class="round decline" disabled={callStore.ending} onclick={() => callStore.decline()} aria-label={$t('msg_call_decline')}><Icon name="phone-hangup" size={30} /></button>
          <span>{$t('msg_call_decline')}</span>
        </div>
        {#if call.media === 'video' && !busy}
          <div class="action">
            <button class="round soft" disabled={callStore.busy} onclick={() => callStore.acceptWithoutVideo()} aria-label={$t('msg_call_answer_audio')}><Icon name="video-off" size={26} /></button>
            <span>{$t('msg_call_answer_voice')}</span>
          </div>
        {/if}
        {#if !busy}
          <div class="action">
            <button class="round answer" disabled={callStore.busy} onclick={() => callStore.accept()} aria-label={$t('msg_call_answer')}>
              <Icon name={call.media === 'video' ? 'video' : 'phone'} size={30} />
            </button>
            <span>{$t('msg_call_answer')}</span>
          </div>
        {/if}
      {:else if call}
        <div class="action">
          <button class="round soft" class:on={call.muted} disabled={callStore.busy || call.phase === 'outgoing'} onclick={() => callStore.toggleMute()}
            aria-pressed={call.muted} aria-label={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')}>
            <Icon name={call.muted ? 'mic-off' : 'mic'} size={26} />
          </button>
          <span>{$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')}</span>
        </div>
        <div class="action">
          <button class="round soft" class:lit={cameraOn} disabled={callStore.busy} onclick={() => callStore.toggleCamera()}
            aria-pressed={cameraOn} aria-label={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')}>
            <Icon name={cameraOn ? 'video' : 'video-off'} size={26} />
          </button>
          <span>{$t('msg_call_camera')}</span>
        </div>
        {#if cameraOn}
          <div class="action">
            <button class="round soft" disabled={callStore.busy} onclick={() => callStore.switchCamera()} aria-label={$t('msg_call_camera_switch')}>
              <Icon name="switch-camera" size={26} />
            </button>
            <span>{$t('msg_call_camera_flip')}</span>
          </div>
        {/if}
        {#if hasSpeaker}
          <div class="action">
            <button class="round soft" class:on={speaker} disabled={callStore.busy} onclick={() => callStore.toggleSpeaker()}
              aria-pressed={speaker} aria-label={$t('msg_call_speaker')}>
              <Icon name="volume" size={26} />
            </button>
            <span>{$t('msg_call_speaker')}</span>
          </div>
        {/if}
        <div class="action">
          <button class="round decline" disabled={callStore.ending} onclick={() => callStore.hangUp()} aria-label={$t('msg_call_end')}><Icon name="phone-hangup" size={30} /></button>
          <span>{$t('msg_call_end')}</span>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .screen {
    position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column;
    padding: calc(var(--sat, 0px) + var(--sp-2)) var(--sp-4) calc(var(--sab, 0px) + var(--sp-8));
    /* One look with a group's room (GroupCallScreen, design 15): dark --call-bg in both themes, light words. */
    background: var(--call-bg); color: #fff;
  }
  /* Over the videos: the top and the bottom darkened behind the words. */
  .screen.video { padding-bottom: calc(var(--sab, 0px) + var(--sp-6)); }
  .stage { position: absolute; inset: 0; z-index: 0; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .screen.video::before, .screen.video::after { content: ''; position: absolute; left: 0; right: 0; z-index: 1; pointer-events: none; }
  .screen.video::before { top: 0; height: calc(var(--sat, 0px) + 140px); background: linear-gradient(rgba(0, 0, 0, 0.55), transparent); }
  .screen.video::after { bottom: 0; height: calc(var(--sab, 0px) + 200px); background: linear-gradient(transparent, rgba(0, 0, 0, 0.62)); }
  .top, .caption, .controls, .error.over { position: relative; z-index: 2; }

  .top { display: flex; align-items: center; gap: var(--sp-2); min-height: 48px; }
  /* «Назад» (collapse) as a group's room has it; «Открыть чат» the common .icon-btn (40, zone 44) in the light of the screen. */
  .round-btn {
    border: none; background: none; color: inherit; width: 44px; height: 44px; margin-left: -8px; border-radius: 50%; cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
  }
  .top .icon-btn { color: var(--on-media-text); margin-right: -4px; }
  .top .icon-btn:hover:not(:disabled), .top .icon-btn:active:not(:disabled) { background: var(--on-media-bg-hover); color: #fff; }
  .e2e { display: inline-flex; align-items: center; gap: var(--sp-1); font-size: var(--fs-xs); color: var(--on-media-text-2); position: absolute; left: 50%; transform: translateX(-50%); }
  .spacer { flex: 1; }
  .middle { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sp-2); text-align: center; padding-bottom: var(--sp-8); }
  .name { margin: var(--sp-5) 0 0; font-size: var(--fs-2xl); font-weight: var(--fw-extrabold); letter-spacing: var(--ls-tight); overflow-wrap: anywhere; }
  .status { font-size: var(--fs-md); color: var(--on-media-text); }
  .status.clock { font-variant-numeric: tabular-nums; color: var(--call-ok); font-weight: var(--fw-semibold); }

  /* The name and the clock over the video, on the left under the top bar
     (the small video is on the right). */
  .caption {
    display: flex; flex-direction: column; align-items: flex-start; gap: 3px; margin-top: var(--sp-1);
    max-width: calc(100% - 130px); text-shadow: 0 1px 6px rgba(0, 0, 0, 0.5);
  }
  .cap-name { font-size: var(--fs-lg); font-weight: var(--fw-extrabold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .cap-status { font-size: var(--fs-sm); color: var(--on-media-text); }
  .cap-status.clock { font-variant-numeric: tabular-nums; font-weight: var(--fw-semibold); }
  .cap-chips { display: flex; flex-wrap: wrap; gap: var(--sp-1-5); margin-top: 2px; }
  .grow { flex: 1; }

  .chips { display: flex; gap: var(--sp-2); flex-wrap: wrap; justify-content: center; min-height: 26px; margin-top: var(--sp-1); }
  .call-chip {
    display: inline-flex; align-items: center; gap: var(--sp-1); padding: var(--sp-1) var(--sp-2-5); border-radius: var(--radius-pill);
    font-size: var(--fs-xs); background: var(--on-media-bg); border: 1px solid transparent; color: var(--on-media-text);
  }
  .call-chip.relay { background: color-mix(in srgb, var(--accent) 30%, var(--on-media-bg)); }
  .call-chip.warn { background: var(--warn-bg); border-color: var(--warn-border); color: var(--warn-text); }
  .call-chip.quiet { font-variant-numeric: tabular-nums; }
  .call-chip.small { padding: 1px 8px; font-size: var(--fs-xs); text-shadow: none; }
  .error { margin: var(--sp-2) 0 0; max-width: 320px; }
  .error.over {
    align-self: center; margin: 0 0 var(--sp-3); padding: var(--sp-1-5) var(--sp-3); border-radius: var(--radius-md); text-align: center;
    background: var(--danger-bg); border: 1px solid var(--danger-border);
  }

  .controls { display: flex; justify-content: center; gap: clamp(28px, 10vw, 72px); min-height: 108px; }
  .controls.many { gap: clamp(var(--sp-1-5), 2.5vw, var(--sp-6)); }
  .action { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--on-media-text); }
  .many .action { font-size: var(--fs-xs); width: 72px; text-align: center; }
  .round {
    width: 72px; height: 72px; border-radius: 50%; border: none; cursor: pointer; color: #fff;
    display: inline-flex; align-items: center; justify-content: center;
    transition: transform var(--dur-fast) var(--ease), background var(--dur-fast) var(--ease);
  }
  /* Five of 72 do not fit a phone of 390: there they shrink, as in a group's room. */
  @media (max-width: 420px) {
    .crowd .action { width: 62px; }
    .crowd .round { width: 58px; height: 58px; }
  }
  .round:active:not(:disabled) { transform: scale(0.94); }
  .round:disabled { opacity: 0.45; }
  .decline { background: var(--danger); box-shadow: var(--shadow-danger); }
  .answer { background: var(--success); animation: nudge 1.6s ease-in-out infinite; }
  .soft { background: var(--on-media-bg-hover); border: 1px solid rgba(255, 255, 255, 0.22); }
  .soft.on { background: #fff; color: #111; }
  .soft.lit { background: color-mix(in srgb, var(--accent) 55%, rgba(0, 0, 0, 0.3)); border-color: transparent; }
  .video .soft { backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px); }
  @keyframes nudge { 0%, 70%, 100% { transform: rotate(0); } 76% { transform: rotate(-14deg); } 82% { transform: rotate(12deg); } 88% { transform: rotate(-8deg); } 94% { transform: rotate(5deg); } }
  @media (prefers-reduced-motion: reduce) { .answer { animation: none; } }
</style>

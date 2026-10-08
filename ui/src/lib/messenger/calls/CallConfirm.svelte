<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  "Start a voice call?" / "Start a video call?" before a call button calls,
  so that none is made by accident (a dialog on a computer, a sheet on a
  phone). "Don't ask again" turns the question off at once, on this device;
  it comes back on the Calls tab of the settings.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import type { CallMedia } from '../api';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import type { CallPeer } from './peer';

  interface Props {
    /** The kind asked about; `null`: closed. */
    media: CallMedia | null;
    peer: CallPeer;
    /** The question instead of "Start a voice call?" / "Start a video call?" (a group call asks its own). */
    question?: string;
    /** The words of the button that calls, instead of "Call" / "Video call". */
    action?: string;
    oncall: (media: CallMedia) => void;
    onclose: () => void;
  }
  let { media, peer, question, action, oncall, onclose }: Props = $props();

  // The choice of the box is kept while the dialog is open and applied as it is ticked.
  let never = $state(false);
  $effect(() => { if (media) never = !callStore.confirm; });

  function tick(on: boolean) {
    never = on;
    callStore.setConfirm(!on);
  }

  function go() {
    const m = media;
    if (!m) return;
    onclose();
    oncall(m);
  }
</script>

<Dialog open={media !== null} width="min(380px, calc(100vw - 32px))" {onclose}>
  {#if media}
    <div class="ask">
      <CallFace {peer} size={64} />
      <div class="name">{peer.name}</div>
      <div class="question">{question ?? $t(media === 'video' ? 'msg_call_confirm_video' : 'msg_call_confirm_audio')}</div>
    </div>
    <label class="never">
      <input type="checkbox" checked={never} onchange={(e) => tick((e.currentTarget as HTMLInputElement).checked)} />
      <span>{$t('msg_call_confirm_never')}</span>
    </label>
    {#if never}<p class="hint">{$t('msg_call_confirm_never_hint')}</p>{/if}
  {/if}
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('common_cancel')}</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button type="button" class="btn btn-success" onclick={go} autofocus>
      <CallIcon name={media === 'video' ? 'video' : 'phone'} size={15} />
      {action ?? $t(media === 'video' ? 'msg_call_video_start' : 'msg_call_start')}
    </button>
  {/snippet}
</Dialog>

<style>
  .ask { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; padding: var(--sp-2) 0 var(--sp-4); }
  .name { margin-top: var(--sp-2); font-size: var(--fs-lg); font-weight: var(--fw-extrabold); letter-spacing: -0.3px; overflow-wrap: anywhere; }
  .question { font-size: var(--fs-sm); color: var(--text-2); }
  .never { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-body); cursor: pointer; user-select: none; }
  .hint { margin: var(--sp-2) 0 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
</style>

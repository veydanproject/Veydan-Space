<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The settings of calls: whether calls ring on this device and whether a
  call button asks first; which way a call may take (directly when it
  works, or always through a relay, which keeps my address from the
  peer), the quality of my video. The call nodes and the trust level are in the
  network settings (net/CallNodesPanel.svelte); a card here leads there.

  "Accept calls on this device" is the runtime's (shown once it says it);
  "Ask before calling" is kept in this webview, like the place of the call's window.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { RelayPolicy, VideoQuality } from '../api';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { callErrorText } from './words';
  import { settingsRequest } from '../pages/settingsTab.svelte';

  const POLICIES: RelayPolicy[] = ['auto', 'relay_only'];
  const QUALITIES: VideoQuality[] = ['360p', '720p'];

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  let refusal = $state('');

  onMount(() => {
    // A refusal left from a call's screen is not about these settings.
    if (!callStore.call) callStore.clearError();
    callStore.load().catch((e) => (refusal = callErrorText(e, tr)));
  });

  const error = $derived(refusal || (callStore.error && !callStore.call ? callErrorText(callStore.error, tr) : ''));
</script>

<div class="card calls">
  <div class="card-title"><CallIcon name="incoming" size={16} />{$t('msg_calls_device_title')}</div>
  {#if callStore.incoming !== null}
    <div class="block">
      <div class="line">
        <span>{$t('msg_calls_incoming')}</span>
        <button class="toggle" class:on={callStore.incoming} disabled={callStore.busy || !callStore.loaded}
          onclick={() => { refusal = ''; callStore.setIncoming(!callStore.incoming); }}
          aria-pressed={callStore.incoming} aria-label={$t('msg_calls_incoming')}></button>
      </div>
      <p class="hint">{$t(callStore.incoming ? 'msg_calls_incoming_hint' : 'msg_calls_incoming_off_hint')}</p>
    </div>
  {/if}
  <div class="block">
    <div class="line">
      <span>{$t('msg_calls_confirm')}</span>
      <button class="toggle" class:on={callStore.confirm} onclick={() => callStore.setConfirm(!callStore.confirm)}
        aria-pressed={callStore.confirm} aria-label={$t('msg_calls_confirm')}></button>
    </div>
    <p class="hint">{$t('msg_calls_confirm_hint')}</p>
  </div>
</div>

<div class="card calls">
  <div class="card-title"><CallIcon name="phone" size={16} />{$t('msg_calls_title')}</div>
  <p class="muted">{$t('msg_calls_intro')}</p>
  {#if callStore.loaded && !callStore.available}
    <p class="warn-line">{$t('msg_calls_unavailable')}</p>
  {/if}

  <div class="block">
    <div class="label">{$t('msg_calls_policy')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_calls_policy')}>
      {#each POLICIES as p}
        <button class="seg-btn" class:active={callStore.policy === p} role="radio" aria-checked={callStore.policy === p}
          disabled={callStore.busy || !callStore.loaded} onclick={() => { refusal = ''; callStore.setPolicy(p); }}>
          {$t(`msg_calls_policy_${p}` as 'msg_calls_policy_auto')}
        </button>
      {/each}
    </div>
    <p class="hint">{$t(`msg_calls_policy_${callStore.policy}_hint` as 'msg_calls_policy_auto_hint')}</p>
  </div>
</div>

<div class="card calls">
  <div class="card-title"><CallIcon name="video" size={16} />{$t('msg_calls_video_title')}</div>
  <div class="block">
    <div class="label">{$t('msg_calls_video_quality')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_calls_video_quality')}>
      {#each QUALITIES as q}
        <button class="seg-btn" class:active={callStore.videoQuality === q} role="radio" aria-checked={callStore.videoQuality === q}
          disabled={callStore.busy || !callStore.loaded} onclick={() => { refusal = ''; callStore.setVideoQuality(q); }}>
          {$t(`msg_calls_video_${q}` as 'msg_calls_video_360p')}
        </button>
      {/each}
    </div>
    <p class="hint">{$t('msg_calls_video_hint')}</p>
  </div>
</div>

<div class="card calls">
  <div class="card-title"><Icon name="network" size={16} />{$t('msg_calls_nodes_title')}</div>
  <p class="muted">{$t('msg_calls_nodes_moved')}</p>
  <button class="btn btn-ghost btn-sm open" onclick={() => (settingsRequest.tab = 'network')}>
    <Icon name="arrow-right" size={12} />{$t('msg_card_open')}
  </button>
  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

<style>
  .calls { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted, .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .hint { font-size: var(--fs-xs); color: var(--text-3); }
  .warn-line { margin: 0; font-size: var(--fs-sm); color: var(--warn-text); background: var(--warn-bg); border: 1px solid var(--warn-border); border-radius: var(--radius-sm); padding: 8px 10px; }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); color: var(--text); }
  /* A phone is narrower than the two choices in one line: each wraps its words instead of the row running off. */
  .seg { align-self: flex-start; max-width: 100%; }
  .seg-btn { flex: 1 1 auto; min-width: 0; height: auto; min-height: 34px; padding-block: 6px; white-space: normal; text-align: center; justify-content: center; line-height: 1.25; }
  .open { align-self: flex-start; }
</style>

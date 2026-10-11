<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The profile at the first start, right after the account: a name and a
  picture, so the account the wizard leaves is one people can recognise.
  The picture is set at once (the avatar has commands of its own, and its
  picker is the settings' one); the name is published with Continue. Both
  can be passed by and changed later in the settings of the messenger. An
  account brought from elsewhere comes with its profile, shown here as it is.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import type { OnboardingStepProps } from '$lib/core/module';
  import { profileInputOf } from '../api';
  import { messengerStore } from '../store.svelte';
  import AvatarPicker from '../contacts/AvatarPicker.svelte';
  import { profileErrorText } from '../contacts/profileErrors';

  let { next }: OnboardingStepProps = $props();

  let name = $state('');
  let busy = $state(false);
  let error = $state('');

  // The session with the servers starts a moment after they are chosen: the picture and the name wait for it.
  const canAct = $derived(!!messengerStore.status?.runtime?.session_active);
  const had = $derived((messengerStore.ownProfile?.display_name ?? '').trim());

  onMount(() => {
    name = messengerStore.ownProfile?.display_name ?? '';
    // The status is asked again until the session is there (the store hears of it by its events too).
    const again = setInterval(() => {
      if (messengerStore.status?.runtime?.session_active) clearInterval(again);
      else void messengerStore.refresh().catch(() => {});
    }, 1500);
    return () => clearInterval(again);
  });

  async function save() {
    if (busy) return;
    const wanted = name.trim();
    if (wanted === had) return next();
    busy = true;
    error = '';
    try {
      await messengerStore.saveOwnProfile({ ...profileInputOf(messengerStore.ownProfile), display_name: wanted || null });
      next();
    } catch (e) {
      error = profileErrorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="ob-main">
  <h1 class="ob-title">{$t('msg_ob_profile_title')}</h1>
  <p class="ob-text">{$t('msg_ob_profile_text')}</p>
  <div class="picture"><AvatarPicker disabled={!canAct || busy} /></div>
  <label class="field">
    <span>{$t('msg_ob_profile_name')}</span>
    <input type="text" bind:value={name} maxlength="100" autocomplete="nickname" disabled={busy} />
  </label>
  {#if !canAct}<p class="ob-wait" role="status"><span class="spinner" aria-hidden="true"></span>{$t('msg_ob_profile_connecting')}</p>{/if}
  {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
</div>
<div class="ob-foot">
  <button type="button" class="btn btn-primary" disabled={busy || (!canAct && name.trim() !== had)} onclick={save}>{$t('msg_ob_continue')}</button>
  <button type="button" class="ob-skip" disabled={busy} onclick={next}>{$t('msg_ob_skip')}</button>
</div>

<style>
  .picture { margin-bottom: var(--sp-4); }
  .field { display: flex; flex-direction: column; gap: 6px; margin-bottom: var(--sp-3); font-size: var(--fs-sm); }
  .field span { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .field input {
    font: inherit; color: var(--text); min-height: 44px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 8px 10px;
  }
  .field input:focus { outline: none; border-color: var(--accent-border); }
</style>

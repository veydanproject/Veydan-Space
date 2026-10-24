<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The account at the first start of the app (the wizard of the core,
  `ModuleDef.onboarding`): what an account gives, a new one by a password
  for the copy of its key, or the one the person has already — from another
  device, by the 12 words, through the trusted people, by the key itself.
  The servers are the project's unless the person says otherwise, in a row
  of the form: with them the key is followed by the servers and the free
  plan of the cloud without one more question, and with the person's own
  the step of the servers asks for the relays (ServersStep). The step is
  passed by where the product is more than the chats. It is given the
  wizard's moves and imports nothing of the wizard.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { OnboardingStepProps } from '$lib/core/module';
  import { messengerApi, messengerError, profileLabel } from '../api';
  import { messengerStore } from '../store.svelte';
  import BackupReveal from '../identity/BackupReveal.svelte';
  import IdentityOnboarding from '../identity/IdentityOnboarding.svelte';
  import { cloudOnboarding } from './cloud.svelte';
  import ServersStep from './ServersStep.svelte';

  let { next, detour, intent, only, onback }: OnboardingStepProps = $props();

  /** `key`: the form of an existing key (nsec, ncryptsec, the seed phrase). */
  type Phase = 'intro' | 'have' | 'create' | 'key';
  // svelte-ignore state_referenced_locally
  let phase = $state<Phase>(intent === 'have' ? 'have' : 'intro');
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state('');
  let password = $state('');
  let password2 = $state('');
  /** Whose servers the account uses: the project's unless the person chose their own. */
  let servers = $state<'veydan' | 'own'>('veydan');
  let serversOpen = $state(false);
  /** The project's servers were tried for this key; a failure leaves the choice to the person. */
  let settled = $state(false);

  const locked = $derived(!messengerStore.secretsUnlocked);
  const identity = $derived(messengerStore.identity);
  const backup = $derived(messengerStore.pendingBackup);
  const ready = $derived(!!identity && !backup && !!messengerStore.serversMode);
  /** The account was here when the step opened: it is shown, not walked through again. */
  let wasReady = $state(false);
  const who = $derived(profileLabel(messengerStore.ownProfile) || shortNpub(identity?.npub ?? ''));

  function shortNpub(npub: string): string {
    return npub.length > 20 ? `${npub.slice(0, 10)}…${npub.slice(-6)}` : npub;
  }

  onMount(async () => {
    await messengerStore.refresh().catch(() => {});
    wasReady = !!messengerStore.identity && !messengerStore.pendingBackup && !!messengerStore.serversMode;
    loaded = true;
  });

  async function create() {
    error = '';
    if ([...password.normalize('NFKC')].length < 8) { error = $t('msg_id_err_password_short'); return; }
    if (password.normalize('NFKC') !== password2.normalize('NFKC')) { error = $t('msg_id_err_password_mismatch'); return; }
    busy = true;
    try {
      await messengerStore.createIdentity(password);
      password = ''; password2 = '';
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  /** The key is here and the servers are the project's: they are taken, and the free plan begins. */
  async function settle() {
    settled = true;
    busy = true;
    error = '';
    try {
      await messengerApi.relays.useVeydan();
      // The free plan's proof of work runs while the wizard goes on.
      void cloudOnboarding.start();
      await messengerStore.refresh();
      if (messengerStore.serversMode) next();
    } catch (e) {
      // The step of the servers takes over, with the words of the refusal.
      error = messengerError(e);
    } finally {
      busy = false;
    }
  }

  // A key with no servers yet, from this step or from a screen it sent the
  // person to (another device, the 12 words): the project's are taken once.
  $effect(() => {
    if (loaded && identity && !backup && !messengerStore.serversMode && servers === 'veydan' && !settled && !busy) void settle();
  });

  // The wizard's Back, while the step is on a screen of its own: from the form of a key to the ways in, from those and the new account to the first screen.
  $effect(() => {
    onback(() => {
      if (messengerStore.identity || phase === 'intro') return false;
      phase = phase === 'key' ? 'have' : 'intro';
      error = '';
      return true;
    });
    return () => onback(null);
  });
</script>

{#if !loaded}
  <div class="ob-main center"><span class="spinner" aria-hidden="true"></span></div>
{:else if ready}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="check-circle" size={28} /></span>
    <h1 class="ob-title">{wasReady ? $t('msg_ob_ready_title') : $t('msg_ob_created_title')}</h1>
    <p class="ob-text">{who}</p>
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" onclick={next}>{$t('msg_ob_continue')}</button>
  </div>
{:else if backup}
  <div class="ob-main">
    <div class="ob-embed">
      <BackupReveal npub={backup.npub} ncryptsec={backup.ncryptsec} onDone={() => messengerStore.ackBackup()} />
    </div>
  </div>
{:else if identity}
  <!-- The key is here, the servers are not chosen yet. -->
  {#if busy}
    <div class="ob-main center">
      <p class="ob-wait"><span class="spinner" aria-hidden="true"></span>{$t('msg_srv_fetching')}</p>
    </div>
  {:else}
    <div class="ob-main">
      {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
      <div class="ob-embed">
        <ServersStep start={servers === 'own' ? 'own' : 'choose'} wizard ondone={next} />
      </div>
    </div>
  {/if}
{:else if phase === 'create'}
  <div class="ob-main">
    <h1 class="ob-title">{$t('msg_ob_password_title')}</h1>
    <p class="ob-text">{$t('msg_ob_password_text')}</p>
    <div class="pw">
      <label class="field">
        <span>{$t('msg_id_backup_password')}</span>
        <input type="password" bind:value={password} autocomplete="new-password" disabled={busy} />
      </label>
      <label class="field">
        <span>{$t('msg_id_backup_password_repeat')}</span>
        <input type="password" bind:value={password2} autocomplete="new-password" disabled={busy} />
      </label>
    </div>
    <div class="ob-rows servers">
      <button type="button" class="ob-row" onclick={() => (serversOpen = !serversOpen)} aria-expanded={serversOpen}>
        <Icon name="server-2" size={18} />
        <span class="ob-row-b"><span>{$t('msg_onb_step_servers')}</span></span>
        <span class="ob-row-v">{servers === 'veydan' ? $t('msg_ob_servers_veydan') : $t('msg_ob_servers_own')}</span>
        <span class="chev"><Icon name={serversOpen ? 'chevron-down' : 'chevron-right'} size={16} /></span>
      </button>
    </div>
    {#if serversOpen}
      <div class="ob-opts servers-choice" role="radiogroup">
        <button type="button" class="ob-opt" class:on={servers === 'veydan'} role="radio" aria-checked={servers === 'veydan'} onclick={() => (servers = 'veydan')}>
          <span class="ob-opt-b"><span class="ob-opt-t">{$t('msg_srv_veydan_title')}</span><span class="ob-opt-s">{$t('msg_srv_veydan_text')}</span></span>
          <span class="ob-radio" aria-hidden="true"></span>
        </button>
        <button type="button" class="ob-opt" class:on={servers === 'own'} role="radio" aria-checked={servers === 'own'} onclick={() => (servers = 'own')}>
          <span class="ob-opt-b"><span class="ob-opt-t">{$t('msg_srv_own_title')}</span><span class="ob-opt-s">{$t('msg_srv_own_text')}</span></span>
          <span class="ob-radio" aria-hidden="true"></span>
        </button>
      </div>
    {/if}
    {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" disabled={busy || locked} onclick={create}>{$t('msg_id_create_confirm')}</button>
  </div>
{:else if phase === 'key'}
  <div class="ob-main">
    <div class="ob-embed">
      <IdentityOnboarding importing onback={() => (phase = 'have')} />
    </div>
  </div>
{:else if phase === 'have'}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="user" size={28} /></span>
    <h1 class="ob-title">{$t('msg_ob_have_title')}</h1>
    <p class="ob-text">{$t('msg_ob_have_text')}</p>
    <div class="ob-rows">
      <button type="button" class="ob-row" disabled={locked} onclick={() => detour('/cloud/link')}>
        <Icon name="qr-code" size={18} />
        <span class="ob-row-b"><span>{$t('msg_ob_have_device')}</span><span class="ob-row-s">{$t('msg_ob_have_device_hint')}</span></span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
      <button type="button" class="ob-row" disabled={locked} onclick={() => detour('/cloud/restore')}>
        <Icon name="key" size={18} />
        <span class="ob-row-b"><span>{$t('msg_ob_have_words')}</span><span class="ob-row-s">{$t('msg_ob_have_words_hint')}</span></span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
      <button type="button" class="ob-row" disabled={locked} onclick={() => detour('/cloud/recover')}>
        <Icon name="users" size={18} />
        <span class="ob-row-b"><span>{$t('msg_ob_have_trusted')}</span><span class="ob-row-s">{$t('msg_ob_have_trusted_hint')}</span></span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
      <button type="button" class="ob-row" disabled={locked} onclick={() => (phase = 'key')}>
        <Icon name="file-import" size={18} />
        <span class="ob-row-b"><span>{$t('msg_ob_have_key')}</span><span class="ob-row-s">{$t('msg_ob_have_key_hint')}</span></span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
    </div>
  </div>
{:else}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="user" size={28} /></span>
    <h1 class="ob-title">{$t('msg_ob_title')}</h1>
    <p class="ob-text">{$t('msg_ob_text')}</p>
    <ul class="ob-gives">
      <li><Icon name="message-circle" size={18} /><span>{$t('msg_ob_gives_chats')}</span></li>
      <li><Icon name="refresh-cw" size={18} /><span>{$t('msg_ob_gives_sync')}</span></li>
      <li><Icon name="key" size={18} /><span>{$t('msg_ob_gives_recovery')}</span></li>
    </ul>
    {#if locked}<p class="ob-error">{$t('msg_id_locked')}</p>{/if}
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" disabled={locked} onclick={() => { phase = 'create'; error = ''; }}>{$t('msg_ob_create')}</button>
    <button type="button" class="btn btn-ghost" disabled={locked} onclick={() => { phase = 'have'; error = ''; }}>{$t('msg_ob_have')}</button>
    {#if !only}
      <button type="button" class="ob-skip" onclick={next}>{$t('msg_ob_skip')}</button>
      <p class="ob-foot-hint">{$t('msg_ob_skip_hint')}</p>
    {/if}
  </div>
{/if}

<style>
  /* The password and its repeat side by side in the window of a computer, one under the other on a phone. */
  .pw { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-4); margin-bottom: var(--sp-3); }
  :global(.ob.m) .pw { grid-template-columns: minmax(0, 1fr); gap: var(--sp-3); }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-base); }
  .field span { color: var(--text-2); font-size: var(--fs-base); font-weight: var(--fw-semibold); }
  .field input {
    font: inherit; color: var(--text); min-height: var(--control-h-lg);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 8px 10px;
  }
  :global(.ob.m) .field input { min-height: var(--sp-12); font-size: var(--fs-md); }
  .field input:focus { outline: none; border-color: var(--accent-border); }
  .servers { margin-top: var(--sp-2); }
  .servers-choice { margin-top: var(--sp-3); }
</style>

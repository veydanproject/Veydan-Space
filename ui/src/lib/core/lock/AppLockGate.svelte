<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Hides the whole UI behind the PIN/password while the lock is engaged. -->
<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { lockInputAttrs } from '$lib/core/lock/secret';
  import LockRecoverFlow from '$lib/core/lock/LockRecoverFlow.svelte';
  import { gateScreen } from '$lib/core/lock/gate';
  import { confirmState } from '$lib/core/ui/confirm.svelte';
  import { isMobile } from '$lib/core/platform';

  interface Props {
    children: Snippet;
  }

  let { children }: Props = $props();

  let password = $state('');
  let error = $state(false);
  let busy = $state(false);
  let input = $state<HTMLInputElement | null>(null);

  const kind = $derived(appLock.status.kind);
  const attrs = $derived(lockInputAttrs(kind));
  const hint = $derived(appLock.status.hint);
  // The recovery flow stays up after it unlocked the app: its last step shows the new key.
  const screen = $derived(
    gateScreen({
      ready: appLock.ready,
      failed: appLock.failed,
      locked: appLock.locked,
      recovering: appLock.recovering,
      keyPending: appLock.newRecoveryKey !== null,
    }),
  );

  onMount(() => {
    void appLock.refresh();
    void appLock.listen();
  });

  // A lock closes the flow; a key it issued stays in the store and step 3
  // comes back after the unlock.
  $effect(() => {
    if (appLock.locked) {
      password = '';
      error = false;
      appLock.recovering = false;
      queueMicrotask(() => input?.focus());
    }
  });

  // A question asked before the lock engaged is answered "no" and its dialog
  // goes: nothing of the app is asked over the lock screen.
  $effect(() => {
    if (appLock.gated) confirmState.answer(false);
  });

  async function submit() {
    if (!password || busy) return;
    busy = true;
    try {
      await appLock.unlock(password);
      error = false;
    } catch {
      error = true;
      password = '';
      input?.focus();
    } finally {
      busy = false;
    }
  }
</script>

{#if screen === 'blank'}
  <div class="lock-screen" class:m={isMobile}></div>
{:else if screen === 'error'}
  <div class="lock-screen" class:m={isMobile}>
    <div class="lock-card" role="alert">
      <span class="lock-tile"><Icon name="alert-triangle" size={24} /></span>
      <h3>{$t('lock_status_failed')}</h3>
      <button class="btn btn-primary" type="button" onclick={() => void appLock.refresh()}>{$t('common_try_again')}</button>
    </div>
  </div>
{:else if screen === 'recover'}
  <div class="lock-screen" class:m={isMobile}>
    <div class="lock-card wide">
      <LockRecoverFlow onback={() => (appLock.recovering = false)} ondone={() => appLock.finishRecovery()} />
    </div>
  </div>
{:else if screen === 'lock'}
  <div class="lock-screen" class:m={isMobile}>
    <form class="lock-card" onsubmit={(e) => { e.preventDefault(); void submit(); }}>
      <span class="lock-tile"><Icon name="lock" size={24} /></span>
      <h3>{kind === 'pin' ? $t('lock_title_pin') : $t('lock_title_password')}</h3>
      <input
        bind:this={input}
        class:pin={kind === 'pin'}
        class:invalid={error}
        aria-invalid={!!error}
        type={kind === 'pin' ? 'text' : 'password'}
        inputmode={attrs.inputmode}
        pattern={attrs.pattern}
        bind:value={password}
        placeholder={kind === 'pin' ? $t('lock_kind_pin') : $t('lock_kind_password')}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
      />
      {#if error}
        <span class="error-msg">{$t('lock_wrong')}</span>
        {#if hint}<span class="hint">{$t('lock_hint_prefix', { hint })}</span>{/if}
      {/if}
      <button class="btn btn-primary" type="submit" disabled={!password || busy}>{$t('lock_unlock')}</button>
      <button class="forgot" type="button" onclick={() => (appLock.recovering = true)}>{$t('lock_forgot')}</button>
    </form>
  </div>
{:else}
  {@render children()}
{/if}

<style>
  .lock-screen {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    min-height: 0;
    padding: var(--sp-4);
    padding-top: max(var(--sp-4), var(--sat, 0px));
    overflow-y: auto;
    background: var(--bg-image), var(--bg);
  }
  /* A phone: a page, not the computer's card. The keyboard (--kb) and the gesture stripe under it; with the
     keyboard up the form goes near the top, «Разблокировать» and «Забыли» stay in sight. */
  .lock-screen.m {
    padding: max(var(--sp-4), var(--sat, 0px)) var(--sp-4) calc(var(--kb) + var(--sab) + var(--sp-4));
  }
  :global(.shell.kb-open) .lock-screen.m { align-items: flex-start; padding-top: calc(var(--sat, 0px) + var(--sp-8)); }
  .lock-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-4);
    width: 400px;
    max-width: 100%;
    padding: var(--sp-8);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    background: var(--surface);
    box-shadow: var(--shadow-lg);
    color: var(--text-2);
  }
  .lock-tile {
    display: inline-flex; align-items: center; justify-content: center;
    width: 56px; height: 56px; border-radius: var(--radius-lg);
    background: var(--accent-bg); color: var(--accent-text);
  }
  .lock-card.wide { width: 440px; align-items: stretch; }
  /* The whole width (a column of 560 on a tablet), no frame, plate or shadow; the field 44 and 16, the button 48 (mobile.css). */
  .m .lock-card, .m .lock-card.wide {
    width: 100%;
    max-width: 560px;
    margin: auto 0;
    padding: var(--sp-4) 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
  }
  :global(.shell.kb-open) .m .lock-card { margin: 0; }
  .lock-card h3 { margin: 0; font-size: var(--fs-xl); font-weight: var(--fw-bold); color: var(--text); text-align: center; }
  .lock-card input {
    width: 100%;
    min-height: var(--field-h);
    padding: 0 var(--sp-2-5);
    text-align: center;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--surface-3);
    color: var(--text);
  }
  .lock-card input.invalid { border-color: var(--danger); }
  .lock-card input.pin { -webkit-text-security: disc; }
  .lock-card .btn { width: 100%; justify-content: center; }
  .hint { text-align: center; }
  .forgot {
    border: none;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: var(--fw-bold);
    cursor: pointer;
  }
  .forgot:hover { text-decoration: underline; }
</style>

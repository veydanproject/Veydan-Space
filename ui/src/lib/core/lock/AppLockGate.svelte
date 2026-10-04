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
  <div class="lock-screen"></div>
{:else if screen === 'recover'}
  <div class="lock-screen">
    <div class="lock-card wide">
      <LockRecoverFlow onback={() => (appLock.recovering = false)} ondone={() => appLock.finishRecovery()} />
    </div>
  </div>
{:else if screen === 'lock'}
  <div class="lock-screen">
    <form class="lock-card" onsubmit={(e) => { e.preventDefault(); void submit(); }}>
      <Icon name="lock" size={28} />
      <h3>{kind === 'pin' ? $t('lock_title_pin') : $t('lock_title_password')}</h3>
      <input
        bind:this={input}
        class:pin={kind === 'pin'}
        class:invalid={error}
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
        <span class="err">{$t('lock_wrong')}</span>
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
    background: var(--bg);
  }
  .lock-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    width: 300px;
    max-width: 100%;
    padding: var(--sp-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    color: var(--text-2);
  }
  .lock-card.wide { width: 380px; align-items: stretch; }
  .lock-card h3 { margin: 0; font-size: var(--fs-md); font-weight: 600; color: var(--text); }
  .lock-card input {
    width: 100%;
    min-height: 40px;
    padding: 0 10px;
    font: inherit;
    text-align: center;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--text);
  }
  .lock-card input.invalid { border-color: var(--danger); }
  .lock-card input.pin { -webkit-text-security: disc; }
  .lock-card .btn { width: 100%; justify-content: center; }
  .err { font-size: var(--fs-xs); color: var(--danger); }
  .hint { font-size: var(--fs-sm); color: var(--text); text-align: center; }
  .forgot {
    border: none;
    background: transparent;
    color: var(--text-3);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    text-decoration: underline;
  }
  .forgot:hover { color: var(--text); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Восстановить аккаунт» → «Через доверенных людей» (cloud-recovery.md
  §8.5, §11): the key of the account, «готовим запрос…», then «Запрос
  отправлен» with the code to say by voice, the timer of 30 minutes,
  «Подтвердили: 1 из 3 — npub1…abc», «Поделиться ссылкой», «Отменить»;
  at k confirmations the new PIN or password of the lock and the restore
  of §7; then the 12 words once more.
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import LockSecretFields from '$lib/core/lock/LockSecretFields.svelte';
  import { lockSecretIssue } from '$lib/core/lock/secret';
  import RecoveryKeyCard from '$lib/core/lock/RecoveryKeyCard.svelte';
  import type { LockKind } from '$lib/core/types';
  import { VAULT_UNREACHABLE } from '$lib/cloud/api';
  import Actions from '$lib/cloud/Actions.svelte';
  import Countdown from '$lib/cloud/Countdown.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import StateView from '$lib/cloud/StateView.svelte';
  import { RecoverStore, shortKey, socialErrorKey } from '$lib/cloud/social.svelte';

  const store = new RecoverStore();
  let npub = $state('');
  let kind = $state<LockKind>('pin');
  let password = $state('');
  let confirm = $state('');
  let hint = $state('');
  let copied = $state(false);
  let wordsDone = $state(false);
  let now = $state(Math.floor(Date.now() / 1000));
  let timer: ReturnType<typeof setInterval> | null = null;

  const view = $derived(store.view);
  /** The storage of the account is out of reach: the restore without it is offered. */
  const vaultAway = $derived(store.error?.code === VAULT_UNREACHABLE);
  const errorText = $derived.by(() => {
    if (vaultAway) {
      const detail = store.error?.message;
      return detail ? `${$t('cloud_vault_unreachable')} (${detail})` : $t('cloud_vault_unreachable');
    }
    const key = socialErrorKey(store.error);
    return key ? $t(key) : (store.error?.message ?? '');
  });
  const secretOk = $derived(lockSecretIssue(kind, password, confirm) === null);

  onMount(() => {
    let ticks = 0;
    timer = setInterval(() => {
      now = Math.floor(Date.now() / 1000);
      // The clock moves every second, the server is asked every other one.
      if (++ticks % 2 === 0) void store.poll();
    }, 1000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  async function share() {
    if (!view.link) return;
    try {
      await navigator.clipboard.writeText(view.link);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch {
      // No clipboard here: the link is on the screen.
    }
  }

  async function finish(withoutVault = false) {
    if (await store.finish({ password: password.trim(), kind, hint: hint.trim() || null }, withoutVault)) {
      password = '';
      confirm = '';
      await appLock.refresh();
    }
  }
</script>

{#if store.recovered}
  {#if wordsDone}
    <StateView tone="ok" title={$t('cloud_restore_done')}>
      {#if !store.recovered.restored.vault_joined}<p>{$t('cloud_link_done_vault')}</p>{/if}
      {#if !store.recovered.closed}<p>{$t('cloud_recover_not_closed')}</p>{/if}
      {#snippet actions()}
        <Actions><button class="btn btn-primary" type="button" onclick={() => goto('/messenger')}>{$t('cloud_restore_open')}</button></Actions>
      {/snippet}
    </StateView>
  {:else}
    <Notice tone="ok">{$t('cloud_recover_done')}</Notice>
    <RecoveryKeyCard code={store.recovered.words} ondone={() => (wordsDone = true)} />
  {/if}
{:else if view.phase === 'collected'}
  <Notice tone="ok">{$t('cloud_recover_collected')}</Notice>
  <div class="block">
    <h2>{$t('cloud_restore_new_secret')}</h2>
    <p class="text">{$t('cloud_restore_new_secret_hint')}</p>
  </div>
  <LockSecretFields bind:kind bind:password bind:confirm bind:hint disabled={store.busy} />
  {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
  <Actions>
    {#if vaultAway}
      <button class="btn btn-ghost" type="button" disabled={store.busy || !secretOk} onclick={() => finish(true)}>{$t('cloud_vault_skip')}</button>
    {/if}
    <button class="btn btn-primary" type="button" disabled={store.busy || !secretOk} onclick={() => finish()}>{$t('cloud_recover_finish')}</button>
  </Actions>
{:else if view.phase === 'expired'}
  <StateView tone="info" title={$t('cloud_recover_expired_title')}>
    <p>{$t('cloud_recover_expired')}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => store.cancel()}>{$t('cloud_link_again')}</button></Actions>
    {/snippet}
  </StateView>
{:else if view.phase === 'waiting'}
  <div class="block">
    <h2>{$t('cloud_recover_sent_title')}</h2>
    <p class="text">{$t('cloud_recover_say_code')}</p>
  </div>
  <p class="code mono">{view.code}</p>
  {#if view.expires_at !== null}
    <Countdown expiresAt={view.expires_at} total={view.created_at !== null ? view.expires_at - view.created_at : 1800} {now} label={(time) => $t('cloud_recover_time_left', { time })} />
  {/if}
  <div class="confirmed">
    <div class="pips" aria-hidden="true">
      {#each Array(view.k) as _, i (i)}<span class="pip" class:on={i < view.confirmed.length}></span>{/each}
    </div>
    <div class="who">
      <span class="count">{$t('cloud_recover_confirmed', { n: String(view.confirmed.length), k: String(view.k) })}</span>
      {#each view.confirmed as c (c.npub)}
        <code class="mono-chip" class:forged={c.forged}>{shortKey(c.npub)}</code>
      {/each}
    </div>
  </div>
  {#each view.confirmed.filter((c) => c.forged) as c (c.npub)}
    <Notice tone="danger">{$t('cloud_recover_forged', { who: shortKey(c.npub) })}</Notice>
  {/each}
  {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
  <Actions>
    <button class="btn btn-ghost" type="button" onclick={() => store.cancel()}>{$t('cloud_recover_cancel')}</button>
    {#if view.link}
      <button class="btn btn-primary" type="button" onclick={share}>
        <Icon name={copied ? 'check' : 'copy'} size={14} />{copied ? $t('cloud_recover_link_copied') : $t('cloud_recover_share_link')}
      </button>
    {/if}
  </Actions>
{:else}
  <p class="text">{$t('cloud_recover_intro')}</p>
  <form class="form" onsubmit={(e) => { e.preventDefault(); void store.start(npub); }}>
    <div class="form-group">
      <label for="cloud-recover-npub">{$t('cloud_recover_npub_label')}</label>
      <input id="cloud-recover-npub" class="mono" type="text" bind:value={npub} placeholder="npub1…" spellcheck="false" autocapitalize="off" autocomplete="off" disabled={store.busy} />
    </div>
    {#if view.phase === 'preparing'}
      <p class="wait"><span class="spinner" aria-hidden="true"></span>{$t('cloud_recover_preparing')}</p>
    {/if}
    {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
    <Actions>
      <a class="btn btn-ghost" href="/messenger">{$t('cloud_restore_back')}</a>
      <button class="btn btn-primary" type="submit" disabled={store.busy || !npub.trim()}>{$t('cloud_recover_go')}</button>
    </Actions>
  </form>
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-1); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  input::placeholder { font-family: var(--font-ui); }
  /* The code is said by voice: large, apart, never broken inside a group. */
  .code {
    margin: 0; padding: var(--sp-4); text-align: center; border-radius: var(--radius-md);
    background: var(--accent-tint); border: 1px solid var(--accent-tint-border); color: var(--text);
    font-size: var(--fs-xl); font-weight: var(--fw-bold); letter-spacing: 0.06em; white-space: nowrap;
    user-select: text; -webkit-user-select: text;
  }
  .confirmed { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md); background: var(--surface-2); border: 1px solid var(--border); }
  /* One dot for each person needed: unlike the line of the time above, these fill one by one. */
  .pips { display: flex; gap: var(--sp-2); }
  .pip { width: 14px; height: 14px; border-radius: 50%; background: var(--surface-3); border: 1px solid var(--border-2); }
  .pip.on { background: var(--success); border-color: var(--success); }
  .who { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; font-size: var(--fs-base); color: var(--text-body); }
  .count { font-weight: var(--fw-semibold); }
  .forged { color: var(--danger-text); }
  .wait { margin: 0; display: flex; align-items: center; gap: var(--sp-2); color: var(--text-2); font-size: var(--fs-base); }
  .spinner { width: 14px; height: 14px; flex-shrink: 0; }
</style>

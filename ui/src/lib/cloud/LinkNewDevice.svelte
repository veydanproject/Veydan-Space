<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Войти с другого устройства» (cloud-recovery.md §6, §11): the QR code, the 8 words and the 4 words to compare; the account comes from the other device and, when the lock of this device asks, its password opens it. -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { VAULT_UNREACHABLE } from '$lib/cloud/api';
  import { linkErrorKey, NewDeviceLink, qrDataUrl } from '$lib/cloud/link.svelte';
  import Actions from '$lib/cloud/Actions.svelte';
  import Countdown from '$lib/cloud/Countdown.svelte';
  import StateView from '$lib/cloud/StateView.svelte';
  import WordChips from '$lib/cloud/WordChips.svelte';

  const link = new NewDeviceLink();
  let password = $state('');
  let now = $state(Math.floor(Date.now() / 1000));
  /** How long the code lived when it was shown: the line of the countdown runs from here. */
  let total = $state(0);
  let timer: ReturnType<typeof setInterval> | null = null;

  const errorText = $derived.by(() => {
    const key = linkErrorKey(link.error);
    return key ? $t(key) : (link.error?.message ?? '');
  });

  async function begin() {
    await link.begin();
    now = Math.floor(Date.now() / 1000);
    total = link.shown ? Math.max(0, link.shown.expires_at - now) : 0;
  }

  onMount(() => {
    void begin();
    let ticks = 0;
    timer = setInterval(() => {
      now = Math.floor(Date.now() / 1000);
      // The clock moves every second, the relays are asked every other one.
      if (++ticks % 2 === 0) void link.poll();
    }, 1000);
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
    // A link left open ends: the code goes from the relays, T is wiped.
    if (link.phase !== 'done') void link.cancel();
  });

  async function signIn() {
    const secret = password;
    password = '';
    await link.receive(secret);
  }

  /** «Отмена» at the lock's password: the link ends and the screen is left, there is nothing to show here. */
  async function leave() {
    await link.cancel();
    await goto('/messenger');
  }

  function openChats() {
    // The messenger reads its identity anew on a fresh start of the page.
    window.location.assign('/messenger');
  }
</script>

{#if link.phase === 'done'}
  <StateView tone="ok" title={$t('cloud_link_done_title')}>
    <p>{$t('cloud_link_done_text')}</p>
    {#if link.done && !link.done.vault_joined}<p>{$t('cloud_link_done_vault')}</p>{/if}
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={openChats}>{$t('cloud_link_open_chats')}</button></Actions>
    {/snippet}
  </StateView>
{:else if link.phase === 'received' || link.phase === 'signing_in'}
  {#if link.needsPassword}
    <div class="block">
      <h2>{$t('cloud_link_received')}</h2>
      <p class="text">{$t('cloud_link_password_hint')}</p>
    </div>
    <form class="form" onsubmit={(e) => { e.preventDefault(); if (password) void signIn(); }}>
      <div class="form-group">
        <label for="cloud-link-lock">{$t('cloud_link_password')}</label>
        <input id="cloud-link-lock" type="password" bind:value={password} autocomplete="current-password" disabled={link.phase === 'signing_in'} />
      </div>
      {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
      <Actions>
        <button class="btn btn-ghost" type="button" onclick={leave}>{$t('cloud_link_cancel')}</button>
        <button class="btn btn-primary" type="submit" disabled={!password || link.phase === 'signing_in'}>{$t('cloud_link_sign_in')}</button>
      </Actions>
    </form>
  {:else if errorText}
    <StateView tone="danger" title={$t('cloud_link_failed_title')}>
      <p>{errorText}{#if link.error?.code === VAULT_UNREACHABLE && link.error.message} ({link.error.message}){/if}</p>
      {#snippet actions()}
        <Actions>
          {#if link.vaultAway}
            <!-- The account can still come without the storage: the link is kept. -->
            <button class="btn btn-ghost" type="button" onclick={leave}>{$t('cloud_link_cancel')}</button>
            <button class="btn btn-primary" type="button" onclick={() => link.receive(null, true)}>{$t('cloud_vault_skip')}</button>
          {:else}
            <button class="btn btn-primary" type="button" onclick={begin}>{$t('cloud_link_new_code')}</button>
          {/if}
        </Actions>
      {/snippet}
    </StateView>
  {:else}
    <p class="wait"><span class="spinner" aria-hidden="true"></span>{$t('cloud_link_signing_in')}</p>
  {/if}
{:else if link.phase === 'expired'}
  <StateView tone="info" title={$t('cloud_link_expired_title')}>
    <p>{$t('cloud_link_expired_text')}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={begin}>{$t('cloud_link_new_code')}</button></Actions>
    {/snippet}
  </StateView>
{:else if link.phase === 'failed' || link.phase === 'idle'}
  <StateView tone={errorText ? 'danger' : 'info'} title={errorText ? $t('cloud_link_failed_title') : $t('cloud_link_expired_title')}>
    {#if errorText}<p>{errorText}</p>{/if}
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={begin}>{$t('cloud_link_new_code')}</button></Actions>
    {/snippet}
  </StateView>
{:else if link.shown}
  <p class="text">{$t('cloud_link_new_hint')}</p>
  <!-- Drawn in Rust by the generator of group links, shown as an image: nothing inside it can run. -->
  <img class="qr" src={qrDataUrl(link.shown.qr_svg)} alt={$t('cloud_link_qr_alt')} />
  <div class="block">
    <span class="section-label">{$t('cloud_link_words')}</span>
    <WordChips words={link.shown.words} numbered />
  </div>
  <div class="check">
    <span class="section-label">{$t('cloud_link_fingerprint')}</span>
    <WordChips words={link.shown.fingerprint} large />
    <p class="text small">{$t('cloud_link_fingerprint_hint')}</p>
  </div>
  <div class="foot">
    <p class="wait"><span class="spinner" aria-hidden="true"></span>{$t('cloud_link_waiting')}</p>
    <Countdown expiresAt={link.shown.expires_at} {total} {now} label={(time) => $t('cloud_link_time_left', { time })} />
  </div>
  {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
{:else}
  <p class="wait"><span class="spinner" aria-hidden="true"></span>{$t('cloud_link_preparing')}</p>
{/if}

<style>
  .block, .check, .foot { display: flex; flex-direction: column; gap: var(--sp-2); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .text.small { font-size: var(--fs-sm); }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  /* A QR code is black on white in either theme: a camera reads nothing else as well. */
  .qr { align-self: center; width: min(232px, 100%); height: auto; background: #fff; border-radius: var(--radius-lg); padding: var(--sp-2); image-rendering: pixelated; }
  .check { padding: var(--sp-4); border-radius: var(--radius-md); background: var(--surface-2); border: 1px solid var(--border); }
  .foot { padding-top: var(--sp-2); }
  .wait { margin: 0; display: flex; align-items: center; gap: var(--sp-2); color: var(--text-2); font-size: var(--fs-base); }
  .spinner { width: 14px; height: 14px; border-width: 2px; flex-shrink: 0; }
</style>

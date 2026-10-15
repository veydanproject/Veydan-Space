<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Добавить устройство» (cloud-recovery.md §6, §11): the QR code or the 8 words of the new device, then its 4 words to compare and the lock's password; «Совпадает» sends the account, «Не совпадает» sends nothing. -->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { linkErrorKey, OldDeviceLink } from '$lib/cloud/link.svelte';
  import Actions from '$lib/cloud/Actions.svelte';
  import QrScanner from '$lib/core/ui/QrScanner.svelte';
  import StateView from '$lib/cloud/StateView.svelte';
  import Steps from '$lib/cloud/Steps.svelte';
  import WordChips from '$lib/cloud/WordChips.svelte';

  const link = new OldDeviceLink();
  let input = $state('');
  let password = $state('');
  let scanning = $state(false);

  const errorText = $derived.by(() => {
    const key = linkErrorKey(link.error);
    return key ? $t(key) : (link.error?.message ?? '');
  });
  /** The lock guards the account: its password is asked before it goes. */
  const needsPassword = $derived(appLock.status.enabled);
  const comparing = $derived(link.phase === 'compare' || link.phase === 'sending');
  const steps = $derived([$t('cloud_link_step_code'), $t('cloud_link_step_compare'), $t('cloud_link_step_sent')]);

  async function next() {
    await link.resolve(input);
  }

  /** The camera read a code: it goes where the typed words go. */
  async function scanned(content: string) {
    scanning = false;
    input = content;
    await link.resolve(content);
  }

  async function approve() {
    const secret = password;
    password = '';
    await link.approve(needsPassword ? secret : null);
    if (link.phase === 'sent') input = '';
  }

  onDestroy(() => {
    // A link found and not sent ends with the page.
    if (link.phase === 'compare') void link.reset();
  });
</script>

{#if scanning}
  <QrScanner
    title={$t('cloud_scan_title')}
    hint={$t('cloud_scan_hint')}
    manual={$t('cloud_scan_manual')}
    words={{
      starting: $t('cloud_scan_starting'),
      denied: $t('cloud_scan_denied'),
      failed: $t('cloud_scan_failed'),
      retry: $t('cloud_scan_retry'),
      openSettings: $t('cloud_scan_open_settings'),
    }}
    onresult={scanned}
    onclose={() => (scanning = false)}
  />
{/if}

{#if link.phase === 'sent'}
  <StateView tone="ok" title={$t('cloud_link_sent_title')}>
    <p>{$t('cloud_link_sent_text')}</p>
    {#snippet actions()}
      <Actions><a class="btn btn-primary" href="/cloud">{$t('cloud_done')}</a></Actions>
    {/snippet}
  </StateView>
{:else if link.phase === 'mismatch'}
  <StateView tone="warn" title={$t('cloud_link_mismatch_title')}>
    <p>{$t('cloud_link_mismatch_text')}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => link.reset()}>{$t('cloud_link_again')}</button></Actions>
    {/snippet}
  </StateView>
{:else}
  <Steps {steps} current={comparing ? 1 : 0} />
  {#if comparing && link.target}
    <div class="block">
      <h2>{$t('cloud_link_compare')}</h2>
      <p class="text">{$t('cloud_link_compare_hint')}</p>
    </div>
    <WordChips words={link.target.fingerprint} large />
    {#if needsPassword}
      <div class="form-group">
        <label for="cloud-link-password">{$t('cloud_link_password')}</label>
        <input id="cloud-link-password" type="password" bind:value={password} autocomplete="current-password" disabled={link.phase === 'sending'} />
      </div>
    {/if}
    {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
    <Actions>
      <button class="btn btn-ghost" type="button" disabled={link.phase === 'sending'} onclick={() => link.mismatch()}>
        {$t('cloud_link_no_match')}
      </button>
      <button class="btn btn-primary" type="button" disabled={link.phase === 'sending' || (needsPassword && !password)} onclick={approve}>
        {link.phase === 'sending' ? $t('cloud_link_sending') : $t('cloud_link_match')}
      </button>
    </Actions>
  {:else}
    <div class="block">
      <h2>{$t('cloud_link_code_title')}</h2>
      <p class="text">{$t('cloud_link_add_hint')}</p>
    </div>
    {#if isMobile}
      <button class="m-btn-grad" type="button" disabled={link.phase === 'resolving'} onclick={() => (scanning = true)}>
        <Icon name="qr-code" size={18} />{$t('cloud_link_scan')}
      </button>
      <div class="or"><span>{$t('cloud_or')}</span></div>
    {/if}
    <form class="form" onsubmit={(e) => { e.preventDefault(); void next(); }}>
      <div class="form-group">
        <label for="cloud-link-words">{$t('cloud_link_input_label')}</label>
        <textarea id="cloud-link-words" class="mono" rows="3" bind:value={input} spellcheck="false" autocomplete="off" autocapitalize="off" disabled={link.phase === 'resolving'}></textarea>
      </div>
      {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
      <Actions>
        <button class="btn" class:btn-primary={!isMobile} class:btn-ghost={isMobile} type="submit" disabled={link.phase === 'resolving' || !input.trim()}>
          {link.phase === 'resolving' ? $t('cloud_link_looking') : $t('cloud_link_next')}
        </button>
      </Actions>
    </form>
  {/if}
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-1); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  textarea { resize: vertical; line-height: 1.5; }
  .or { display: flex; align-items: center; gap: var(--sp-3); color: var(--text-3); font-size: var(--fs-sm); }
  .or::before, .or::after { content: ''; flex: 1; height: 1px; background: var(--border-2); }
</style>

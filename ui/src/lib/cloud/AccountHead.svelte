<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The head of the page «Account»: who the account is — the name of the profile and the address of the key, a tap away from the clipboard — or, on a device without one, the way to make it. The owner of the key answers (core/account). -->
<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { accountName, accountSource, shortNpub, type AccountProfile } from '$lib/core/account';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';

  let { profile }: { profile: AccountProfile | null } = $props();

  let copied = $state(false);
  const letter = $derived(profile ? ([...accountName(profile).trim()][0] ?? '?').toUpperCase() : '');

  async function copy() {
    if (!profile) return;
    try {
      await navigator.clipboard.writeText(profile.npub);
      copiedToast();
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // No clipboard here: the address stays on the screen.
    }
  }
</script>

{#if profile}
  <header class="head" class:m={isMobile}>
    <span class="ava" aria-hidden="true">{letter}</span>
    <strong>{accountName(profile)}</strong>
    <button type="button" class="npub" onclick={copy} aria-label={$t('cloud_account_copy')} title={$t('cloud_account_copy')}>
      <span class="mono">{shortNpub(profile.npub)}</span>
      <Icon name={copied ? 'check' : 'copy'} size={13} />
    </button>
  </header>
{:else}
  <div class="{isMobile ? 'm-card' : 'card'} none">
    <span class="ava ghost" aria-hidden="true"><Icon name="user" size={24} /></span>
    <strong>{$t('cloud_account_none')}</strong>
    <p>{$t('cloud_account_invite')}</p>
    <a class="btn btn-primary" href={accountSource()?.createHref ?? '/'}>{$t('cloud_account_create')}</a>
  </div>
{/if}

<style>
  .head { display: flex; flex-direction: column; align-items: center; gap: var(--sp-1); padding: var(--sp-2) 0 var(--sp-3); }
  .ava {
    display: inline-flex; align-items: center; justify-content: center; width: 68px; height: 68px; border-radius: 50%; flex-shrink: 0;
    background: var(--accent-grad); color: #fff; font-size: var(--fs-2xl); font-weight: 800; margin-bottom: var(--sp-2);
  }
  .ava.ghost { background: var(--accent-tint); color: var(--accent-text); }
  strong { font-size: var(--fs-lg, 19px); font-weight: var(--fw-bold); text-align: center; overflow-wrap: anywhere; }
  .npub {
    display: inline-flex; align-items: center; gap: 6px; min-height: 32px; padding: 0 var(--sp-2); border: 0; border-radius: var(--radius-sm);
    background: none; color: var(--text-2); font-size: var(--fs-sm); cursor: pointer;
  }
  .npub:hover { color: var(--text); }
  .none { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); padding: var(--sp-6) var(--sp-4); text-align: center; }
  .none p { margin: 0 0 var(--sp-2); color: var(--text-2); font-size: var(--fs-base); }
</style>

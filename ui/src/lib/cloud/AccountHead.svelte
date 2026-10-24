<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The head of the page «Account»: who the account is — the name of the profile and the address of the key, a tap away from the clipboard — in one line with what the page puts at its end (the refresh), or, on a device without one, the way to make it. The owner of the key answers (core/account). -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { accountName, accountSource, shortNpub, type AccountProfile } from '$lib/core/account';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';

  let { profile, actions }: { profile: AccountProfile | null; actions?: Snippet } = $props();

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
    <span class="who">
      <strong>{accountName(profile)}</strong>
      <button type="button" class="npub" onclick={copy} aria-label={$t('cloud_account_copy')} title={$t('cloud_account_copy')}>
        <span class="mono">{shortNpub(profile.npub)}</span>
        <Icon name={copied ? 'check' : 'copy'} size={14} />
      </button>
    </span>
    {#if actions}{@render actions()}{/if}
  </header>
{:else}
  <div class="{isMobile ? 'm-card' : 'card'} none" class:m={isMobile}>
    <span class="ava ghost" aria-hidden="true"><Icon name="user" size={36} /></span>
    <strong>{$t('cloud_account_none')}</strong>
    <p>{$t('cloud_account_invite')}</p>
    <a class="btn btn-primary" href={accountSource()?.createHref ?? '/'}>{$t('cloud_account_create')}</a>
  </div>
{/if}

<style>
  .head { display: flex; align-items: center; gap: var(--sp-4); padding: var(--sp-1) 0; }
  .ava {
    display: inline-flex; align-items: center; justify-content: center; width: 56px; height: 56px; border-radius: 50%; flex-shrink: 0;
    background: var(--accent-grad); color: #fff; font-size: var(--fs-xl); font-weight: var(--fw-extrabold);
  }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; }
  .who strong { max-width: 100%; font-size: var(--fs-xl); font-weight: var(--fw-bold); line-height: 1.3; overflow-wrap: anywhere; }
  .npub {
    display: inline-flex; align-items: center; gap: 6px; min-height: var(--control-h-sm); margin-left: calc(-1 * var(--sp-2)); padding: 0 var(--sp-2);
    border: 0; border-radius: var(--radius-sm); background: none; color: var(--text-2); font-size: var(--fs-sm); cursor: pointer;
  }
  .m .npub { min-height: var(--control-h-lg); }
  .npub:hover { color: var(--text); }
  /* No account: the invitation stands alone, larger and centred, its one action under it. */
  .none { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); padding: var(--sp-12) var(--sp-8); text-align: center; }
  .none.m { padding: var(--sp-10) var(--sp-5); }
  .ava.ghost { width: 88px; height: 88px; margin-bottom: var(--sp-2); background: var(--accent-tint); color: var(--accent-text); box-shadow: 0 0 0 var(--sp-2) var(--accent-tint); }
  .none strong { font-size: var(--fs-xl); font-weight: var(--fw-bold); }
  .none p { margin: 0 0 var(--sp-3); max-width: 420px; color: var(--text-2); font-size: var(--fs-body); }
  .none .btn { min-width: 200px; min-height: var(--control-h-lg); }
</style>

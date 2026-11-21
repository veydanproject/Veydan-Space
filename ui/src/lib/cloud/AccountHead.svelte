<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The head of the page «Account»: who the account is — the name of the profile and the address of the key, a tap away from the clipboard — in one line with what the page puts at its end (the refresh), or, on a device without one, the way to make it. The owner of the key answers (core/account). -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '$lib/cloud/Avatar.svelte';
  import { accountName, accountSource, shortNpub, type AccountProfile } from '$lib/core/account';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';

  let { profile, actions }: { profile: AccountProfile | null; actions?: Snippet } = $props();

  async function copy() {
    if (!profile) return;
    try {
      await navigator.clipboard.writeText(profile.npub);
      copiedToast();
    } catch {
      // No clipboard here: the address stays on the screen.
    }
  }
</script>

{#if profile}
  <header class="head" class:m={isMobile}>
    <Avatar name={accountName(profile)} size={56} />
    <span class="who">
      <strong>{accountName(profile)}</strong>
      <button type="button" class="npub" onclick={copy} aria-label={$t('cloud_account_copy')} title={$t('cloud_account_copy')}>
        <span class="mono">{shortNpub(profile.npub)}</span>
        <Icon name="copy" size={14} />
      </button>
    </span>
    {#if actions}{@render actions()}{/if}
  </header>
{:else}
  <div class="{isMobile ? 'm-card' : 'card'} empty-state">
    <span class="empty-icon" aria-hidden="true"><Icon name="user" size={22} /></span>
    <p>{$t('cloud_account_none')}</p>
    <span>{$t('cloud_account_invite')}</span>
    <a class="btn btn-primary" href={accountSource()?.createHref ?? '/'}>{$t('common_create')}</a>
  </div>
{/if}

<style>
  .head { display: flex; align-items: center; gap: var(--sp-4); padding: var(--sp-1) 0; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; }
  .who strong { max-width: 100%; font-size: var(--fs-xl); font-weight: var(--fw-bold); line-height: var(--lh-snug); overflow-wrap: anywhere; }
  .npub {
    display: inline-flex; align-items: center; gap: var(--sp-1-5); min-height: var(--control-h-sm); margin-left: calc(-1 * var(--sp-2)); padding: 0 var(--sp-2);
    border: 0; border-radius: var(--radius-sm); background: none; color: var(--text-2); font-size: var(--fs-sm); cursor: pointer;
  }
  .m .npub { min-height: var(--control-h-lg); }
  .npub:hover { color: var(--text); }
</style>

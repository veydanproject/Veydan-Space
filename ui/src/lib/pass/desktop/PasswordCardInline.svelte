<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The pass module's part of a note's context card for a password: its tags,
  the codes of its TOTP entries, and the row of actions (open, copy username,
  copy password through the lock). The notes module renders the card's head
  and passes its own controls as children.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { formatError, hasErrorCode } from '$lib/core/utils';
  import { appLock } from '$lib/core/lock/store.svelte';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';
  import PassTags from '$lib/pass/desktop/PassTags.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { passEntities, totpName } from '$lib/pass/entities';

  let { id, children }: { id: string; children?: Snippet } = $props();

  const def = passEntities.find((d) => d.kind === 'password')!;
  const entry = $derived(passwordStore.list.find((e) => e.id === id));
  const totpIds = $derived(entry?.totp_ids ?? []);
  let codesOpen = $state(false);
  const shownTotp = $derived(codesOpen ? totpIds : totpIds.slice(0, 1));
  const totpPeriod = (tid: string) => totpStore.list.find((e) => e.id === tid)?.period ?? 30;

  let busy = $state(false);
  let copied = $state('');
  let copyTimer: ReturnType<typeof setTimeout> | undefined;
  let error = $state<string | null>(null);
  let unlockOpen = $state(false);
  let pendingCopy = $state(false);

  function markCopied(key: string) {
    copied = '';
    requestAnimationFrame(() => (copied = key));
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = ''), 1600);
  }

  async function run(actionId: string) {
    const action = def.actions.find((a) => a.id === actionId);
    if (!action) return;
    if (actionId === 'copy-password' && appLock.locked) {
      pendingCopy = true;
      unlockOpen = true;
      return;
    }
    busy = true;
    error = null;
    try {
      await action.run(id);
      markCopied(actionId);
    } catch (e) {
      if (actionId === 'copy-password' && (hasErrorCode(e, 'vault_locked') || hasErrorCode(e, 'vault_mismatch') || hasErrorCode(e, 'decrypt_failed'))) {
        pendingCopy = true;
        unlockOpen = true;
      } else {
        error = formatError(e);
      }
    } finally {
      busy = false;
    }
  }
</script>

{#if entry}
  <PassTags tags={entry.tags} />
  {#each shownTotp as tid (tid)}
    <div class="pw-code">
      <span class="pw-code-name">{totpName(tid)}</span>
      <TotpLiveCode entryId={tid} period={totpPeriod(tid)} copiedLabel={$t('totp_copy')} compact />
    </div>
  {/each}
{/if}
{@render children?.()}
<div class="foot">
  {#if totpIds.length > 1}
    <button type="button" class="more" onclick={() => (codesOpen = !codesOpen)}>
      {codesOpen ? $t('ctx_totp_less') : $t('ctx_totp_more', { n: String(totpIds.length - 1) })}
    </button>
  {/if}
  <div class="pw-actions">
    <button type="button" class="icon-btn" title={$t('ctx_action_open')} onclick={() => def.open?.(id)}>
      <Icon name="external-link" size={13} />
    </button>
    {#if entry?.username}
      <button
        type="button"
        class="icon-btn"
        class:success={copied === 'copy-username'}
        class:pop={copied === 'copy-username'}
        title={$t('ctx_action_copy_username')}
        disabled={busy}
        onclick={() => run('copy-username')}
      >
        <Icon name={copied === 'copy-username' ? 'check' : 'user'} size={13} />
      </button>
    {/if}
    <button
      type="button"
      class="icon-btn"
      class:success={copied === 'copy-password'}
      class:pop={copied === 'copy-password'}
      title={appLock.locked ? $t('lock_title') : $t('ctx_action_copy_password')}
      disabled={busy}
      onclick={() => run('copy-password')}
    >
      <Icon name={copied === 'copy-password' ? 'check' : appLock.locked ? 'lock' : 'copy'} size={13} />
    </button>
  </div>
</div>
{#if error}
  <div class="error">{error}</div>
{/if}
<VaultUnlock
  bind:open={unlockOpen}
  onunlocked={() => {
    if (!pendingCopy) return;
    pendingCopy = false;
    void run('copy-password');
  }}
/>

<style>
  .foot {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    width: 100%;
    margin-top: auto;
  }
  .error { font-size: var(--fs-xs); color: var(--danger-text); }
  .pw-actions { display: flex; gap: 0.25rem; margin-left: auto; }
  .icon-btn.pop { animation: icon-pop 0.35s ease; }
  @keyframes icon-pop {
    0% { transform: scale(1); }
    40% { transform: scale(1.12); }
    100% { transform: scale(1); }
  }
  .pw-code {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    width: 100%;
    min-width: 0;
  }
  .pw-code-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-2xs);
    color: var(--text-2);
  }
  .pw-code :global(.live.compact) { min-height: 0; }
  .more {
    background: none;
    border: 0;
    padding: 0;
    color: var(--text-3);
    font-size: var(--fs-2xs);
    cursor: pointer;
  }
  .more:hover { color: var(--text); }
</style>

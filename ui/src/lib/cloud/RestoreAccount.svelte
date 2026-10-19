<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Восстановить аккаунт» → «У меня есть 12 слов» (cloud-recovery.md §7, §11):
  the words; the copy read before anything is written says whether the
  account has a lock, and only then its PIN or password is asked — the one
  the person remembers, or a new one; the six steps with their progress,
  then the messenger. A storage of the user's own that is not
  within reach is said, with the way to restore without it. Opened from the
  first screen of the messenger (/cloud/restore).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Actions from '$lib/cloud/Actions.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import StateView from '$lib/cloud/StateView.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { looksLikeWords } from '$lib/core/lock/words';
  import RestoreSecret from '$lib/cloud/RestoreSecret.svelte';
  import { emptySecretState, restoreSecretOf } from '$lib/cloud/restoreSecret';
  import {
    asCloudError,
    cloudApi,
    NO_SECRET,
    RESTORE_EVENT,
    VAULT_UNREACHABLE,
    type CloudError,
    type CloudRestored,
    type CloudRestorePhase,
  } from '$lib/cloud/api';

  let words = $state('');
  /** The account has a lock: its secret is asked (after the look at the copy). */
  let needsSecret = $state(false);
  let secret = $state(emptySecretState());
  let busy = $state(false);
  let phase = $state<CloudRestorePhase | null>(null);
  let error = $state('');
  /** The last refusal was `vault_unreachable`: the restore without the storage is offered. */
  let vaultAway = $state(false);
  let restored = $state<CloudRestored | null>(null);

  const wordsOk = $derived(looksLikeWords(words));
  const wordCount = $derived(words.trim().split(/\s+/).filter(Boolean).length);
  const typed = $derived(restoreSecretOf(secret));
  const ready = $derived(wordsOk && (!needsSecret || typed !== null));

  onMount(() => {
    let stop: (() => void) | null = null;
    void (async () => {
      if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
      const { listen } = await import('@tauri-apps/api/event');
      stop = await listen<{ phase: CloudRestorePhase }>(RESTORE_EVENT, (e) => (phase = e.payload.phase));
    })();
    return () => stop?.();
  });

  /** The words of a refusal of the restore, by its code. */
  function refusalText(e: CloudError, tr: (key: TranslationKey) => string): string {
    switch (e.code) {
      case 'not_found':
        return tr('cloud_restore_not_found');
      case 'bad_words':
        return tr('cloud_restore_bad_words');
      case 'escrow_invalid':
        return tr('cloud_restore_invalid');
      case 'identity_exists':
        return tr('cloud_restore_identity_exists');
      case 'not_clean':
        return tr('cloud_restore_not_clean');
      case VAULT_UNREACHABLE:
        return e.message ? `${tr('cloud_vault_unreachable')} (${e.message})` : tr('cloud_vault_unreachable');
      case 'wrong_password':
        return tr('cloud_restore_wrong_old');
      case 'lock_secret_needed':
        return tr('cloud_restore_lock_needed');
      default:
        return e.message ? `${e.code}: ${e.message}` : e.code;
    }
  }

  const phaseKey = (p: CloudRestorePhase) => `cloud_restore_phase_${p}` as const;

  /**
   * «Далее» / «Восстановить»: the words alone first — the copy is read and
   * says whether the account has a lock; without one the restore goes on
   * at once, with one its secret is asked. Then the restore.
   */
  async function next(withoutVault = false) {
    error = '';
    vaultAway = false;
    if (!needsSecret) {
      busy = true;
      phase = 'reading';
      try {
        needsSecret = (await cloudApi.restorePeek(words.trim())).lock;
      } catch (e) {
        fail(e);
        return;
      } finally {
        busy = false;
      }
      phase = null;
      if (needsSecret) return;
    }
    await restore(withoutVault);
  }

  async function restore(withoutVault: boolean) {
    const given = needsSecret ? typed : NO_SECRET;
    if (!given) return;
    busy = true;
    phase = 'reading';
    try {
      restored = await cloudApi.restoreWithWords(words.trim(), given, withoutVault);
      words = '';
      secret = emptySecretState();
      await appLock.refresh();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  function fail(e: unknown) {
    const refusal = asCloudError(e);
    vaultAway = refusal.code === VAULT_UNREACHABLE;
    // The copy fell behind a change of the lock: the secret is asked now.
    if (refusal.code === 'lock_secret_needed') needsSecret = true;
    error = refusalText(refusal, (key) => $t(key));
    phase = null;
  }

  /** Back to the words: what was typed of the secret goes. */
  function back() {
    needsSecret = false;
    secret = emptySecretState();
    error = '';
    vaultAway = false;
  }
</script>

{#if restored}
  <StateView tone="ok" title={$t('cloud_restore_done')}>
    <p>
      {#if restored.lock === 'kept'}{$t('cloud_restore_lock_kept')}{:else if restored.lock === 'opened'}{$t('cloud_restore_lock_opened')}{:else if restored.lock === 'reinstalled'}{$t('cloud_restore_lock_reinstalled')}{:else}{$t('cloud_restore_lock_none')}{/if}
    </p>
    {#snippet actions()}
      {#if restored && !restored.vault_joined}<p>{$t('cloud_link_done_vault')}</p>{/if}
      {#if restored?.escrow_refreshed === false}
        <Notice tone="warn" role="alert">{$t('cloud_restore_escrow_stale')}</Notice>
      {/if}
      <Actions><button class="btn btn-primary" type="button" onclick={() => goto('/messenger')}>{$t('cloud_restore_open')}</button></Actions>
    {/snippet}
  </StateView>
{:else}
  <div class="block">
    <h2>{$t('cloud_restore_words_entry')}</h2>
    <p class="text">{$t('cloud_restore_words_hint')}</p>
  </div>
  <div class="form-group">
    <div class="label-row">
      <label for="cloud-restore-words">{$t('cloud_restore_words_label')}</label>
      <span class="count" class:ok={wordsOk}>{$t('cloud_restore_words_count', { n: String(wordCount) })}</span>
    </div>
    <textarea id="cloud-restore-words" class="mono" rows="3" bind:value={words} spellcheck="false" autocapitalize="off" autocomplete="off" disabled={busy || needsSecret}></textarea>
  </div>
  {#if needsSecret}
    <RestoreSecret bind:state={secret} disabled={busy} />
  {/if}
  {#if busy && phase}
    <p class="wait"><span class="spinner" aria-hidden="true"></span>{$t(phaseKey(phase))}</p>
  {/if}
  {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
  <Actions>
    {#if needsSecret}
      <button class="btn btn-ghost" type="button" disabled={busy} onclick={back}>{$t('cloud_restore_back')}</button>
    {:else}
      <a class="btn btn-ghost" href="/messenger">{$t('cloud_restore_back')}</a>
    {/if}
    {#if vaultAway}
      <button class="btn btn-ghost" type="button" disabled={busy || !ready} onclick={() => next(true)}>{$t('cloud_vault_skip')}</button>
    {/if}
    <button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={() => next()}>{needsSecret ? $t('cloud_restore_go') : $t('cloud_restore_next')}</button>
  </Actions>
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-1); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .label-row { display: flex; align-items: baseline; justify-content: space-between; gap: var(--sp-2); }
  .count { font-size: var(--fs-xs); color: var(--text-3); }
  .count.ok { color: var(--success-text); }
  textarea { resize: vertical; line-height: 1.5; }
  .wait { margin: 0; display: flex; align-items: center; gap: var(--sp-2); color: var(--text-2); font-size: var(--fs-base); }
  .spinner { width: 14px; height: 14px; flex-shrink: 0; }
</style>

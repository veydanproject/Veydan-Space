<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Delete account» (vcloud-plan.md 4.6): always in sight, what goes away said first, then the lock, the word DELETE and the choice of the identity and data of this device; the shares of the trusted people once they exist (stage C5, `SHARES_REVOCABLE`). -->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { t } from '$lib/core/i18n';
  import LockSecretInput from '$lib/core/lock/LockSecretInput.svelte';
  import { api } from '$lib/core/api';
  import { deleteReady, emptyDeleteForm, errorKeyOf, SHARES_REVOCABLE, type DeleteForm } from '$lib/cloud/account';
  import { cloudPage } from '$lib/cloud/status.svelte';
  import { isMobile } from '$lib/core/platform';
  import Actions from '$lib/cloud/Actions.svelte';
  import NavRow from '$lib/core/ui/NavRow.svelte';
  import RowList from '$lib/cloud/RowList.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';

  let { startOpen = false }: { startOpen?: boolean } = $props();

  let open = $state(false);
  let form = $state<DeleteForm>(emptyDeleteForm(false));
  let error = $state('');
  /** The deletion itself is under way: other commands of the page do not relabel the button, and it is not sent twice. */
  let deleting = $state(false);
  const ready = $derived(deleteReady(form));
  let formEl = $state<HTMLFormElement>();

  onMount(() => {
    if (startOpen) void show();
  });

  async function show() {
    let lockOn = false;
    try {
      lockOn = (await api.lock.status()).enabled;
    } catch {
      // No lock in this app: nothing to pass.
    }
    form = emptyDeleteForm(lockOn);
    error = '';
    open = true;
    // A phone: the five points above are long; the form and its button come into sight.
    if (isMobile) {
      await tick();
      formEl?.scrollIntoView({ block: 'start', behavior: 'smooth' });
    }
  }

  async function confirm() {
    if (deleting) return;
    error = '';
    deleting = true;
    try {
      const receipt = await cloudPage.deleteAccount(form);
      if (receipt) {
        open = false;
        return;
      }
      const e = cloudPage.error;
      if (e) {
        const key = errorKeyOf(e.code);
        error = key ? $t(key) : e.message || e.code;
        // Said here, by the form: the page does not repeat it under itself.
        cloudPage.error = null;
      }
    } finally {
      deleting = false;
    }
  }
</script>

<RowList id="delete">
  {#if !open}
    <NavRow variant="list" icon="trash-2" tone="bad" danger title={$t('cloud_delete_title')} sub={$t('cloud_delete_sub')} onclick={show} />
  {:else}
    <div class="zone" class:m={isMobile}>
      <h2>{$t('cloud_delete_title')}</h2>
      <p class="text">{$t('cloud_delete_intro')}</p>
      <ul class="what">
        <li>{$t('cloud_delete_what_files')}</li>
        <li>{$t('cloud_delete_what_recovery')}</li>
        <li>{$t('cloud_delete_what_media')}</li>
        <li>{$t('cloud_delete_what_push')}</li>
        <li>{$t('cloud_delete_what_plan')}</li>
      </ul>
      <div class="notes">
        <p>{$t('cloud_delete_relays')}</p>
        <p>{$t('cloud_delete_payments')}</p>
      </div>
      <form class="form" bind:this={formEl} onsubmit={(e) => { e.preventDefault(); if (ready) void confirm(); }}>
        {#if form.lockOn}
          <LockSecretInput id="cloud-delete-lock" label={$t('cloud_delete_lock')} bind:value={form.secret} />
        {/if}
        <div class="form-group">
          <label for="cloud-delete-word">{$t('cloud_delete_word')}</label>
          <input id="cloud-delete-word" class="mono" type="text" bind:value={form.word} autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="DELETE" />
        </div>
        <label class="check">
          <input type="checkbox" bind:checked={form.wipeDevice} />
          <span>{$t('cloud_delete_wipe')}</span>
        </label>
        {#if SHARES_REVOCABLE}
          <label class="check">
            <input type="checkbox" bind:checked={form.revokeShares} />
            <span>{$t('cloud_delete_revoke')}</span>
          </label>
        {/if}
        {#if error}<Notice tone="danger">{error}</Notice>{/if}
        {#snippet buttons()}
          <button class="btn btn-ghost" type="button" disabled={deleting} onclick={() => (open = false)}>
            {$t('common_cancel')}
          </button>
          <button class="btn btn-danger" type="submit" disabled={!ready || deleting || cloudPage.busy}>
            {deleting ? $t('cloud_delete_running') : $t('cloud_delete_confirm')}
          </button>
        {/snippet}
        {#if isMobile}
          <!-- A phone: the common foot, pinned to the bottom of the screen while the form is in sight. -->
          <div class="page-foot">{@render buttons()}</div>
        {:else}
          <Actions>{@render buttons()}</Actions>
        {/if}
      </form>
    </div>
  {/if}
</RowList>

<style>
  .zone { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-5); }
  .zone.m { padding: var(--sp-4); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--danger-text); }
  .text { margin: 0; font-size: var(--fs-base); color: var(--text); line-height: var(--lh-base); }
  .what { margin: 0; padding-left: var(--sp-5); font-size: var(--fs-base); color: var(--text-body); line-height: var(--lh-base); }
  .notes { display: flex; flex-direction: column; gap: var(--sp-1); }
  .notes p { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: var(--lh-snug); }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); padding-top: var(--sp-3); border-top: 1px solid var(--border); }
  .check { display: flex; align-items: flex-start; gap: var(--sp-3); font-size: var(--fs-base); line-height: var(--lh-snug); cursor: pointer; color: var(--text-body); }
  .check input { margin-top: 1px; }
  /* The foot spans the card and sticks to the screen's bottom while the zone scrolls; the list clips
     without being a scroller, so the sticking holds. */
  :global(.m-list:has(> .zone.m)) { overflow: clip; }
  .zone.m .page-foot {
    position: sticky;
    bottom: 0;
    margin: 0 calc(-1 * var(--sp-4)) calc(-1 * var(--sp-4));
    padding-bottom: var(--sp-3);
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Ввести ключ активации» (vcloud-plan.md 4.3, cloud-api.md 4.2): 20 characters of Crockford base32, grouped as they are typed and read the way the server reads them, so a key dictated with O and I still goes. A dialog on a computer, a sheet on a phone. -->
<script lang="ts">
  import { t, locale } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import type { CloudError } from '$lib/cloud/api';
  import { dateOf, errorKeyOf, formatKeyInput, normalizeKey } from '$lib/cloud/account';
  import StateView from '$lib/cloud/StateView.svelte';

  let {
    open = $bindable(false),
    busy = false,
    /** The plan the key was asked for, by its name: said in the title. */
    planName = '',
    /** What the account became after the key: its plan, the day it is paid until, and the plan that waits for that day. */
    result = null,
    onRedeem,
  }: {
    open?: boolean;
    busy?: boolean;
    planName?: string;
    result?: { plan: string; until: number | null; next?: { plan: string; since: number } | null } | null;
    onRedeem: (key: string) => Promise<CloudError | null>;
  } = $props();

  let key = $state('');
  let error = $state('');
  let done = $state(false);
  /** A key is on its way to the server: it is not sent twice. */
  let sending = $state(false);
  const normalized = $derived(normalizeKey(key));

  // A dialog opened again starts clean.
  $effect(() => {
    if (open) {
      key = '';
      error = '';
      done = false;
    }
  });

  async function apply() {
    if (sending) return;
    error = '';
    if (!normalized) {
      error = $t('cloud_err_key_malformed');
      return;
    }
    sending = true;
    try {
      const failed = await onRedeem(normalized);
      if (failed) {
        const k = errorKeyOf(failed.code);
        error = k ? $t(k) : failed.message || failed.code;
        return;
      }
      done = true;
    } finally {
      sending = false;
    }
  }

  /** The key grouped as it is typed, the caret kept after the character it stood after: a fix in the middle stays in the middle. */
  function typed(field: HTMLInputElement) {
    const before = field.value.slice(0, field.selectionStart ?? field.value.length).replace(/[^0-9a-z]/gi, '').length;
    key = formatKeyInput(field.value);
    field.value = key;
    let at = 0;
    for (let seen = 0; at < key.length && seen < before; at++) if (key[at] !== '-') seen++;
    field.setSelectionRange(at, at);
    error = '';
  }
</script>

<Dialog bind:open title={done ? '' : planName ? $t('cloud_key_title_for', { plan: planName }) : $t('cloud_key_enter')}>
  {#if done}
    <StateView tone="ok" title={$t('cloud_key_done')}>
      {#if result}
        <p>
          {result.until ? $t('cloud_key_done_until', { plan: result.plan, date: dateOf(result.until, $locale) }) : $t('cloud_key_done_plan', { plan: result.plan })}
        </p>
        {#if result.next}
          <p>{$t('cloud_next_plan', { plan: result.next.plan, date: dateOf(result.next.since, $locale) })}</p>
        {/if}
      {/if}
    </StateView>
  {:else}
    <form id="cloud-redeem" class="form" onsubmit={(e) => { e.preventDefault(); void apply(); }}>
      <p class="text">{$t('cloud_key_hint')}</p>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="key mono"
        type="text"
        value={key}
        oninput={(e) => typed(e.currentTarget)}
        placeholder="XXXX-XXXX-XXXX-XXXX-XXXX"
        autocomplete="off"
        autocapitalize="characters"
        spellcheck="false"
        autofocus
        aria-label={$t('cloud_key_enter')}
        aria-invalid={error ? 'true' : undefined}
      />
      {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
    </form>
  {/if}
  {#snippet footer()}
    {#if done}
      <button class="btn btn-primary" type="button" onclick={() => (open = false)}>{$t('cloud_done')}</button>
    {:else}
      <button class="btn btn-ghost" type="button" onclick={() => (open = false)}>{$t('common_cancel')}</button>
      <button class="btn btn-primary" type="submit" form="cloud-redeem" disabled={busy || sending || !key.trim()}>
        {sending ? $t('cloud_key_applying') : $t('cloud_key_apply')}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .key { width: 100%; height: var(--control-h-lg); text-align: center; font-size: var(--fs-md); letter-spacing: 0.06em; }
  .key::placeholder { letter-spacing: 0.06em; }
</style>

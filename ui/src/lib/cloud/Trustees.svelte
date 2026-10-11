<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Доверенные люди» (cloud-recovery.md §8.1, §8.3, §11): the picker of 3 to
  7 approved contacts and the threshold k, the warning, the state of each
  person («доставлено / не доставлено / проверено N дней назад»), «Заменить»,
  «Проверить», «Выключить». The shares go in Rust (`cloud_trustees_setup`).
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { dateOf } from '$lib/cloud/account';
  import Actions from '$lib/cloud/Actions.svelte';
  import Avatar from '$lib/cloud/Avatar.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import { daysBetween, healthKey, MAX_PEOPLE, MIN_PEOPLE, socialErrorKey, TrusteesStore } from '$lib/cloud/social.svelte';

  const store = new TrusteesStore();
  let picking = $state(false);
  let confirmingOff = $state(false);
  let saved = $state(false);
  let stop: (() => void) | null = null;
  const now = Math.floor(Date.now() / 1000);

  const errorText = $derived.by(() => {
    const key = socialErrorKey(store.error);
    return key ? $t(key) : (store.error?.message ?? '');
  });
  const n = $derived(store.chosen.length);
  const ks = $derived(Array.from({ length: Math.max(0, n - 1) }, (_, i) => i + 2));

  onMount(() => {
    void store.load().then(() => {
      if (!store.status.configured) startPicking();
    });
    void store.listen().then((s) => (stop = s));
  });
  onDestroy(() => stop?.());

  function startPicking() {
    store.startFromStatus();
    saved = false;
    picking = true;
  }

  async function save() {
    if (await store.save()) {
      picking = false;
      saved = true;
    }
  }

  async function turnOff() {
    confirmingOff = false;
    if (await store.disable()) {
      saved = false;
      startPicking();
    }
  }
</script>

{#if picking}
  <p class="text">{$t('cloud_trustees_intro', { k: String(store.k) })}</p>
  {#if store.approved.length < MIN_PEOPLE}
    <Notice tone="warn">{$t('cloud_trustees_none_approved')}</Notice>
  {/if}
  {#if store.approved.length > 0}
    <div class="block">
      <div class="label-row">
        <span class="section-label">{$t('cloud_trustees_people')}</span>
        <span class="count" class:ok={n >= MIN_PEOPLE && n <= MAX_PEOPLE}>{$t('cloud_trustees_chosen_of', { n: String(n), min: String(MIN_PEOPLE), max: String(MAX_PEOPLE) })}</span>
      </div>
      <ul class="people">
        {#each store.approved as c (c.pubkey)}
          {@const on = store.chosen.includes(c.pubkey)}
          <li>
            <label class="person pick" class:on>
              <Avatar name={c.name} />
              <span class="name">{c.name}</span>
              <input type="checkbox" checked={on} onchange={() => store.toggle(c.pubkey)} disabled={store.busy || (!on && n >= MAX_PEOPLE)} />
            </label>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
  {#if n >= 2}
    <div class="block">
      <span class="section-label">{$t('cloud_trustees_threshold_label')}</span>
      <div class="seg" role="radiogroup" aria-label={$t('cloud_trustees_threshold_label')}>
        {#each ks as k (k)}
          <button class="seg-btn" class:active={store.k === k} type="button" role="radio" aria-checked={store.k === k} disabled={store.busy} onclick={() => (store.k = k)}>
            {$t('cloud_trustees_k_of_n', { k: String(k), n: String(n) })}
          </button>
        {/each}
      </div>
    </div>
    <Notice tone="warn">{$t('cloud_trustees_warning', { k: String(store.k), n: String(n) })}</Notice>
  {/if}
  {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
  <Actions>
    {#if store.status.configured}
      <button class="btn btn-ghost" type="button" disabled={store.busy} onclick={() => (picking = false)}>{$t('cloud_link_back')}</button>
    {/if}
    <button class="btn btn-primary" type="button" disabled={store.busy || !store.ready} onclick={save}>
      {store.busy ? $t('cloud_trustees_saving') : $t('cloud_trustees_save')}
    </button>
  </Actions>
{:else}
  {#if saved}<Notice tone="ok">{$t('cloud_trustees_saved')}</Notice>{/if}
  <div class="summary">
    <div class="stat">
      <span class="value">{$t('cloud_trustees_k_of_n', { k: String(store.status.k), n: String(store.status.n) })}</span>
      <span class="label">{$t('cloud_trustees_stat_need')}</span>
    </div>
    <div class="stat">
      <span class="value" class:bad={store.status.acked < store.status.n}>{store.status.acked}/{store.status.n}</span>
      <span class="label">{$t('cloud_trustees_stat_delivered')}</span>
    </div>
    <div class="stat">
      <span class="value">{store.status.checked_at !== null ? `${store.status.alive}/${store.status.n}` : '—'}</span>
      <span class="label">
        {store.status.checked_at !== null ? $t('cloud_trustees_checked', { days: String(daysBetween(store.status.checked_at, now)) }) : $t('cloud_trustees_stat_unchecked')}
      </span>
    </div>
  </div>
  <ul class="people">
    {#each store.status.people as p (p.pubkey)}
      {@const bad = p.health === 'not_delivered' || p.health === 'unconfirmed'}
      <li class="person">
        <Avatar name={p.name} />
        <div class="id">
          <span class="name">{p.name}</span>
          <span class="badge" class:badge-danger={bad} class:badge-ok={p.health === 'alive'}>
            {$t(healthKey(p.health))}{#if p.checked_at !== null} · {dateOf(p.checked_at, $locale)}{/if}
          </span>
        </div>
        <button class="btn btn-ghost btn-sm" type="button" disabled={store.busy} onclick={() => { store.replace(p.pubkey); picking = true; }}>
          {$t('cloud_trustees_replace')}
        </button>
      </li>
    {/each}
  </ul>
  {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
  {#if confirmingOff}
    <Notice tone="danger">{$t('cloud_trustees_disable_confirm')}</Notice>
    <Actions>
      <button class="btn btn-ghost" type="button" onclick={() => (confirmingOff = false)}>{$t('cloud_delete_cancel')}</button>
      <button class="btn btn-danger" type="button" disabled={store.busy} onclick={turnOff}>{$t('cloud_trustees_disable')}</button>
    </Actions>
  {:else}
    <Actions>
      <button class="btn btn-ghost off" type="button" disabled={store.busy} onclick={() => (confirmingOff = true)}>{$t('cloud_trustees_disable')}</button>
      <button class="btn btn-ghost" type="button" disabled={store.busy} onclick={() => store.check()}>
        <Icon name="refresh-cw" size={14} />{$t('cloud_trustees_check')}
      </button>
      <button class="btn btn-primary" type="button" disabled={store.busy} onclick={startPicking}>{$t('cloud_trustees_change')}</button>
    </Actions>
  {/if}
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .label-row { display: flex; align-items: baseline; justify-content: space-between; gap: var(--sp-2); }
  .count { font-size: var(--fs-xs); color: var(--text-3); }
  .count.ok { color: var(--success-text); }
  .people { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .person {
    display: flex; align-items: center; gap: var(--sp-3); min-height: 56px; padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-md); border: 1px solid var(--border); background: var(--surface-2);
  }
  .pick { cursor: pointer; }
  .pick.on { border-color: var(--accent-border); background: var(--accent-tint); }
  .name { flex: 1; min-width: 0; font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .id { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 4px; }
  .id .name { flex: none; max-width: 100%; }
  .seg { align-self: flex-start; flex-wrap: wrap; }
  .summary { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--sp-2); }
  .stat { display: flex; flex-direction: column; gap: 2px; padding: var(--sp-3); border-radius: var(--radius-md); background: var(--surface-2); border: 1px solid var(--border); min-width: 0; }
  .value { font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--text); }
  .value.bad { color: var(--danger-text); }
  .label { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.35; }
  .off { color: var(--danger-text); }
</style>

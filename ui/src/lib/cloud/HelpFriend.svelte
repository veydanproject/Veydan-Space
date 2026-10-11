<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Помочь другу восстановить доступ» → «Подтвердить восстановление»
  (cloud-recovery.md §8.5 steps 5-7, §11): the key of the friend or the
  link they sent, then their name, key, the code computed here, the age of
  the request; the mandatory checkbox of a talk by voice or in person;
  «Подтвердить» sends the share to the one who asked, «Это не он — закрыть»
  sends nothing. A tampered link («Запрос подменён») has no button.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import Actions from '$lib/cloud/Actions.svelte';
  import Avatar from '$lib/cloud/Avatar.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import StateView from '$lib/cloud/StateView.svelte';
  import { timeLeft } from '$lib/cloud/link.svelte';
  import { helpInputOf, HelpStore, shortKey, socialErrorKey } from '$lib/cloud/social.svelte';

  let { search = '' }: { search?: string } = $props();

  const store = new HelpStore();
  let input = $state('');
  let now = $state(Math.floor(Date.now() / 1000));

  const errorText = $derived.by(() => {
    const key = socialErrorKey(store.error);
    return key ? $t(key) : (store.error?.message ?? '');
  });
  const view = $derived(store.view);
  const minutesAgo = $derived(view ? Math.max(0, Math.floor((now - view.created_at) / 60)) : 0);

  /** Minutes in the language of the page: «4 мин», “4 min”. */
  function minutes(n: number): string {
    try {
      return new Intl.NumberFormat($locale, { style: 'unit', unit: 'minute', unitDisplay: 'short' }).format(n);
    } catch {
      return String(n);
    }
  }

  onMount(() => {
    const fromLink = helpInputOf(search);
    if (fromLink) {
      input = fromLink;
      void store.lookup(fromLink);
    }
    const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(timer);
  });
</script>

{#if store.phase === 'sent'}
  <StateView tone="ok" title={$t('cloud_help_sent_title')}>
    <p>{$t('cloud_help_sent_text')}</p>
    {#snippet actions()}
      <Actions><a class="btn btn-primary" href="/cloud">{$t('cloud_done')}</a></Actions>
    {/snippet}
  </StateView>
{:else if store.phase === 'closed'}
  <StateView tone="info" icon="shield" title={$t('cloud_help_dismissed_title')}>
    <p>{$t('cloud_help_dismissed')}</p>
    {#snippet actions()}
      <Actions>
        <a class="btn btn-ghost" href="/cloud">{$t('cloud_done')}</a>
        <button class="btn btn-primary" type="button" onclick={() => { input = ''; store.reset(); }}>{$t('cloud_help_another')}</button>
      </Actions>
    {/snippet}
  </StateView>
{:else if view}
  <div class="person">
    <Avatar name={view.name ?? view.npub.replace(/^npub1/, '')} />
    <div class="id">
      <span class="name">{view.name ?? $t('cloud_help_unknown')}</span>
      <code class="mono key">{shortKey(view.npub)}</code>
    </div>
  </div>
  <div class="block">
    <span class="section-label">{$t('cloud_help_code')}</span>
    <p class="code mono">{view.code}</p>
    {#if view.state === 'open'}
      <p class="text small">{$t('cloud_help_age', { ago: minutes(minutesAgo), left: timeLeft(view.expires_at, now) })}</p>
    {/if}
  </div>
  {#if view.code_matches === false}
    <Notice tone="danger">{$t('cloud_help_tampered')}</Notice>
  {:else if view.state === 'cancelled'}
    <Notice tone="warn">{$t('cloud_help_cancelled')}</Notice>
  {:else if view.state === 'expired'}
    <Notice tone="warn">{$t('cloud_help_expired')}</Notice>
  {:else if view.state === 'closed'}
    <Notice tone="info">{$t('cloud_help_closed')}</Notice>
  {:else}
    {#if view.code_matches === true}<Notice tone="ok">{$t('cloud_help_code_ok')}</Notice>{/if}
    <label class="pledge" class:on={store.spoke}>
      <input type="checkbox" bind:checked={store.spoke} disabled={store.phase === 'sending'} />
      <span>{$t('cloud_help_checkbox')}</span>
    </label>
  {/if}
  {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
  <Actions>
    <button class="btn btn-ghost" type="button" disabled={store.phase === 'sending'} onclick={() => store.close()}>{$t('cloud_help_not_them')}</button>
    {#if view.can_confirm}
      <button class="btn btn-primary" type="button" disabled={!store.spoke || store.phase === 'sending'} onclick={() => store.confirmRequest()}>
        {store.phase === 'sending' ? $t('cloud_link_sending') : $t('cloud_help_confirm')}
      </button>
    {/if}
  </Actions>
{:else}
  <p class="text">{$t('cloud_help_intro')}</p>
  <form class="form" onsubmit={(e) => { e.preventDefault(); void store.lookup(input); }}>
    <div class="form-group">
      <label for="cloud-help-input">{$t('cloud_help_input_label')}</label>
      <input id="cloud-help-input" class="mono" type="text" bind:value={input} placeholder="npub1…" spellcheck="false" autocapitalize="off" autocomplete="off" disabled={store.phase === 'looking'} />
    </div>
    {#if errorText}<div class="error-msg" role="alert">{errorText}</div>{/if}
    <Actions>
      <button class="btn btn-primary" type="submit" disabled={store.phase === 'looking' || !input.trim()}>
        {store.phase === 'looking' ? $t('cloud_help_looking') : $t('cloud_help_lookup')}
      </button>
    </Actions>
  </form>
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .text.small { font-size: var(--fs-sm); }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  input::placeholder { font-family: var(--font-ui); }
  .person { display: flex; align-items: center; gap: var(--sp-3); }
  .id { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .key { font-size: var(--fs-sm); color: var(--text-2); }
  /* The code the friend says by voice: large, apart, never broken inside a group. */
  .code {
    margin: 0; padding: var(--sp-4); text-align: center; border-radius: var(--radius-md);
    background: var(--accent-tint); border: 1px solid var(--accent-tint-border); color: var(--text);
    font-size: var(--fs-xl); font-weight: var(--fw-bold); letter-spacing: 0.06em; white-space: nowrap;
    user-select: text; -webkit-user-select: text;
  }
  /* The promise of a talk by voice is the one thing that guards the friend: a row of its own, not a small print. */
  .pledge {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4); cursor: pointer;
    border-radius: var(--radius-md); border: 1px solid var(--border-2); background: var(--surface-2);
    font-size: var(--fs-base); line-height: 1.45; color: var(--text);
  }
  .pledge.on { border-color: var(--accent-border); background: var(--accent-tint); }
  .pledge input { margin-top: 2px; }
</style>

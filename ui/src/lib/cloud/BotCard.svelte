<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A card of the bot of Veydan Cloud in its chat (cloud-recovery.md §10,
  messenger-wire.md §3), drawn for the messenger through its seam: the
  sentence of the type, a value of the card under it where the table names
  one, and at most one button: «Это не я — отменить» (the command the
  banner calls), «Открыть восстановление», «Ввести ключ активации», «Что
  занимает место» (the three lead to the page of the cloud), and those of
  the web cabinet: «Это не я» and «Завершить сессию» (commands), «Подтвердить»,
  «Открыть аккаунт», «Активировать сейчас» (screens). A request known to be
  over, or out of time, offers no button. A value of a shape held by
  `botCard.ts` — a code of a sign-in, a key of a purchase — is shown large
  (`CopyValue`, with its copy for a key). Every word is `botCard.ts`'s;
  nothing of the card is drawn as markup or as a link.
-->
<script lang="ts" module>
  /** The account and the plans are asked once a run for a card that names a plan, where the page has not read them. */
  let asked = false;
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t, locale, localeTag } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { BotCardProps } from '$lib/messenger/seams/cards.svelte';
  import { shownAccount } from '$lib/cloud/account';
  import { ACTION_ROUTES, cardView, type CardAction } from '$lib/cloud/botCard';
  import { cardsOnScreen } from '$lib/cloud/onScreen.svelte';
  import { alertsStore } from '$lib/cloud/social.svelte';
  import { cloudPage } from '$lib/cloud/status.svelte';
  import { webStore } from '$lib/cloud/web.svelte';
  import CopyValue from '$lib/cloud/CopyValue.svelte';

  let { card, id, at }: BotCardProps = $props();

  let now = $state(Date.now() / 1000);

  // While the card is drawn here, the banner of the same card stays away (RecoveryAlert).
  onMount(() => cardsOnScreen.show(id, card.t === 'recovery.requested' ? card.request_id : null));

  $effect(() => {
    if ((card.t !== 'plan.expiring' && card.t !== 'order.paid' && card.t !== 'order.key') || asked || cloudPage.status) return;
    asked = true;
    void cloudPage.load();
  });

  const view = $derived(
    card.t === 'unknown'
      ? null
      : cardView(card, {
          now,
          at,
          locale: localeTag($locale),
          closed: alertsStore.closed,
          // The names of the plans as the server gives them (the catalog the page of the cloud reads); none until it is read.
          plans: cloudPage.plans?.plans ?? null,
          account: shownAccount(cloudPage.status),
          web: { denied: webStore.denied, ended: webStore.ended },
        }, $t),
  );

  // The button of a request goes when its time is over, while the chat is open too.
  $effect(() => {
    const until = view?.until;
    if (!until) return;
    const wait = (until - Date.now() / 1000) * 1000;
    if (wait <= 0 || wait > 2 ** 31 - 1) return;
    const timer = setTimeout(() => (now = Date.now() / 1000), wait + 50);
    return () => clearTimeout(timer);
  });

  function run(action: CardAction) {
    if (action.id === 'cancel') {
      void alertsStore.cancel(action.requestId);
      return;
    }
    if (action.id === 'web_deny') {
      void webStore.deny(action.loginId);
      return;
    }
    if (action.id === 'web_end') {
      void webStore.revoke({ id: action.session });
      return;
    }
    if (action.id === 'web_confirm' || action.id === 'activate') {
      void goto(action.to);
      return;
    }
    // The banner of the same card has said its part once its way is taken.
    if (action.id === 'recovery' && alertsStore.alerts.some((a) => a.id === id)) void alertsStore.dismiss(id);
    void goto(ACTION_ROUTES[action.id]);
  }
</script>

{#if view}
  <div class="card {view.tone}">
    <div class="top">
      <span class="ico"><Icon name={view.icon} size={16} /></span>
      <div class="words">
        {#if view.title}<p class="title">{view.title}</p>{/if}
        <p class="text">{view.text}</p>
        {#if view.detail}<p class="detail">{view.detail}</p>{/if}
      </div>
    </div>
    {#if view.value}<div class="value"><CopyValue text={view.value.text} copy={view.value.copy} large={!view.value.copy} /></div>{/if}
    {#if view.note}<p class="note">{view.note}</p>{/if}
    {#if view.action}
      {@const action = view.action}
      {@const danger = action.id === 'cancel' || action.id === 'web_deny' || action.id === 'web_end'}
      <div class="row">
        <button class="btn btn-sm {danger ? 'btn-danger' : 'btn-primary'}" type="button" disabled={(action.id === 'cancel' && alertsStore.busy) || ((action.id === 'web_deny' || action.id === 'web_end') && webStore.busy)} onclick={() => run(action)}>
          {action.label}
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .card { display: flex; flex-direction: column; gap: var(--sp-2); width: min(340px, 100%); }
  .top { display: flex; align-items: flex-start; gap: var(--sp-2); min-width: 0; }
  /* The mark and its tone are those of the banners of the same cards (RecoveryAlert.svelte). */
  .ico {
    width: 30px; height: 30px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .danger .ico { color: var(--danger-text); background: var(--danger-bg); }
  .warn .ico { color: var(--warn-text); background: var(--warn-bg); }
  .ok .ico { color: var(--success-text); background: var(--success-bg); }
  .words { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .title { margin: 0; font-size: var(--fs-sm); font-weight: var(--fw-bold); line-height: 1.45; }
  .text { margin: 0; font-size: var(--fs-sm); line-height: 1.45; overflow-wrap: anywhere; }
  .detail { margin: 0; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
  .value { padding-left: calc(30px + var(--sp-2)); }
  .note { margin: 0; padding-left: calc(30px + var(--sp-2)); font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
  .row { display: flex; padding-left: calc(30px + var(--sp-2)); }
</style>

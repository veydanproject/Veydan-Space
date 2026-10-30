<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Входы на сайт» on the page «Account» (cloud-api.md §16.4): the way to
  approve a sign-in to the web cabinet by its QR code («Сканировать QR
  сайта» — the camera on a phone, a picture or a pasted link on a computer,
  all on the screen of approval), the live sessions of the cabinet with
  «Завершить» each and for all, and «Снять заморозку входа по коду» when the
  server reports the code sign-in frozen. A sign-in is the identity's, not
  the account's: the block is there without an account too.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { dateOf, errorKeyOf } from '$lib/cloud/account';
  import { webStore } from '$lib/cloud/web.svelte';
  import { CABINET_HOST, originLine, WEB_LOGIN_ROUTE } from '$lib/cloud/web';
  import NavRow from '$lib/core/ui/NavRow.svelte';
  import RowList from '$lib/cloud/RowList.svelte';

  onMount(() => {
    void webStore.load();
  });

  const sessions = $derived(webStore.sessions ?? []);
  const error = $derived.by(() => {
    const e = webStore.error;
    if (!e) return '';
    const key = errorKeyOf(e.code);
    return key ? $t(key) : e.message || e.code;
  });

  function line(created: number, seen: number): string {
    return $t('cloud_web_session_sub', { since: dateOf(created, $locale), seen: dateOf(seen, $locale) });
  }
</script>

<RowList label={$t('cloud_web_title')} id="web-sign-ins">
  <NavRow variant="list" icon={isMobile ? 'scan' : 'qr-code'} title={$t('cloud_web_scan')} sub={$t('cloud_web_scan_sub', { host: CABINET_HOST })} href={WEB_LOGIN_ROUTE} />
  {#each sessions as session (session.id)}
    <NavRow variant="list" icon="world" title={originLine(session.country, session.ua_family, $t, $locale)} sub={line(session.created_at, session.last_seen_at)}>
      {#snippet end()}
        <button class="btn btn-ghost btn-sm danger" type="button" disabled={webStore.busy} onclick={() => webStore.revoke({ id: session.id })}>{$t('cloud_web_end')}</button>
      {/snippet}
    </NavRow>
  {/each}
  {#if sessions.length > 1}
    <NavRow variant="list" icon="x" tone="bad" danger title={$t('cloud_web_end_all')} onclick={() => webStore.revoke({ all: true })} disabled={webStore.busy} />
  {/if}
  {#if webStore.frozenUntil}
    <NavRow variant="list" icon="lock" tone="warn" title={$t('cloud_web_frozen', { date: dateOf(webStore.frozenUntil, $locale) })} sub={$t('cloud_web_frozen_sub')}>
      {#snippet end()}
        <button class="btn btn-ghost btn-sm" type="button" disabled={webStore.busy} onclick={() => webStore.unfreeze()}>{$t('cloud_web_unfreeze')}</button>
      {/snippet}
    </NavRow>
  {/if}
</RowList>
{#if error && webStore.sessions !== null}<div class="error-msg" role="alert">{error}</div>{/if}

<style>
  .danger { color: var(--danger-text); }
</style>

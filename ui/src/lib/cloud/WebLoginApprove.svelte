<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Вход на my.veydan.net» (cloud-api.md §16.2, 16.4): the QR code of the
  sign-in page of the web cabinet, read by the camera of a phone, from a
  picture on a computer, or pasted as its link (`veydan://web/<id>`); then
  where the request comes from — the country and the browser, large, since
  they are what tells a relayed code from one's own — and «Войти» or «Это
  не я». The domain is a fixed word of the app. A sign-in by code is not
  approved here (its code is typed on the site); only «Это не я» is offered.
  Reached from the page «Account», from a scanned or pasted link in the
  messenger (`?id=`), never from a link a message or the OS opens.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import QrScanner from '$lib/core/ui/QrScanner.svelte';
  import { readQrImage } from '$lib/core/ui/qrImage';
  import { asCloudError, webApi, type CloudError, type CloudWebLogin } from '$lib/cloud/api';
  import { dateOf, errorKeyOf } from '$lib/cloud/account';
  import { webStore } from '$lib/cloud/web.svelte';
  import { browserKey, CABINET_HOST, countryName, isWebId, webLoginId } from '$lib/cloud/web';
  import Actions from '$lib/cloud/Actions.svelte';
  import Countdown from '$lib/cloud/Countdown.svelte';
  import StateView from '$lib/cloud/StateView.svelte';

  let { search = '', onleave }: { search?: string; onleave?: () => void } = $props();

  /** A QR sign-in lives two minutes (cloud-api.md 16.2): the length of the line of the countdown. */
  const QR_TTL_SECS = 120;

  type Phase = 'input' | 'loading' | 'ask' | 'sending' | 'approved' | 'denied' | 'over';
  let phase = $state<Phase>('input');
  let login = $state<CloudWebLogin | null>(null);
  let input = $state('');
  let scanning = $state(false);
  let reading = $state(false);
  let error = $state('');
  let frozenUntil = $state<number | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);
  let now = $state(Date.now() / 1000);

  const place = $derived(login ? countryName(login.country, $locale) : null);
  const browser = $derived(login ? $t(browserKey(login.ua_family)) : '');
  const byCode = $derived(login?.method === 'code');

  onMount(() => {
    const timer = setInterval(() => {
      now = Date.now() / 1000;
      if (phase === 'ask' && login && now >= login.expires_at) phase = 'over';
    }, 1000);
    const id = new URLSearchParams(search).get('id');
    if (isWebId(id)) void open(id);
    else if (id !== null) error = $t('cloud_web_err_not_link');
    return () => clearInterval(timer);
  });

  function words(e: CloudError): string {
    if (e.code === 'not_found') return $t('cloud_web_err_not_found');
    if (e.code === 'login_expired') return $t('cloud_web_over');
    if (e.code === 'method_mismatch') return $t('cloud_web_by_code');
    const key = errorKeyOf(e.code);
    return key ? $t(key) : e.message || e.code;
  }

  /** What the sign-in `id` is: its state decides the screen. */
  async function open(id: string) {
    error = '';
    phase = 'loading';
    try {
      login = await webApi.inspect(id);
      now = Date.now() / 1000;
      if (login.state === 'pending' && now < login.expires_at) phase = 'ask';
      else if (login.state === 'approved') phase = 'approved';
      else if (login.state === 'denied') phase = 'denied';
      else phase = 'over';
    } catch (e) {
      login = null;
      phase = 'input';
      error = words(asCloudError(e));
    }
  }

  /** A scanned or pasted text: the link of a sign-in, or nothing this screen takes. */
  function take(text: string) {
    const id = webLoginId(text);
    if (!id) {
      error = $t('cloud_web_err_not_link');
      return;
    }
    input = '';
    void open(id);
  }

  async function picked(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    reading = true;
    error = '';
    try {
      const found = await readQrImage(file);
      if (found === null) error = $t('cloud_web_err_no_qr');
      else take(found);
    } catch {
      error = $t('cloud_web_err_no_qr');
    } finally {
      reading = false;
      if (fileInput) fileInput.value = '';
    }
  }

  async function approve() {
    if (!login || phase !== 'ask') return;
    phase = 'sending';
    error = '';
    try {
      await webApi.approve(login.id);
      phase = 'approved';
    } catch (e) {
      const refusal = asCloudError(e);
      error = words(refusal);
      phase = refusal.code === 'login_expired' ? 'over' : 'ask';
    }
  }

  async function deny() {
    if (!login) return;
    error = '';
    const done = await webStore.deny(login.id);
    if (done) {
      frozenUntil = done.frozen_until;
      phase = 'denied';
    } else if (webStore.error) {
      error = words(webStore.error);
    }
  }

  function again() {
    login = null;
    error = '';
    frozenUntil = null;
    phase = 'input';
  }
</script>

{#if scanning}
  <QrScanner
    title={$t('cloud_web_scan_title')}
    hint={$t('cloud_web_scan_hint')}
    manual={$t('cloud_web_scan_manual')}
    words={{
      starting: $t('cloud_scan_starting'),
      denied: $t('cloud_scan_denied'),
      failed: $t('cloud_scan_failed'),
      retry: $t('cloud_scan_retry'),
      openSettings: $t('cloud_scan_open_settings'),
    }}
    onresult={(text) => { scanning = false; take(text); }}
    onclose={() => (scanning = false)}
    onmanual={() => (scanning = false)}
  />
{/if}

{#if phase === 'approved'}
  <StateView tone="ok" title={$t('cloud_web_approved_title')}>
    <p>{$t('cloud_web_approved_text', { host: CABINET_HOST })}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => onleave?.()}>{$t('cloud_done')}</button></Actions>
    {/snippet}
  </StateView>
{:else if phase === 'denied'}
  <StateView tone="warn" title={$t('cloud_web_denied_title')}>
    <p>{frozenUntil ? $t('cloud_web_denied_frozen', { date: dateOf(frozenUntil, $locale) }) : $t('cloud_web_denied')}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => onleave?.()}>{$t('cloud_done')}</button></Actions>
    {/snippet}
  </StateView>
{:else if phase === 'over'}
  <StateView tone="info" title={$t('cloud_web_over')}>
    <p>{$t('cloud_web_over_text', { host: CABINET_HOST })}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={again}>{$t('cloud_web_scan_again')}</button></Actions>
    {/snippet}
  </StateView>
{:else if (phase === 'ask' || phase === 'sending') && login}
  <div class="ask">
    <span class="mark"><Icon name="world" size={26} /></span>
    <h2>{$t('cloud_web_sign_in_to', { host: CABINET_HOST })}</h2>
    <div class="origin">
      {#if place}<span class="place">{place}</span>{/if}
      <span class="browser">{browser}</span>
    </div>
    {#if !byCode}
      <Countdown expiresAt={login.expires_at} total={QR_TTL_SECS} {now} label={(time) => $t('cloud_web_time_left', { time })} />
      <p class="text">{$t('cloud_web_ask_hint')}</p>
    {:else}
      <p class="text">{$t('cloud_web_by_code')}</p>
    {/if}
    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
    <Actions>
      <button class="btn btn-ghost danger" type="button" disabled={phase === 'sending' || webStore.busy} onclick={deny}>{$t('cloud_web_not_me')}</button>
      {#if !byCode}
        <button class="btn btn-primary" type="button" disabled={phase === 'sending' || webStore.busy} onclick={approve}>
          {phase === 'sending' ? $t('cloud_web_signing_in') : $t('cloud_web_sign_in')}
        </button>
      {/if}
    </Actions>
  </div>
{:else}
  <div class="block">
    <h2>{$t('cloud_web_input_title')}</h2>
    <p class="text">{$t('cloud_web_input_hint', { host: CABINET_HOST })}</p>
  </div>
  {#if phase === 'loading'}
    <div class="loading"><span class="spinner" aria-hidden="true"></span></div>
  {:else}
    {#if isMobile}
      <button class="m-btn-grad" type="button" onclick={() => { error = ''; scanning = true; }}>
        <Icon name="qr-code" size={18} />{$t('cloud_web_scan')}
      </button>
    {:else}
      <input bind:this={fileInput} type="file" accept="image/*" hidden onchange={picked} />
      <button class="btn btn-ghost" type="button" disabled={reading} onclick={() => { error = ''; fileInput?.click(); }}>
        <Icon name={reading ? 'loader' : 'image'} size={14} />{$t('cloud_web_scan_image')}
      </button>
    {/if}
    <div class="or"><span>{$t('cloud_or')}</span></div>
    <form class="form" onsubmit={(e) => { e.preventDefault(); take(input); }}>
      <div class="form-group">
        <label for="cloud-web-link">{$t('cloud_web_paste_label')}</label>
        <input id="cloud-web-link" class="mono" type="text" bind:value={input} placeholder="veydan://web/…" spellcheck="false" autocomplete="off" autocapitalize="off" />
      </div>
      {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
      <Actions>
        <button class="btn" class:btn-primary={!isMobile} class:btn-ghost={isMobile} type="submit" disabled={!input.trim()}>{$t('cloud_link_next')}</button>
      </Actions>
    </form>
  {/if}
{/if}

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-1); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  .or { display: flex; align-items: center; gap: var(--sp-3); color: var(--text-3); font-size: var(--fs-sm); }
  .or::before, .or::after { content: ''; flex: 1; height: 1px; background: var(--border-2); }
  .loading { display: flex; justify-content: center; padding: var(--sp-6); }
  .ask { display: flex; flex-direction: column; gap: var(--sp-3); }
  .ask h2 { text-align: center; font-size: var(--fs-lg); }
  .mark {
    align-self: center; display: inline-flex; align-items: center; justify-content: center; width: 56px; height: 56px; border-radius: 50%;
    background: var(--accent-tint); color: var(--accent-text); border: 1px solid var(--accent-tint-border);
  }
  .origin { display: flex; flex-direction: column; align-items: center; gap: 2px; padding: var(--sp-3); border-radius: var(--radius); background: var(--surface-2); }
  .place { font-size: var(--fs-xl); font-weight: var(--fw-bold); color: var(--text); }
  .browser { font-size: var(--fs-md); color: var(--text-2); }
  .danger { color: var(--danger-text); }
</style>

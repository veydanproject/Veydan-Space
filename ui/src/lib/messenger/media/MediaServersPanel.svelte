<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerApi, type MessengerMediaServer } from '../api';
  import { mediaErrorText } from './errors';
  import { onPhone } from '../shared/phone';

  /** The phone: the address on the first line, whole; Check and the switch under it, at the right. */
  const phone = onPhone();

  let servers = $state<MessengerMediaServer[]>([]);
  let busy = $state(false);
  let error = $state('');
  let checked = $state<Record<string, 'ok' | 'fail'>>({});
  let editing = $state<string | null>(null);
  let adding = $state(false);

  let kind = $state<'s3' | 'blossom'>('s3');
  let url = $state('');
  let bucket = $state('');
  let region = $state('us-east-1');
  let accessKey = $state('');
  let secretKey = $state('');

  const canAct = $derived(!!messengerStore.status?.runtime?.session_active);

  function explain(e: unknown): string {
    return mediaErrorText(e, (key) => $t(key));
  }

  async function load() { servers = await messengerApi.media.servers(); }
  onMount(() => { load().catch(() => {}); });

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); await load(); }
    catch (e) { error = explain(e); }
    finally { busy = false; }
  }

  function resetForm() { kind = 's3'; url = ''; bucket = ''; region = 'us-east-1'; accessKey = ''; secretKey = ''; editing = null; adding = false; }

  function editCredentials(s: MessengerMediaServer) {
    editing = s.id; adding = false;
    kind = s.kind; url = s.url; bucket = s.bucket ?? ''; region = s.region ?? 'us-east-1';
    accessKey = s.access_key ?? ''; secretKey = '';
  }

  async function save() {
    await run(async () => {
      const saved = await messengerApi.media.putServer({
        id: editing, kind, url: url.trim(),
        bucket: kind === 's3' ? bucket.trim() : null,
        region: kind === 's3' ? region.trim() : null,
        access_key: kind === 's3' ? accessKey.trim() : null,
        secret_key: kind === 's3' && secretKey ? secretKey : null,
      });
      resetForm();
      if (saved.kind === 'blossom' || saved.has_secret) await check(saved.id);
    });
  }

  async function check(id: string) {
    error = ''; busy = true;
    try { await messengerApi.media.checkServer(id); checked = { ...checked, [id]: 'ok' }; }
    catch (e) { checked = { ...checked, [id]: 'fail' }; error = explain(e); }
    finally { busy = false; }
  }

  const needsSecret = (s: MessengerMediaServer) => s.kind === 's3' && !(s.has_secret && s.access_key);
  const formValid = $derived(
    /^https?:\/\/.+/.test(url.trim()) && (kind === 'blossom' || (bucket.trim() && accessKey.trim() && (secretKey || editing))),
  );
</script>

<div class="card media-servers">
  <div class="card-title"><Icon name="server-2" size={16} /> {$t('msg_media_servers_title')}</div>
  <p class="muted">{$t('msg_media_servers_text')}</p>

  {#if servers.length === 0}
    <div class="muted small">{$t('msg_media_servers_empty')}</div>
  {:else}
    <ul class="list" class:phone>
      {#each servers as s (s.id)}
        <li class="row" class:off={!s.enabled}>
          <span class="kind">{s.kind === 's3' ? 'S3' : 'Blossom'}</span>
          <span class="info">
            <code>{s.public_base}</code>
            <span class="meta">
              {s.source === 'manifest' ? $t('msg_relay_source_manifest') : $t('msg_relay_source_user')}
              {#if needsSecret(s)} · <span class="warn">{$t('msg_media_needs_keys')}</span>
              {:else if checked[s.id] === 'ok'} · <span class="ok">{$t('msg_media_check_ok')}</span>
              {:else if checked[s.id] === 'fail'} · <span class="bad">{$t('msg_media_check_fail')}</span>{/if}
            </span>
          </span>
          <span class="actions">
            {#if s.kind === 's3'}
              <button class="btn btn-ghost" disabled={busy} onclick={() => editCredentials(s)}><Icon name="key" size={18} />{$t('msg_media_keys')}</button>
            {/if}
            <button class="btn btn-ghost" disabled={busy || !canAct || needsSecret(s)} onclick={() => check(s.id)}>{$t('msg_media_check')}</button>
            <button class="toggle" class:on={s.enabled} disabled={busy} aria-pressed={s.enabled} aria-label={$t("msg_relay_enabled")} title={$t("msg_relay_enabled")}
              onclick={() => run(() => messengerApi.media.setServerEnabled(s.id, !s.enabled))}></button>
            {#if s.source === 'user'}
              <button class="icon-btn danger-soft" disabled={busy} onclick={() => run(() => messengerApi.media.removeServer(s.id))} title={$t('msg_relay_remove')}><Icon name="trash-2" size={18} /></button>
            {/if}
          </span>
        </li>
      {/each}
    </ul>
  {/if}

  {#if editing || adding}
    <form class="form" onsubmit={(e) => { e.preventDefault(); if (formValid) save(); }}>
      {#if adding}
        <div class="seg kinds" role="radiogroup" aria-label="S3 / Blossom">
          <button type="button" class="seg-btn" class:active={kind === 's3'} role="radio" aria-checked={kind === 's3'} onclick={() => (kind = 's3')}>S3</button>
          <button type="button" class="seg-btn" class:active={kind === 'blossom'} role="radio" aria-checked={kind === 'blossom'} onclick={() => (kind = 'blossom')}>Blossom</button>
        </div>
        <label class="wide"><span>{kind === 's3' ? $t('msg_media_endpoint') : 'URL'}</span>
          <input type="text" bind:value={url} placeholder="https://…" spellcheck="false" disabled={busy} /></label>
      {/if}
      {#if kind === 's3'}
        {#if adding}
          <label><span>{$t('msg_media_bucket')}</span><input type="text" bind:value={bucket} spellcheck="false" disabled={busy} /></label>
          <label><span>{$t('msg_media_region')}</span><input type="text" bind:value={region} spellcheck="false" disabled={busy} /></label>
        {/if}
        <label><span>{$t('msg_media_access_key')}</span><input type="text" bind:value={accessKey} spellcheck="false" autocomplete="off" disabled={busy} /></label>
        <label><span>{$t('msg_media_secret_key')}</span><input type="password" bind:value={secretKey} autocomplete="new-password" disabled={busy}
          placeholder={editing ? $t('msg_media_secret_keep') : ''} /></label>
      {/if}
      <div class="form-actions wide">
        <button type="button" class="btn btn-ghost" disabled={busy} onclick={resetForm}>{$t('msg_back')}</button>
        <button type="submit" class="btn btn-primary" disabled={busy || !formValid}>{$t('msg_media_save_check')}</button>
      </div>
      <p class="muted small wide">{$t('msg_media_secret_hint')}</p>
    </form>
  {:else}
    <button class="btn btn-ghost add" disabled={busy} onclick={() => { resetForm(); adding = true; }}><Icon name="plus" size={18} />{$t('msg_media_add')}</button>
  {/if}

  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

<style>
  .media-servers { display: flex; flex-direction: column; gap: var(--sp-3); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; line-height: 1.5; }
  .small { font-size: var(--fs-xs); }
  /* The servers as rows of 48 with a line between them (as the facts of «Профиль и ключ»). */
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: var(--sp-2) var(--sp-3); min-height: 48px; padding: var(--sp-2) 0; flex-wrap: wrap; }
  .row + .row { border-top: 1px solid var(--border); }
  .row.off { opacity: 0.6; }
  .kind { flex-shrink: 0; font-size: var(--fs-xs); font-weight: var(--fw-semibold); padding: 2px var(--sp-2); border-radius: var(--radius-sm); background: var(--accent-tint); color: var(--accent-text-2); }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1 1 200px; }
  .info code { font-family: var(--font-mono); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: var(--fs-xs); color: var(--text-3); }
  .ok { color: var(--success-text); } .bad { color: var(--danger-text); } .warn { color: var(--warn-text); }
  .actions { display: inline-flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .phone .info { flex: 1 1 0; }
  .phone .info code { white-space: normal; overflow-wrap: anywhere; }
  .phone .actions { flex-basis: 100%; justify-content: flex-end; }
  @media (max-width: 560px) { .form { grid-template-columns: 1fr; } }
  /* The form under the list: a line above it, fields of 44 under captions as on «Профиль и ключ». */
  .form { display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-3); padding-top: var(--sp-3); border-top: 1px solid var(--border); }
  .form .wide { grid-column: 1 / -1; }
  .form label { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); }
  .form input {
    font: inherit; font-size: var(--fs-base); font-weight: var(--fw-normal); color: var(--text); min-height: 44px; min-width: 0;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: 0 var(--sp-3);
  }
  .form input:focus { outline: none; border-color: var(--accent-border); }
  .form-actions { display: flex; justify-content: flex-end; gap: var(--sp-2); flex-wrap: wrap; }
  .kinds { grid-column: 1 / -1; justify-self: start; }
  .add { align-self: flex-start; }
</style>

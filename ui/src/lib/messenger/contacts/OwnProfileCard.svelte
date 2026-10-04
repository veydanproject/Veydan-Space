<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError, profileLabel, type MessengerProfileInput } from '../api';
  import Avatar from './Avatar.svelte';

  let editing = $state(false);
  let busy = $state(false);
  let error = $state('');
  let form = $state<MessengerProfileInput>({});

  const p = $derived(messengerStore.ownProfile);
  const canAct = $derived(!!messengerStore.status?.runtime?.session_active);

  function startEdit() {
    form = {
      name: p?.name ?? '', display_name: p?.display_name ?? '', about: p?.about ?? '',
      picture: p?.picture ?? '', website: p?.website ?? '', nip05: p?.nip05 ?? '', lud16: p?.lud16 ?? '',
    };
    editing = true; error = '';
  }

  async function save() {
    busy = true; error = '';
    try { await messengerStore.saveOwnProfile(form); editing = false; }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }
</script>

<div class="card own">
  <div class="card-title"><Icon name="user" size={16} /> {$t('msg_own_title')}</div>
  {#if !editing}
    <div class="head">
      <Avatar url={p?.picture ?? null} label={profileLabel(p) || (messengerStore.identity?.npub ?? '?')} seed={messengerStore.identity?.pubkey} size={48} />
      <div class="info">
        <div class="name">{profileLabel(p) || $t('msg_own_unnamed')}</div>
        {#if p?.nip05}<div class="meta">{p.nip05}</div>{/if}
        {#if p?.about}<p class="about">{p.about}</p>{/if}
      </div>
    </div>
    <div class="actions">
      <button class="btn btn-ghost btn-sm" disabled={!canAct} onclick={startEdit}><Icon name="pencil" size={12} />{$t('msg_own_edit')}</button>
    </div>
  {:else}
    <div class="grid">
      <label><span>{$t('msg_own_name')}</span><input type="text" bind:value={form.name} disabled={busy} /></label>
      <label><span>{$t('msg_own_display_name')}</span><input type="text" bind:value={form.display_name} disabled={busy} /></label>
      <label class="wide"><span>{$t('msg_own_about')}</span><textarea rows="2" bind:value={form.about} disabled={busy}></textarea></label>
      <label class="wide"><span>{$t('msg_own_picture')}</span><input type="text" bind:value={form.picture} placeholder="https://…" spellcheck="false" disabled={busy} /></label>
      <label><span>NIP-05</span><input type="text" bind:value={form.nip05} placeholder="user@domain" spellcheck="false" disabled={busy} /></label>
      <label><span>{$t('msg_own_website')}</span><input type="text" bind:value={form.website} placeholder="https://…" spellcheck="false" disabled={busy} /></label>
    </div>
    {#if error}<div class="error-msg">{error}</div>{/if}
    <div class="actions">
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (editing = false)}>{$t('msg_back')}</button>
      <button class="btn btn-primary btn-sm" disabled={busy} onclick={save}>{$t('msg_own_publish')}</button>
    </div>
    <p class="muted small">{$t('msg_own_publish_hint')}</p>
  {/if}
</div>

<style>
  .own { max-width: 640px; display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .head { display: flex; gap: var(--sp-3); align-items: flex-start; }
  .info { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .name { font-weight: var(--fw-semibold); }
  .meta { color: var(--text-3); font-size: var(--fs-xs); }
  .about { margin: 0; font-size: var(--fs-sm); color: var(--text-body); white-space: pre-wrap; }
  .muted { color: var(--text-2); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .actions { display: flex; gap: var(--sp-2); justify-content: flex-end; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-2); }
  @media (max-width: 560px) { .grid { grid-template-columns: 1fr; } }
  .grid .wide { grid-column: 1 / -1; }
  .grid label { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); }
  .grid input, .grid textarea {
    font: inherit; font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 6px 8px;
  }
  .grid textarea { resize: vertical; }
</style>

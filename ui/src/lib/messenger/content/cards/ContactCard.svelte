<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A person someone shared: who it is, and how to reach them. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import { dmErrorCode, messengerError, type LinkView } from '../../api';
  import { linkActions } from '../actions';

  interface Props { view: Extract<LinkView, { kind: 'contact' }> }
  let { view }: Props = $props();

  let busy = $state(false);
  let error = $state('');

  const key = $derived(`${view.npub.slice(0, 12)}…${view.npub.slice(-4)}`);
  const name = $derived(view.name.trim() || key);

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) {
      const code = dmErrorCode(e);
      error = code ? $t(`msg_err_${code}` as 'msg_err_dm_blocked', { name }) : messengerError(e);
    }
    finally { busy = false; }
  }
</script>

<div class="box">
  <div class="top">
    <Avatar url={view.picture} label={name} seed={view.pubkey} size={44} />
    <div class="about">
      <span class="name">{name}</span>
      <span class="sub">
        {#if view.nip05}<span class="nip05">{view.nip05}</span>{:else}<code>{key}</code>{/if}
      </span>
    </div>
  </div>

  {#if view.is_me}
    <div class="status"><Icon name="user" size={12} />{$t('msg_card_contact_me')}</div>
  {:else if view.blocked}
    <div class="status closed"><Icon name="ban" size={12} />{$t('msg_card_contact_blocked')}</div>
  {:else}
    {#if error}<div class="error">{error}</div>{/if}
    <div class="row">
      <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => run(() => linkActions.write(view))}>
        <Icon name="message-circle" size={13} />{$t('msg_contacts_write')}
      </button>
      {#if view.is_contact}
        <span class="known"><Icon name="check" size={12} />{$t('msg_card_contact_known')}</span>
      {:else}
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => run(() => linkActions.addContact(view))}>
          <Icon name="user-plus" size={13} />{$t('msg_card_contact_add')}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); }
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub code { font-family: var(--font-mono); }
  .status { display: flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-2); }
  .status.closed { color: var(--warn-text); }
  .row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .known { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--text-3); padding: 0 4px; }
  .error { font-size: var(--fs-xs); color: var(--danger-text); line-height: 1.4; }
</style>

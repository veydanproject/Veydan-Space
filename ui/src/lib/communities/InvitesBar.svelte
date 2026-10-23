<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/InvitesBar.svelte at 8ce09c39, then changed for channels. -->

<!-- Invitations to channels that wait for my answer, above the list of channels. -->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { Avatar, onPhone } from '$lib/messenger/seams/shared';
  import { channelStore } from './channelStore.svelte';
  import { nameStore } from './names.svelte';
  import { channelError } from './errors';
  import type { InviteView } from './api';
  import { cover } from './cover';

  /** On the phone the cards share the inset of the search field (12px). */
  const phone = onPhone();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);

  let busy = $state<string | null>(null);
  let error = $state('');

  async function answer(i: InviteView, accept: boolean) {
    error = ''; busy = i.invite_id;
    try {
      await channelStore.answerInvite(i.invite_id, accept);
    } catch (e) {
      error = channelError(e, tr);
      channelStore.refreshInvites().catch(() => {});
    } finally { busy = null; }
  }
</script>

{#if channelStore.invites.length}
  <div class="invites" class:phone>
    {#each channelStore.invites as i (i.invite_id)}
      <div class="invite">
        <Avatar {...cover(i.picture)} label={i.name} seed={i.community_id} size={38} />
        <div class="text">
          <span class="title">{i.name}</span>
          <span class="sub">{$t('chan_invited_by', { name: nameStore.label(i.peer) })}</span>
        </div>
        <button class="btn btn-primary btn-sm" disabled={busy === i.invite_id} onclick={() => answer(i, true)}>{$t('chan_invite_accept')}</button>
        <button class="icon" disabled={busy === i.invite_id} title={$t('chan_invite_decline')} aria-label={$t('chan_invite_decline')} onclick={() => answer(i, false)}><Icon name="x" size={15} /></button>
      </div>
    {/each}
    {#if error}<div class="error-msg">{error}</div>{/if}
  </div>
{/if}

<style>
  .invites { display: flex; flex-direction: column; gap: 4px; padding: 0 var(--sp-2) var(--sp-2); flex-shrink: 0; }
  .invite {
    display: flex; align-items: center; gap: var(--sp-2); padding: var(--sp-2); border-radius: var(--radius-md);
    background: var(--accent-tint); border: 1px solid var(--accent-tint-border);
  }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .title { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .invites.phone { padding-inline: var(--sp-3); }
  @media (pointer: coarse) { .icon { padding: 10px; } }
</style>

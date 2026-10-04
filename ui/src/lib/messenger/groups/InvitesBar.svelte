<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Invitations to groups that wait for my answer, above the chats. -->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import { membersKey } from '../i18n';
  import { groupError } from './errors';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { groupStore } from './groupStore.svelte';
  import { nameStore } from './names.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { type MessengerGroupInvite } from '../api';
  import { onPhone } from '../shared/phone';

  /** On the phone the cards share the inset of the search field and the push offer (12px). */
  const phone = onPhone();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as "msg_you", params);

  let busy = $state<string | null>(null);
  let error = $state('');

  async function answer(i: MessengerGroupInvite, accept: boolean) {
    error = ''; busy = i.invite_id;
    try {
      await groupStore.answerInvite(i.invite_id, accept);
      if (accept) chatStore.scheduleChatsRefresh();
    } catch (e) {
      error = groupError(e, tr);
      groupStore.refreshInvites().catch(() => {});
    } finally { busy = null; }
  }
</script>

{#if groupStore.invites.length}
  <div class="invites" class:phone>
    {#each groupStore.invites as i (i.invite_id)}
      <div class="invite">
        <Avatar url={i.picture || null} label={i.name} seed={i.group_id} size={38} />
        <div class="text">
          <span class="title">{i.name}</span>
          <span class="sub">{$t('msg_group_invited_by', { name: nameStore.label(i.peer) })} · {$t(membersKey(i.members), { n: String(i.members) })}</span>
        </div>
        <button class="btn btn-primary btn-sm" disabled={busy === i.invite_id} onclick={() => answer(i, true)}>{$t('msg_group_invite_accept')}</button>
        <button class="icon" disabled={busy === i.invite_id} title={$t('msg_group_invite_decline')} onclick={() => answer(i, false)}><Icon name="x" size={15} /></button>
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
  /* The phone: the inset of the search field and the push offer above (12px) */
  .invites.phone { padding-inline: var(--sp-3); }
  @media (pointer: coarse) { .icon { padding: 10px; } }
</style>

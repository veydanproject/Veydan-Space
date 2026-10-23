<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/InvitePicker.svelte at 8ce09c39, then changed for channels. -->

<!--
  Whom to invite into a channel, or whom to make its author: a contact, or
  anyone by their key. An invitation is a note the keeper lets in by; an
  author is a role in the journal (a manager's).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { Avatar, contactHaystacks, contactLabel, contacts as contactsOf, matchesQuery } from '$lib/messenger/seams/shared';
  import { channelStore } from './channelStore.svelte';
  import { channelError } from './errors';
  import { staffOf } from './permissions';
  import { channelsApi, type ChannelDetail } from './api';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);

  interface Props {
    open: boolean;
    detail: ChannelDetail;
    /** `invite`: an invitation; `author`: the role of an author. */
    purpose?: 'invite' | 'author';
  }
  let { open = $bindable(), detail, purpose = 'invite' }: Props = $props();

  let key = $state('');
  let busy = $state(false);
  let error = $state('');
  let done = $state<string[]>([]);

  $effect(() => { if (!open) { key = ''; error = ''; done = []; } });

  const cid = $derived(detail.channel.id);
  const looksLikeKey = $derived(/^(npub1[0-9a-z]{20,}|[0-9a-f]{64})$/i.test(key.trim()));
  const all = $derived(contactsOf());
  const haystacks = $derived(contactHaystacks(all));
  /** The banned of the channel: neither invited nor made authors (a moderator's list). */
  let banned = $state<Set<string>>(new Set());
  $effect(() => {
    if (!open) return;
    const id = cid;
    channelsApi.moderation.blocked(id).then((b) => { if (id === cid) banned = new Set(b); }).catch(() => {});
  });
  /** Those it makes sense for, as the groups' picker: not the team already, not banned. */
  const shown = $derived(
    all.filter((c) => staffOf(detail, c.pubkey).role === 'member' && !banned.has(c.pubkey) && matchesQuery(haystacks.get(c.pubkey) ?? '', key)),
  );

  async function pick(who: string) {
    error = ''; busy = true;
    try {
      if (purpose === 'author') await channelStore.act(cid, { op: 'set_role', who, role: 'author' });
      else await channelStore.invite(cid, who);
      done = [...done, who];
      if (who === key.trim()) key = '';
    } catch (e) {
      error = channelError(e, tr);
    } finally { busy = false; }
  }

  const cta = $derived(purpose === 'author' ? $t('chan_make_author') : $t('chan_invite'));
</script>

<Dialog bind:open title={purpose === 'author' ? $t('chan_add_author_title', { name: detail.channel.name }) : $t('chan_invite_title', { name: detail.channel.name })}
  width="min(440px, calc(100vw - 24px))">
  <div class="body">
    <input type="text" bind:value={key} placeholder={$t('chan_invite_placeholder')} aria-label={$t('chan_invite_field')} spellcheck="false" disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter' && looksLikeKey) pick(key.trim()); }} />
    {#if looksLikeKey}
      <button class="btn btn-primary" disabled={busy} onclick={() => pick(key.trim())}>{cta}</button>
    {/if}
    <p class="hint">{purpose === 'author' ? $t('chan_add_author_hint') : $t('chan_invite_hint')}</p>
    {#if error}<div class="error-msg">{error}</div>{/if}
    {#if shown.length}
      <ul class="list divided">
        {#each shown as c (c.pubkey)}
          <li class="row">
            <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={34} />
            <span class="name">{contactLabel(c)}</span>
            {#if done.includes(c.pubkey)}
              <span class="done"><Icon name="check" size={13} />{purpose === 'author' ? $t('chan_made_author') : $t('chan_invite_sent')}</span>
            {:else}
              <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => pick(c.pubkey)}>{cta}</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  input {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  input:focus { outline: none; border-color: var(--accent-border); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  .list { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .row { display: flex; align-items: center; gap: var(--sp-3); padding: 6px 8px; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .done { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--success-text, var(--success)); }
  .body > .btn { justify-content: center; }
  @media (pointer: coarse) { input { font-size: var(--fs-md); min-height: 44px; } }
</style>

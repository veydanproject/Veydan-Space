<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A group someone shared: what it is, and the way in. -->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import { membersKey } from '../../i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import { groupError } from '../../groups/errors';
  import type { LinkView } from '../../api';
  import { linkActions } from '../actions';

  interface Props { view: Extract<LinkView, { kind: 'group' }> }
  let { view }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);

  let busy = $state(false);
  let error = $state('');
  let asking = $state(false);
  let note = $state('');

  const open = $derived(view.group_kind === 'public');
  const name = $derived(view.name.trim() || $t('msg_group_unnamed'));
  /** `in`: I am a member. `wait`: asked, not answered. `closed`: no way in. `out`: the way in is shown. */
  const stage = $derived.by(() => {
    switch (view.membership) {
      case 'joined': return 'in';
      case 'joining': case 'requested': return 'wait';
      case 'banned': case 'disbanded': case 'stale_link': return 'closed';
      default: return 'out';
    }
  });

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) { error = groupError(e, tr); }
    finally { busy = false; }
  }

  function join() {
    if (!open && !asking) { asking = true; return; }
    run(async () => {
      await linkActions.joinGroup(view, open ? '' : note);
      asking = false; note = '';
    });
  }
</script>

<div class="box">
  <div class="top">
    <Avatar url={view.picture} label={name} seed={view.group_id} size={44} />
    <div class="about">
      <span class="name">{name}</span>
      <span class="sub">
        <Icon name={open ? 'globe' : 'lock'} size={11} />
        {$t(open ? 'msg_card_group_public' : 'msg_card_group_private')}{#if view.members !== null} · {$t(membersKey(view.members), { n: String(view.members) })}{/if}
      </span>
    </div>
  </div>

  {#if stage === 'wait' || stage === 'closed'}
    <div class="status" class:closed={stage === 'closed'}>
      <Icon name={stage === 'wait' ? 'clock' : 'lock'} size={12} />
      {$t(`msg_group_state_${view.membership}` as 'msg_group_state_requested')}
    </div>
  {/if}

  {#if asking}
    <input class="note" type="text" bind:value={note} maxlength="300" placeholder={$t('msg_group_join_note')} disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter') join(); if (e.key === 'Escape') asking = false; }} />
  {/if}
  {#if error}<div class="error">{error}</div>{/if}

  {#if stage === 'in'}
    <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => run(() => linkActions.openGroup(view))}>{$t('msg_card_open')}</button>
  {:else if stage === 'out'}
    <div class="row">
      <button class="btn btn-primary btn-sm" disabled={busy} onclick={join}>
        {$t(open ? 'msg_group_join' : asking ? 'msg_card_group_send' : 'msg_group_join_ask')}
      </button>
      {#if asking}<button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (asking = false)}>{$t('msg_group_cancel')}</button>{/if}
    </div>
  {/if}
</div>

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); }
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--text-3); }
  .status { display: flex; align-items: flex-start; gap: 5px; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; }
  .status.closed { color: var(--warn-text); }
  .status :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .note {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 7px 10px;
  }
  .note:focus { outline: none; border-color: var(--accent-border); }
  .row { display: flex; gap: 6px; flex-wrap: wrap; }
  .row .btn-primary, .box > .btn { flex: 1; justify-content: center; }
  .error { font-size: var(--fs-xs); color: var(--danger-text); line-height: 1.4; }
  @media (pointer: coarse) { .note { font-size: 16px; } }
</style>

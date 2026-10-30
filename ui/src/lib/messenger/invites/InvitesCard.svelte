<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Мои приглашения» in the privacy settings: every link, code and
  permanent QR of mine, what each is (kind, for whom), how much of it is
  used and until when it holds, with a way to revoke it and to make a new
  link or code. A pause after someone guessed at codes shows above them:
  until it ends, nobody gets in by any of them.
-->
<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerError, type InviteView } from '../api';
  import { nameStore } from '../groups/names.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { clock, stamp } from '../shared/time';
  import { inviteStore } from './inviteStore.svelte';
  import { endedBy, formatCode, isActive, listed } from './invites';

  const nowSec = () => Math.floor(Date.now() / 1000);
  let now = $state(nowSec());
  let busy = $state(false);
  let error = $state('');
  let copied = $state<string | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const items = $derived(listed(inviteStore.list ?? [], now));
  const paused = $derived(inviteStore.pausedUntil(now));

  onMount(() => {
    inviteStore.load().catch(() => {});
    // Terms end and pauses lift while the page is open: a minute is fine enough for both.
    timer = setInterval(() => (now = nowSec()), 30_000);
  });
  onDestroy(() => { if (timer) clearInterval(timer); });

  function title(v: InviteView): string {
    if (v.kind === 'code') return $t('msg_inv_kind_code', { code: formatCode(v.secret) });
    return $t(v.kind === 'qr' ? 'msg_inv_kind_qr' : 'msg_inv_kind_link');
  }

  function uses(v: InviteView): string {
    return v.max_uses != null
      ? $t('msg_inv_used', { used: String(v.used), max: String(v.max_uses) })
      : $t('msg_inv_used_n', { used: String(v.used) });
  }

  function term(v: InviteView): string {
    const ended = endedBy(v, now);
    if (ended) return $t(`msg_inv_ended_${ended}` as 'msg_inv_ended_used');
    return v.expires_at != null ? $t('msg_inv_until', { time: stamp(v.expires_at) }) : $t('msg_inv_forever');
  }

  /** Uses, term and whom it is for, in one line. */
  function meta(v: InviteView): string {
    const parts = [uses(v), term(v)];
    if (v.for_pubkey) parts.push($t('msg_inv_for', { name: nameStore.label(v.for_pubkey) }));
    return parts.join(' · ');
  }

  async function create(kind: 'link' | 'code') {
    error = '';
    busy = true;
    try {
      const v = await inviteStore.create({ kind, label: '' });
      // A new link is for sending: it is on the clipboard at once.
      if (kind === 'link' && v.link) await copy(v);
    } catch (e) {
      error = messengerError(e);
    } finally {
      busy = false;
    }
  }

  async function copy(v: InviteView) {
    try {
      await navigator.clipboard.writeText(v.kind === 'code' ? v.secret : (v.link ?? ''));
      copiedToast();
      copied = v.id;
      setTimeout(() => { if (copied === v.id) copied = null; }, 1600);
    } catch (e) {
      error = messengerError(e);
    }
  }

  async function revoke(v: InviteView) {
    if (!(await confirmStore.ask($t('msg_inv_revoke_confirm'), $t('msg_inv_revoke'), true))) return;
    await inviteStore.revoke(v.id);
  }
</script>

<div class="card invites">
  <div class="card-title"><Icon name="qr-code" size={16} /> {$t('msg_inv_title')}</div>
  <p class="muted">{$t('msg_inv_intro')}</p>

  {#if paused}
    <div class="strip" role="status">
      <Icon name="alert-triangle" size={14} />
      <span><strong>{$t('msg_inv_paused', { time: clock(paused) })}</strong><br />{$t('msg_inv_paused_hint')}</span>
    </div>
  {/if}

  {#if !inviteStore.list}
    <p class="muted">{inviteStore.error || $t('loading')}</p>
  {:else if items.length === 0}
    <p class="muted">{$t('msg_inv_empty')}</p>
  {:else}
    <ul class="list">
      {#each items as v (v.id)}
        {@const live = isActive(v, now)}
        <li class="row" class:ended={!live}>
          <span class="kind"><Icon name={v.kind === 'code' ? 'key' : v.kind === 'qr' ? 'qr-code' : 'link'} size={16} /></span>
          <span class="info">
            <span class="name">{[title(v), v.label].filter(Boolean).join(' · ')}</span>
            <span class="meta">{meta(v)}</span>
          </span>
          {#if live}
            <span class="buttons">
              {#if v.kind !== 'qr'}
                <button class="btn btn-ghost" onclick={() => copy(v)} aria-label={$t('msg_copy')} title={$t('msg_copy')}>
                  <Icon name={copied === v.id ? 'check' : 'copy'} size={18} />
                </button>
              {/if}
              <button class="btn btn-ghost danger" disabled={inviteStore.revoking === v.id} onclick={() => revoke(v)}>{$t('msg_inv_revoke')}</button>
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  <div class="actions">
    <button class="btn btn-ghost" disabled={busy} onclick={() => create('link')}><Icon name="link" size={18} />{$t('msg_inv_create_link')}</button>
    <button class="btn btn-ghost" disabled={busy} onclick={() => create('code')}><Icon name="key" size={18} />{$t('msg_inv_create_code')}</button>
  </div>
  {#if error || (inviteStore.list && inviteStore.error)}<p class="warn">{error || inviteStore.error}</p>{/if}
</div>

<style>
  .invites { display: flex; flex-direction: column; gap: var(--sp-3); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; line-height: 1.45; }
  .warn { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
  .strip {
    display: flex; align-items: flex-start; gap: 8px; padding: 8px var(--sp-3); border-radius: var(--radius-sm); font-size: var(--fs-xs); line-height: 1.45;
    color: var(--warn-text); background: var(--warn-bg); border: 1px solid var(--warn-border);
  }
  .strip :global(svg) { flex-shrink: 0; margin-top: 2px; }
  /* Rows of 48 with a tile of 36 and a line between them (as the facts of «Профиль и ключ»). */
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; min-height: 48px; padding: var(--sp-1) 0; }
  .row + .row { border-top: 1px solid var(--border); }
  .row.ended { opacity: 0.6; }
  .kind {
    width: 36px; height: 36px; flex-shrink: 0; border-radius: var(--radius); background: var(--surface-3); border: 1px solid var(--border); color: var(--text-2);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .meta { font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; }
  .buttons { display: flex; gap: var(--sp-2); flex-shrink: 0; }
  .danger { color: var(--danger-text); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  @media (pointer: coarse) {
    .actions .btn { flex: 1 1 auto; min-height: 40px; justify-content: center; }
  }
</style>

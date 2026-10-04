<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { contactLabel, messengerApi, messengerError, type MessengerContact } from '../api';
  import ShareDialog from '../content/ShareDialog.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import Avatar from './Avatar.svelte';
  import ContactAddForm from './ContactAddForm.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';

  interface Props {
    onchat?: (pubkey: string) => void;
    /** The phone: its screen has the title and the add action; the contacts are a list, not a card. */
    compact?: boolean;
  }
  let { onchat, compact = false }: Props = $props();

  let busy = $state(false);
  let error = $state('');
  let openId = $state<string | null>(null);
  let editNick = $state('');
  let editNote = $state('');
  let nip05Result = $state<Record<string, boolean>>({});
  /** Whose link is being shared. */
  let sharing = $state<string | null>(null);
  let shareOpen = $state(false);

  async function run(fn: () => Promise<unknown>) {
    error = '';
    busy = true;
    try { await fn(); }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  // The phone: Write and Save stay buttons, the rest of a contact's actions are a sheet.
  let moreFor = $state<MessengerContact | null>(null);
  const moreItems = $derived.by<MenuEntry[]>(() => {
    const c = moreFor;
    if (!c) return [];
    return [
      { label: $t('msg_share_contact'), icon: 'send', onselect: () => { sharing = c.pubkey; shareOpen = true; } },
      {
        label: c.followed ? $t('msg_contacts_unfollow') : $t('msg_contacts_follow'),
        icon: 'user',
        onselect: () => void run(() => messengerStore.setFollowed(c.pubkey, !c.followed)),
      },
      { label: $t('msg_contacts_refresh'), icon: 'refresh-cw', onselect: () => void run(() => messengerStore.requestProfile(c.pubkey)) },
      ...(c.profile?.nip05
        ? [{
            label: $t('msg_contacts_verify_nip05') + (c.pubkey in nip05Result ? (nip05Result[c.pubkey] ? ' ✓' : ' ✗') : ''),
            icon: 'check-circle',
            onselect: () => void run(async () => { nip05Result = { ...nip05Result, [c.pubkey]: await messengerStore.verifyNip05(c.pubkey) }; }),
          }]
        : []),
      { type: 'separator' },
      { label: $t('msg_contacts_remove'), icon: 'trash-2', danger: true, onselect: () => void run(() => messengerStore.removeContact(c.pubkey)) },
    ];
  });

  function toggle(c: MessengerContact) {
    if (openId === c.pubkey) { openId = null; return; }
    openId = c.pubkey;
    editNick = c.nickname ?? '';
    editNote = c.note ?? '';
  }

</script>

<div class="contacts" class:card={!compact} class:compact>
  {#if !compact}<div class="card-title"><Icon name="user" size={16} /> {$t('msg_contacts_title')}</div>{/if}
  <p class="muted">{$t('msg_contacts_text')}</p>

  {#if !compact}<ContactAddForm />{/if}
  {#if error}<div class="error-msg">{error}</div>{/if}

  {#if messengerStore.contacts.length === 0}
    <div class="muted small empty">{$t('msg_contacts_empty')}</div>
  {:else}
    <ul class="list">
      {#each messengerStore.contacts as c (c.pubkey)}
        <li class="row" class:open={openId === c.pubkey}>
          <button class="head" onclick={() => toggle(c)}>
            <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} />
            <span class="info">
              <span class="name">
                {contactLabel(c)}
                {#if c.followed}<span class="tag">{$t('msg_contacts_following')}</span>{/if}
              </span>
              <span class="meta">
                {#if c.profile?.nip05}
                  <!-- The result of Verify stays here: on the phone the sheet that ran it is closed by then. -->
                  {@const checked = c.pubkey in nip05Result ? nip05Result[c.pubkey] : null}
                  {@const ok = checked ?? c.profile.nip05_verified}
                  <span
                    class:verified={ok}
                    class:unverified={checked === false}
                    title={ok ? $t('msg_contacts_nip05_ok') : checked === false ? $t('msg_contacts_nip05_failed') : undefined}
                    aria-live="polite"
                  >{c.profile.nip05}{ok ? ' ✓' : checked === false ? ' ✗' : ''}</span> ·
                {/if}
                <code>{c.npub.slice(0, 16)}…</code>
              </span>
            </span>
          </button>

          {#if openId === c.pubkey}
            <div class="details">
              {#if c.profile?.about}<div class="about"><MessageContent text={c.profile.about} cards={false} /></div>{/if}
              <div class="grid">
                <label><span>{$t('msg_contacts_nickname')}</span><input type="text" bind:value={editNick} disabled={busy} /></label>
                <label><span>{$t('msg_contacts_note')}</span><input type="text" bind:value={editNote} disabled={busy} /></label>
              </div>
              <div class="actions">
                {#if onchat}
                  <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => onchat(c.pubkey)}>
                    <Icon name="message-circle" size={12} />{$t('msg_contacts_write')}
                  </button>
                {/if}
                <button class="btn btn-ghost btn-sm" disabled={busy}
                  onclick={() => run(() => messengerStore.updateContact(c.pubkey, { nickname: editNick.trim() || null, note: editNote.trim() || null }))}>
                  {$t('msg_contacts_save')}
                </button>
                {#if compact}
                  <span class="spacer"></span>
                  <button class="btn btn-ghost btn-sm more" disabled={busy} onclick={() => (moreFor = c)} aria-label={$t('msg_contacts_more')} title={$t('msg_contacts_more')}>
                    <Icon name="more-horizontal" size={16} />
                  </button>
                {:else}
                <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => { sharing = c.pubkey; shareOpen = true; }}>
                  <Icon name="send" size={12} />{$t('msg_share_contact')}
                </button>
                <button class="btn btn-ghost btn-sm" disabled={busy}
                  onclick={() => run(() => messengerStore.setFollowed(c.pubkey, !c.followed))}>
                  {c.followed ? $t('msg_contacts_unfollow') : $t('msg_contacts_follow')}
                </button>
                <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => run(() => messengerStore.requestProfile(c.pubkey))}>
                  <Icon name="refresh-cw" size={12} />{$t('msg_contacts_refresh')}
                </button>
                {#if c.profile?.nip05}
                  <button class="btn btn-ghost btn-sm" disabled={busy}
                    onclick={() => run(async () => { nip05Result = { ...nip05Result, [c.pubkey]: await messengerStore.verifyNip05(c.pubkey) }; })}>
                    {$t('msg_contacts_verify_nip05')}{c.pubkey in nip05Result ? (nip05Result[c.pubkey] ? ' ✓' : ' ✗') : ''}
                  </button>
                {/if}
                <span class="spacer"></span>
                <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={() => run(() => messengerStore.removeContact(c.pubkey))}>
                  <Icon name="trash-2" size={12} />{$t('msg_contacts_remove')}
                </button>
                {/if}
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

{#if moreFor}
  <ContextMenu open x={0} y={0} items={moreItems} onclose={() => (moreFor = null)} />
{/if}

{#if sharing}
  {@const who = sharing}
  <ShareDialog bind:open={shareOpen} link={() => messengerApi.links.contact(who)} exclude={`dm:${who}`} />
{/if}

<style>
  .contacts { max-width: 680px; width: 100%; margin-inline: auto; display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .row { border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--surface); }
  .row.open { border-color: var(--accent-border); }
  .head {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; text-align: left;
    background: none; border: none; color: inherit; font: inherit; cursor: pointer; padding: var(--sp-2) var(--sp-3);
  }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { display: flex; align-items: center; gap: 6px; font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .tag { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.5px; padding: 1px 6px; border-radius: var(--radius-sm); background: var(--accent-tint); color: var(--accent-text-2); }
  .meta { color: var(--text-3); font-size: var(--fs-xs); }
  .meta code { font-family: var(--font-mono); }
  .verified { color: var(--success-text); }
  .unverified { color: var(--danger-text); }
  .details { display: flex; flex-direction: column; gap: var(--sp-2); padding: 0 var(--sp-3) var(--sp-3); border-top: 1px solid var(--border); padding-top: var(--sp-2); }
  .about { margin: 0; font-size: var(--fs-sm); color: var(--text-body); white-space: pre-wrap; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: var(--sp-2); }
  .grid label { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); }
  .grid input {
    font: inherit; font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 6px 8px;
  }
  .actions { display: flex; gap: var(--sp-1); flex-wrap: wrap; align-items: center; }
  .spacer { flex: 1; }
  .btn.danger { color: var(--danger-text); }

  /* The phone: one grouped list of full-width rows, like the shell's lists (mobile.css .m-list). */
  .compact .muted { font-size: 14px; line-height: 1.45; padding: 0 var(--sp-1); }
  .compact .empty { text-align: center; padding: var(--sp-6) var(--sp-4); font-size: var(--fs-sm); }
  .compact .list {
    gap: 0; overflow: hidden; background: var(--m-card, var(--surface));
    border: 1px solid var(--m-card-border, var(--border)); border-radius: var(--m-radius, var(--radius-lg));
  }
  .compact .row { border: none; border-radius: 0; background: none; }
  .compact .row + .row { border-top: 1px solid var(--m-card-border, var(--border)); }
  .compact .row.open { background: var(--surface-2); }
  .compact .head { min-height: 64px; padding: var(--sp-2) var(--sp-4); border-radius: 0; }
  .compact .head:active { background: var(--surface-2); }
  .compact .name { font-size: 15px; }
  .compact .meta { font-size: 13px; }
  .compact .details { padding: var(--sp-3) var(--sp-4) var(--sp-4); border-top-color: var(--m-card-border, var(--border)); }
  .compact .grid input { font-size: 16px; padding: 8px 10px; border-radius: var(--radius); }
</style>

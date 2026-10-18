<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A contact in the pane of a computer's «Контакты»: who it is (the picture,
  the name, NIP-05 and the key), «Написать», a call or a video call when
  a call can be made, «Поделиться»; what the profile says about them; my
  nickname and note of them, kept on this device; following them publicly,
  a fresh copy of the profile, removal.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { copied, toast } from '$lib/core/ui/toast.svelte';
  import { contactLabel, messengerError, type MessengerContact } from '../api';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import ShareDialog from '../content/ShareDialog.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import CallIcon from '../calls/CallIcon.svelte';
  import { dialable, dialBlock, dialer, targetOf } from '../calls/dial.svelte';
  import BioView from './BioView.svelte';
  import SocialLinks from './SocialLinks.svelte';
  import Avatar from './Avatar.svelte';
  import PeerChats from '../servers/PeerChats.svelte';

  interface Props {
    contact: MessengerContact;
    onchat: (pubkey: string) => void;
    /** Shows one chat with them: the home one or a private one («Частный · S1»). */
    onopenchat?: (chatId: string) => void;
    /** The contact was removed: nothing is left to show. */
    onremoved?: () => void;
  }
  let { contact, onchat, onopenchat, onremoved }: Props = $props();

  const name = $derived(contactLabel(contact));
  const chat = $derived(chatStore.chats.find((c) => c.id === `dm:${contact.pubkey}`) ?? null);
  const callable = $derived(dialable(chat) && chat.kind === 'dm');
  const block = $derived(callable ? dialBlock() : $t('msg_call_err_contacts'));

  let nick = $state('');
  let note = $state('');
  let busy = $state(false);
  /** The answer of «Проверить NIP-05» for the contact shown. */
  let nip05 = $state<boolean | null>(null);
  let shareOpen = $state(false);
  let shown = '';
  // The fields follow the contact shown; an edit of the same one stays while it is saved.
  $effect(() => {
    if (contact.pubkey === shown) return;
    shown = contact.pubkey;
    nick = contact.nickname ?? '';
    note = contact.note ?? '';
    nip05 = null;
  });
  const changed = $derived(nick.trim() !== (contact.nickname ?? '') || note.trim() !== (contact.note ?? ''));

  async function run(fn: () => Promise<unknown>) {
    busy = true;
    try { await fn(); }
    catch (e) { toast.error(messengerError(e)); }
    finally { busy = false; }
  }

  const save = () => run(() => messengerStore.updateContact(contact.pubkey, { nickname: nick.trim() || null, note: note.trim() || null }));
  const verify = () => run(async () => { nip05 = await messengerStore.verifyNip05(contact.pubkey); });

  async function copyKey() {
    try {
      await navigator.clipboard.writeText(contact.npub);
      copied();
    } catch (e) {
      toast.error(messengerError(e));
    }
  }

  async function remove() {
    if (!(await confirmStore.ask($t('msg_contacts_remove_confirm', { name }), $t('msg_contacts_remove'), true))) return;
    await run(async () => {
      await messengerStore.removeContact(contact.pubkey);
      onremoved?.();
    });
  }

  const verified = $derived(nip05 ?? contact.profile?.nip05_verified ?? false);
</script>

<div class="details">
  <div class="who">
    <Avatar url={contact.profile?.picture ?? null} label={name} seed={contact.pubkey} size={88} />
    <h2 class="name">{name}</h2>
    {#if contact.nickname && contact.profile && contact.nickname.trim() !== (contact.profile.display_name || contact.profile.name)}
      <span class="sub">{contact.profile.display_name || contact.profile.name}</span>
    {/if}
    {#if contact.profile?.nip05}
      <span class="nip05" class:ok={verified} class:bad={nip05 === false}
        title={verified ? $t('msg_contacts_nip05_ok') : nip05 === false ? $t('msg_contacts_nip05_failed') : undefined}>
        {contact.profile.nip05}{verified ? ' ✓' : nip05 === false ? ' ✗' : ''}
      </span>
    {/if}
    {#if contact.followed}<span class="tag">{$t('msg_contacts_following')}</span>{/if}
  </div>

  <div class="actions">
    <button class="act primary" onclick={() => onchat(contact.pubkey)}>
      <span class="disc"><Icon name="message-circle" size={18} /></span>{$t('msg_contacts_write')}
    </button>
    <button class="act call" disabled={!!block} title={block || undefined} onclick={() => chat && dialer.call(targetOf(chat), 'audio', false)}>
      <span class="disc"><CallIcon name="phone" size={18} /></span>{$t('msg_calls_audio_short')}
    </button>
    <button class="act call" disabled={!!block} title={block || undefined} onclick={() => chat && dialer.call(targetOf(chat), 'video', false)}>
      <span class="disc"><CallIcon name="video" size={18} /></span>{$t('msg_calls_video_short')}
    </button>
    <button class="act" onclick={() => (shareOpen = true)}>
      <span class="disc"><Icon name="send" size={18} /></span>{$t('msg_contacts_share_short')}
    </button>
  </div>

  <PeerChats pubkey={contact.pubkey} onopen={(id) => (onopenchat ? onopenchat(id) : onchat(contact.pubkey))} />

  {#if contact.profile?.bio?.length || contact.profile?.about || contact.profile?.socials?.length}
    <section class="block">
      <h3>{$t('msg_own_about')}</h3>
      {#if contact.profile?.bio?.length}<BioView spans={contact.profile.bio} />{:else if contact.profile?.about}<div class="about"><MessageContent text={contact.profile.about} cards={false} /></div>{/if}
      {#if contact.profile?.socials?.length}<SocialLinks socials={contact.profile.socials} compact />{/if}
    </section>
  {/if}

  <section class="block">
    <h3>{$t('msg_id_npub')}</h3>
    <div class="key">
      <code>{contact.npub}</code>
      <button class="icon" onclick={copyKey} title={$t('msg_copy')} aria-label={$t('msg_copy')}><Icon name="copy" size={15} /></button>
    </div>
  </section>

  <section class="block">
    <h3>{$t('msg_contacts_book')}</h3>
    <div class="grid">
      <label><span>{$t('msg_contacts_nickname')}</span><input type="text" bind:value={nick} disabled={busy} placeholder={contact.profile?.display_name || contact.profile?.name || ''} /></label>
      <label><span>{$t('msg_contacts_note')}</span><input type="text" bind:value={note} disabled={busy} /></label>
    </div>
    {#if changed}
      <div class="save"><button class="btn btn-primary btn-sm" disabled={busy} onclick={save}>{$t('msg_contacts_save')}</button></div>
    {/if}
  </section>

  <div class="more">
    <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => run(() => messengerStore.setFollowed(contact.pubkey, !contact.followed))}>
      <Icon name="user" size={13} />{contact.followed ? $t('msg_contacts_unfollow') : $t('msg_contacts_follow')}
    </button>
    <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => run(() => messengerStore.requestProfile(contact.pubkey))}>
      <Icon name="refresh-cw" size={13} />{$t('msg_contacts_refresh')}
    </button>
    {#if contact.profile?.nip05}
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={verify}><Icon name="rosette-discount-check" size={13} />{$t('msg_contacts_verify_nip05')}</button>
    {/if}
    <span class="spacer"></span>
    <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={remove}><Icon name="trash-2" size={13} />{$t('msg_contacts_remove')}</button>
  </div>
</div>

<ShareDialog bind:open={shareOpen} card={{ pubkey: contact.pubkey }} exclude={`dm:${contact.pubkey}`} />


<style>
  .details { display: flex; flex-direction: column; gap: var(--sp-5); padding: var(--sp-6) var(--sp-5); max-width: 560px; width: 100%; margin: 0 auto; }
  .who { display: flex; flex-direction: column; align-items: center; gap: var(--sp-1); text-align: center; }
  .name { margin: var(--sp-3) 0 0; font-size: var(--fs-xl); font-weight: var(--fw-bold); overflow-wrap: anywhere; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); }
  .nip05 { font-size: var(--fs-sm); color: var(--text-2); }
  .nip05.ok { color: var(--success-text); }
  .nip05.bad { color: var(--danger-text); }
  .tag { margin-top: var(--sp-1); font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.5px; padding: 1px 8px; border-radius: var(--radius-pill); background: var(--accent-tint); color: var(--accent-text-2); }
  .actions { display: flex; justify-content: center; gap: var(--sp-2); flex-wrap: wrap; }
  .act {
    display: flex; flex-direction: column; align-items: center; gap: 6px; min-width: 84px; padding: var(--sp-2) var(--sp-3);
    border: none; border-radius: var(--radius-lg); background: none; color: var(--text-2); font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); cursor: pointer;
  }
  .disc { width: 48px; height: 48px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; background: var(--surface-3); color: var(--text); }
  .act.primary .disc { background: var(--accent-bg); color: var(--accent-text); }
  .act.call .disc { background: var(--success-bg); color: var(--success-text); }
  @media (hover: hover) { .act:hover:not(:disabled) { background: var(--surface-hover); color: var(--text); } }
  .act:disabled { opacity: 0.45; cursor: default; }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-4); border-radius: var(--radius-lg); background: var(--surface-2); border: 1px solid var(--border); }
  .block h3 { margin: 0; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); }
  .about { margin: 0; font-size: var(--fs-sm); color: var(--text-body); white-space: pre-wrap; }
  .key { display: flex; align-items: center; gap: var(--sp-2); }
  .key code { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body); overflow-wrap: anywhere; }
  .icon { border: none; background: none; color: var(--text-soft); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; width: 32px; height: 32px; flex-shrink: 0; border-radius: var(--radius); }
  .icon:hover { color: var(--text); background: var(--surface-hover); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: var(--sp-2); }
  .grid label { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); }
  .grid input {
    font: inherit; font-size: var(--fs-sm); color: var(--text);
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 10px;
  }
  .grid input:focus { outline: none; border-color: var(--accent-border); }
  .save { display: flex; justify-content: flex-end; }
  .more { display: flex; gap: var(--sp-1); flex-wrap: wrap; align-items: center; }
  .spacer { flex: 1; }
  .btn.danger { color: var(--danger-text); }
</style>

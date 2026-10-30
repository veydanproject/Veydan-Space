<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Everything about the person of a conversation of two except the
  conversation: who they are, their key, and what the chat has shared.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { reportStore } from '../shared/report.svelte';
  import PanelHead from '../shared/PanelHead.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import CopyField from '../identity/CopyField.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import BioView from '../contacts/BioView.svelte';
  import SocialLinks from '../contacts/SocialLinks.svelte';
  import { linkActions, openChat } from '../content/actions';
  import PeerChats from '../servers/PeerChats.svelte';
  import ServerLabel from '../servers/ServerLabel.svelte';
  import type { ExternalUrl } from '../content/types';
  import { messengerStore } from '../store.svelte';
  import SharedMedia from '../content/shared/SharedMedia.svelte';
  import SharedList from '../content/shared/SharedList.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { presenceStore } from '../presence/presenceStore.svelte';
  import type { MessengerChat, SharedSection } from '../api';

  interface Props {
    chat: MessengerChat;
    onclose: () => void;
  }
  let { chat, onclose }: Props = $props();

  /** A section of what the chat has shared, shown in place of the panel. */
  let section = $state<SharedSection | null>(null);

  function report() {
    if (!chat.peer_pubkey) return;
    reportStore.open({ accused: chat.peer_pubkey, name: chat.title, context: { t: 'dm' }, canBlock: chat.mode !== 'blocked' });
  }

  const profile = $derived(chat.peer_pubkey ? nameStore.profile(chat.peer_pubkey) : null);
  const name = $derived(profile?.display_name?.trim() || profile?.name?.trim() || '');
  const online = $derived(presenceStore.status(chat.peer_pubkey) === 'online');
  const presenceText = $derived(presenceStore.label(chat.peer_pubkey));
  const website = $derived(profile?.website && /^https?:\/\/\S+$/i.test(profile.website) ? profile.website : null);
  /** The phone the person sent me in their own card; kept fresh by `contact_private.updated`. */
  const phone = $derived(chat.peer_pubkey ? messengerStore.contactPhone(chat.peer_pubkey) ?? null : null);
</script>

<div class="panel">
  {#if section}
    <SharedList chatId={chat.id} {section} onback={() => (section = null)} />
  {:else}
    <PanelHead title={$t('msg_peer_info')} {onclose} />

    <div class="scroll">
      <!-- Who: the avatar beside the name, what they wrote about themselves under a line. -->
      <section class="hero">
        <div class="hero-row">
          <Avatar url={chat.picture} label={chat.title} seed={chat.peer_pubkey ?? chat.id} size={60} {online} />
          <div class="hero-text">
            <div class="name">{chat.title}</div>
            {#if chat.server_id}<ServerLabel server={chat.server_id} size="md" info />{/if}
            {#if presenceText}<div class="sub" class:online>{presenceText}</div>{/if}
            {#if name && name !== chat.title}<div class="sub">{name}</div>{/if}
            {#if profile?.nip05}
              <div class="sub nip05" class:ok={profile.nip05_verified}>
                <Icon name={profile.nip05_verified ? 'rosette-discount-check' : 'world'} size={14} />{profile.nip05}
              </div>
            {/if}
          </div>
        </div>
        {#if profile?.bio?.length}
          <div class="about">
            <span class="about-label">{$t('msg_peer_bio')}</span>
            <BioView spans={profile.bio} />
          </div>
        {:else if profile?.about}
          <div class="about"><MessageContent text={profile.about} cards={false} /></div>
        {/if}
      </section>

      {#if profile?.socials?.length || website}
        <section class="box">
          {#if profile?.socials?.length}
            <span class="label">{$t('msg_peer_links')}</span>
            <SocialLinks socials={profile.socials} />
          {/if}
          {#if website}
            <span class="label">{$t('msg_peer_website')}</span>
            <button type="button" class="site" title={website} onclick={() => void linkActions.follow(website as ExternalUrl)}>
              <Icon name="world" size={16} /><span class="site-text">{website.replace(/^https?:\/\//i, '').replace(/\/$/, '')}</span><Icon name="external-link" size={14} />
            </button>
          {/if}
        </section>
      {/if}

      {#if phone || chat.peer_npub}
        <section class="box keys">
          {#if phone}
            <div class="field">
              <CopyField label={$t('msg_peer_phone')} value={phone} />
              <span class="hint">{$t('msg_peer_phone_hint')}</span>
            </div>
          {/if}
          {#if chat.peer_npub}<div class="field"><CopyField label={$t('msg_peer_key')} value={chat.peer_npub} mono /></div>{/if}
        </section>
      {/if}

      {#if chat.peer_pubkey && !chat.bot}
        <PeerChats pubkey={chat.peer_pubkey} current={chat.id} onopen={(id) => void openChat(id).catch(() => {})} />
      {/if}

      <SharedMedia chatId={chat.id} onopen={(s) => (section = s)} />

      <!-- The last: what cannot be undone, apart. -->
      {#if chat.peer_pubkey && !chat.bot}
        <section class="zone">
          <button type="button" class="btn btn-ghost danger" onclick={report}><Icon name="alert-triangle" size={18} />{$t('msg_report')}</button>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4) var(--sp-4) var(--sp-6); display: flex; flex-direction: column; gap: var(--sp-4); }
  /* A tablet or a wide window: the column stays readable in the middle. */
  .scroll > :global(*) { width: 100%; max-width: 560px; align-self: center; flex-shrink: 0; }

  /* Who: the avatar beside the name; «about» under a line. */
  .hero { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hero-row { display: flex; align-items: center; gap: var(--sp-3); }
  .hero-row > :global(.avatar), .hero-row > :global(.holder) { flex: none; }
  .hero-text { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 2px; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); line-height: 1.25; overflow-wrap: anywhere; word-break: break-word; }
  .sub { display: inline-flex; align-items: center; gap: var(--sp-1); font-size: var(--fs-sm); color: var(--text-3); overflow-wrap: anywhere; word-break: break-word; }
  .sub.online { color: var(--success-text, var(--success)); }
  .nip05 { color: var(--text-2); }
  .nip05 :global(svg) { flex: none; }
  .nip05.ok :global(svg) { color: var(--success-text, var(--success)); }
  .about { display: flex; flex-direction: column; gap: var(--sp-1); padding-top: var(--sp-3); border-top: 1px solid var(--border-2); font-size: var(--fs-base); line-height: var(--lh-base); color: var(--text); }
  .about-label { font-size: var(--fs-xs); font-weight: var(--fw-bold); color: var(--text-2); }

  /* The cards: the links and the site; the phone and the key, each with its copy. */
  .box { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; padding: var(--sp-3); border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  .label { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .box > :global(.socials) { margin-bottom: var(--sp-1); }
  .site {
    display: flex; align-items: center; gap: var(--sp-2); width: 100%; min-width: 0; min-height: var(--control-h); padding: 0 var(--sp-3); text-align: left; cursor: pointer;
    font: inherit; font-size: var(--fs-sm); color: var(--accent-text-2);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); transition: border-color var(--dur-fast) var(--ease);
  }
  .site:hover { border-color: var(--accent-border); }
  .site:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .site :global(svg) { flex: none; }
  .site :global(svg:last-child) { color: var(--text-3); }
  .site-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .field { display: flex; flex-direction: column; gap: var(--sp-1); min-width: 0; }
  .keys { gap: var(--sp-3); }
  .keys > .field + .field { padding-top: var(--sp-3); border-top: 1px solid var(--border-2); }
  .keys :global(.copy-field .label) { font-size: var(--fs-xs); font-weight: var(--fw-bold); color: var(--text-2); }
  .keys :global(.copy-field code) { padding: var(--sp-2) var(--sp-3); background: var(--surface-3); border: 1px solid var(--border); font-size: var(--fs-xs); color: var(--text-2); }
  .hint { font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }

  /* The last: what cannot be undone, apart. */
  .zone { display: flex; flex-direction: column; gap: var(--sp-2); margin-top: var(--sp-2); padding-top: var(--sp-4); border-top: 1px dashed var(--border-2); }
  .zone .btn { min-height: var(--control-h); justify-content: flex-start; gap: var(--sp-2); padding: 0 var(--sp-4); }
  .danger { color: var(--danger-text); }

  @media (pointer: coarse) {
    .zone .btn { min-height: var(--control-h-lg); }
  }
</style>

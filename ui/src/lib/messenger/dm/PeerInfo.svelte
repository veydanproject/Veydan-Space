<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Everything about the person of a conversation of two except the
  conversation: who they are, their key, and what the chat has shared.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import PanelHead from '../shared/PanelHead.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import CopyField from '../identity/CopyField.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import BioView from '../contacts/BioView.svelte';
  import SocialLinks from '../contacts/SocialLinks.svelte';
  import { linkActions } from '../content/actions';
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
      <section class="card-top">
        <Avatar url={chat.picture} label={chat.title} seed={chat.peer_pubkey ?? chat.id} size={64} {online} />
        <div class="name">{chat.title}</div>
        {#if presenceText}<div class="sub" class:online>{presenceText}</div>{/if}
        {#if name && name !== chat.title}<div class="sub">{name}</div>{/if}
        {#if profile?.nip05}
          <div class="sub nip05" class:ok={profile.nip05_verified}>
            <Icon name={profile.nip05_verified ? 'check-circle' : 'globe'} size={12} />{profile.nip05}
          </div>
        {/if}
        {#if !profile?.bio?.length && profile?.about}<div class="about"><MessageContent text={profile.about} cards={false} /></div>{/if}
      </section>

      {#if profile?.bio?.length}
        <section class="field">
          <span class="label">{$t('msg_peer_bio')}</span>
          <BioView spans={profile.bio} />
        </section>
      {/if}

      {#if profile?.socials?.length}
        <section class="field">
          <span class="label">{$t('msg_peer_links')}</span>
          <SocialLinks socials={profile.socials} />
        </section>
      {/if}

      {#if website}
        <section class="field">
          <span class="label">{$t('msg_peer_website')}</span>
          <button type="button" class="site" title={website} onclick={() => linkActions.openExternal(website as ExternalUrl).catch(() => {})}>
            <Icon name="globe" size={14} /><span class="site-text">{website.replace(/^https?:\/\//i, '').replace(/\/$/, '')}</span><Icon name="external-link" size={12} />
          </button>
        </section>
      {/if}

      {#if phone}
        <div class="field">
          <CopyField label={$t('msg_peer_phone')} value={phone} />
          <span class="hint">{$t('msg_peer_phone_hint')}</span>
        </div>
      {/if}

      {#if chat.peer_npub}<CopyField label={$t('msg_peer_key')} value={chat.peer_npub} mono />{/if}

      <SharedMedia chatId={chat.id} onopen={(s) => (section = s)} />
    </div>
  {/if}
</div>

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
  .card-top { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-extrabold); letter-spacing: -0.2px; overflow-wrap: anywhere; word-break: break-word; }
  .sub { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; word-break: break-word; }
  .sub.online { color: var(--accent); }
  .nip05.ok { color: var(--success-text, var(--success)); }
  .about { color: var(--text-body); }
  .field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .label { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .hint { font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; }
  .site {
    display: flex; align-items: center; gap: 8px; width: 100%; min-width: 0; text-align: left; cursor: pointer;
    font: inherit; font-size: var(--fs-sm); color: var(--accent-text-2);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  .site:hover { border-color: var(--accent-border); }
  .site :global(svg) { flex-shrink: 0; }
  .site-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>

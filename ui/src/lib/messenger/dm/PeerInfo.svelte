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
        {#if profile?.about}<div class="about"><MessageContent text={profile.about} cards={false} /></div>{/if}
      </section>

      {#if chat.peer_npub}<CopyField label={$t('msg_peer_key')} value={chat.peer_npub} mono />{/if}

      <SharedMedia chatId={chat.id} onopen={(s) => (section = s)} />
    </div>
  {/if}
</div>

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
  .card-top { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-extrabold); letter-spacing: -0.2px; overflow-wrap: anywhere; }
  .sub { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; }
  .sub.online { color: var(--accent); }
  .nip05.ok { color: var(--success-text, var(--success)); }
  .about { color: var(--text-body); }
</style>

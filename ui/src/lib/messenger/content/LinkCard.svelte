<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The card of one link: where a link outside leads, what a link inside
  leads to, or a wait while the runtime is asked. The same card under a
  message and in the list of what a chat has shared.

  A link of a channel is the business of the module that gives the section
  owning it (seams/sections: Communities): a channel here is shown as its
  row and opened at once; any other goes to the module's dialog with the
  link, which shows the channel's card and lets in or takes a request.
  Without that module the link is one this product cannot open.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatSections } from '../seams/sections.svelte';
  import { openChat } from './actions';
  import { cardOf } from './cards';
  import ExternalCard from './cards/ExternalCard.svelte';
  import UnknownCard from './cards/UnknownCard.svelte';
  import { linkStore } from './linkStore.svelte';
  import type { LinkSegment } from './types';

  interface Props {
    link: LinkSegment;
    /** Whether a channel not here yet asks its relay for its name. A link
        from outside the app does not: the relay is the link's, and the
        user has not pressed anything yet. */
    preview?: boolean;
  }
  let { link: l, preview = true }: Props = $props();

  const view = $derived(l.type === 'internal' ? linkStore.view(l.link) : null);

  const channelChat = $derived(view?.kind === 'channel' ? `channel:${view.community_id}` : null);
  const channelHost = $derived(channelChat ? chatSections.ofChat(channelChat) : null);
  /** The channel's row when it is here already: its name and cover. */
  const channelRow = $derived(channelHost ? (channelHost.rows().find((r) => r.chatId === channelChat) ?? null) : null);
  let opening = $state(false);

  /** The channel's name from its card, while it is not here: the section asks its relay. */
  let cardName = $state<string | null>(null);
  $effect(() => {
    const host = channelHost;
    const v = view;
    cardName = null;
    if (!preview || !host?.linkTitle || v?.kind !== 'channel' || channelRow) return;
    let alive = true;
    host.linkTitle(v.link).then((n) => { if (alive) cardName = n; }).catch(() => {});
    return () => { alive = false; };
  });
  const channelName = $derived(channelRow?.title ?? cardName);

  /** The product has channels and the user switched them off: the link says so and leads to the switch. */
  const channelsOff = $derived(chatSections.isOff(channelChat));

  function openChannel() {
    if (channelRow) void openChat(channelRow.chatId);
    else opening = true;
  }
</script>

<div class="link" class:pending={l.type === 'internal' && !view}>
  {#if l.type === 'external'}
    <ExternalCard url={l.url} />
  {:else if view?.kind === 'channel'}
    {#if channelHost}
      {@const Create = channelHost.create}
      <div class="channel">
        {#if channelRow}
          <Avatar url={channelRow.picture?.startsWith('data:') ? null : channelRow.picture} src={channelRow.picture?.startsWith('data:') ? channelRow.picture : null}
            label={channelRow.title} seed={channelRow.seed} size={40} />
        {:else}
          <span class="ico"><Icon name={channelHost.icon} size={18} /></span>
        {/if}
        <div class="about">
          <span class="name">{channelName ?? $t('msg_card_channel')}</span>
          {#if channelName}<span class="sub"><Icon name={channelHost.icon} size={11} />{$t('msg_card_channel')}</span>{/if}
        </div>
        <button class="btn btn-primary btn-sm" onclick={openChannel}>{$t('msg_card_open')}</button>
      </div>
      <Create bind:open={opening} link={view.link} onopened={(id) => { void openChat(id); }} />
    {:else if channelsOff}
      <div class="channel">
        <span class="ico"><Icon name="speakerphone" size={18} /></span>
        <div class="about">
          <span class="name">{$t('msg_card_channel')}</span>
          <span class="sub wrap">{$t('msg_card_channels_off')}</span>
        </div>
        <button class="btn btn-ghost btn-sm" onclick={() => void goto('/settings#modules')}><Icon name="settings" size={13} />{$t('msg_card_channels_settings')}</button>
      </div>
    {:else}
      <UnknownCard view={{ kind: 'unknown', link_type: 'channel' }} text={l.text} />
    {/if}
  {:else if view}
    {@const Card = cardOf(view)}
    <Card {view} text={l.text} />
  {:else}
    <span class="wait"><Icon name="loader" size={14} />{l.text}</span>
  {/if}
</div>

<style>
  .link {
    padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md); border: 1px solid var(--border);
    background: var(--surface-2); color: var(--text); min-width: 0;
  }
  .link.pending { padding: var(--sp-2) var(--sp-3); }
  .wait { display: flex; align-items: center; gap: 6px; font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wait :global(svg) { flex-shrink: 0; animation: spin 1.1s linear infinite; }
  .channel { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .ico {
    width: 40px; height: 40px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--accent-tint); color: var(--accent-text-2);
  }
  .about { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .channel .btn { flex-shrink: 0; }
  .sub.wrap { white-space: normal; line-height: 1.35; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A contact card sent as a message: who the person is, as the runtime
  checked the card (picture as a `data:` URL, bio as spans, links checked),
  and what to do: add them, write to them. A phone is in a card only when
  its owner put it in their own card; it is copied, never dialled.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Notice from '$lib/core/ui/Notice.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import BioView from '../contacts/BioView.svelte';
  import SocialLinks from '../contacts/SocialLinks.svelte';
  import { messengerStore } from '../store.svelte';
  import { dmErrorCode, messengerApi, messengerError, profileErrorCode, type MessengerMessage, type ServerPreview } from '../api';
  import ServerBox from '../servers/ServerBox.svelte';
  import { cardActions, linkActions } from './actions';
  import type { ExternalUrl } from './types';

  interface Props { message: MessengerMessage }
  let { message: m }: Props = $props();

  const card = $derived(m.card ?? null);
  const shortKey = $derived(card ? `${card.npub.slice(0, 12)}…${card.npub.slice(-4)}` : '');
  const label = $derived(card?.label.trim() || shortKey);
  /** The other name, when the card shows one name and has another. */
  const second = $derived.by(() => {
    if (!card) return '';
    const n = card.name?.trim() ?? '';
    return n && n.toLowerCase() !== label.toLowerCase() ? n : '';
  });
  // The card says what was true when it was shown; the contacts say what is true now.
  const known = $derived(!!card && (card.is_contact || messengerStore.contacts.some((c) => c.pubkey === card.pubkey)));
  const website = $derived(card?.website && /^https?:\/\/\S+$/i.test(card.website) ? card.website : null);
  const websiteText = $derived(website ? website.replace(/^https?:\/\//i, '').replace(/\/$/, '') : '');
  /** A phone kept on "Add contact": only from the sender's own card. */
  const ownCard = $derived(!!card && m.direction === 'in' && card.pubkey === m.sender_pubkey);

  let busy = $state(false);
  let error = $state('');
  let added = $state(false);

  // A long bio: three lines and "Show more".
  let expanded = $state(false);
  let cut = $state(false);
  let bioBox = $state<HTMLElement | null>(null);
  $effect(() => {
    const el = bioBox?.firstElementChild as HTMLElement | null | undefined;
    if (!el || expanded) return;
    void card?.bio;
    const measure = () => { cut = el.scrollHeight > el.clientHeight + 1; };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) {
      const code = profileErrorCode(e);
      const dm = code ? null : dmErrorCode(e);
      error = code ? $t(`msg_err_${code}` as 'msg_err_card_unknown')
        : dm ? $t(`msg_err_${dm}` as 'msg_err_dm_blocked', { name: label })
        : messengerError(e);
    }
    finally { busy = false; }
  }

  const accept = () => run(async () => { await cardActions.accept(m.id); added = true; });
  const write = () => card && run(() => cardActions.write(card.pubkey));
  const openPrivate = () => card && run(() => cardActions.openPrivate(card));

  // The server the card offers a private chat on, as the runtime reads its card.
  let server = $state<ServerPreview | null>(null);
  $effect(() => {
    const link = card?.server ?? null;
    server = null;
    if (!link) return;
    let live = true;
    messengerApi.servers.preview(link).then((s) => { if (live) server = s; }).catch(() => {});
    return () => { live = false; };
  });

  function copyPhone() {
    if (!card?.phone) return;
    // «Скопировано» is the toast of the copy; the row stays as it is.
    linkActions.copy(card.phone).catch(() => {});
  }

  function openSite() {
    if (website) void linkActions.follow(website as ExternalUrl);
  }
</script>

{#if !card}
  <span class="damaged"><Icon name="alert-triangle" size={12} />{$t('msg_err_card_invalid')}</span>
{:else}
  <div class="contact-card">
    <span class="kicker"><Icon name="user" size={14} />{$t('msg_card_contact_title')}</span>
    <div class="top">
      <Avatar url={null} src={card.avatar} {label} seed={card.pubkey} size={52} />
      <div class="who">
        <span class="name" title={label}>{label}</span>
        <span class="sub">{#if second}{second}{:else}<code>{shortKey}</code>{/if}</span>
      </div>
    </div>

    {#if card.bio.length}
      <div class="bio-box" bind:this={bioBox}>
        <BioView spans={card.bio} compact={!expanded} lines={3} />
      </div>
      {#if cut || expanded}
        <button type="button" class="more" onclick={() => (expanded = !expanded)}>
          {expanded ? $t('msg_card_less') : $t('msg_card_more')}
        </button>
      {/if}
    {/if}

    {#if website || card.phone || card.socials.length}
      <!-- Site, phone, profiles: one list, a line between rows. -->
      <div class="links">
        {#if website}
          <button type="button" class="fact" onclick={openSite} title={website}>
            <Icon name="world" size={16} /><span class="value">{websiteText}</span><Icon name="external-link" size={14} />
          </button>
        {/if}
        {#if card.phone}
          <button type="button" class="fact" onclick={copyPhone}
            title={ownCard ? $t('msg_card_phone_hint') : $t('msg_card_copy')} aria-label="{$t('msg_card_phone')}: {card.phone}. {$t('msg_card_copy')}">
            <Icon name="phone" size={16} /><span class="value">{card.phone}</span>
            <span class="copy"><Icon name="copy" size={12} /></span>
          </button>
        {/if}
        {#if card.socials.length}<SocialLinks socials={card.socials} />{/if}
      </div>
    {/if}

    {#if server}
      <!-- The sender's server: a private chat with them there. -->
      <div class="server">
        <span class="offer"><Icon name="message-circle" size={13} />{$t('msg_srv_card_private_chat')}</span>
        <ServerBox {server} />
        {#if m.direction === 'in' && !card.is_me && !card.blocked}
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={openPrivate}>
            <Icon name="message-circle" size={13} />{$t('msg_srv_card_open_private')}
          </button>
        {/if}
      </div>
    {/if}

    {#if card.is_me}
      <div class="status"><Icon name="user" size={12} />{$t('msg_card_this_is_you')}</div>
    {:else if card.blocked}
      <div class="status closed"><Icon name="user-cancel" size={12} />{$t('msg_card_blocked')}</div>
    {:else}
      {#if error}<Notice tone="danger">{error}</Notice>{/if}
      <div class="row">
        {#if known}
          <button class="btn btn-primary" disabled={busy} onclick={write}>
            <Icon name="message-circle" size={18} />{$t('msg_card_write')}
          </button>
          <span class="known"><Icon name="check" size={12} />{added ? $t('msg_card_added') : $t('msg_card_already_contact')}</span>
        {:else}
          <button class="btn btn-primary" disabled={busy} onclick={accept}>
            <Icon name="user-plus" size={18} />{$t('msg_card_add_contact')}
          </button>
          <button class="btn btn-ghost" disabled={busy} onclick={write}>
            <Icon name="message-circle" size={18} />{$t('msg_card_write')}
          </button>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  /* The bubble is the card: no box inside it. As wide as its content, 320 to
     420, as a link's cards; a phone's is the bubble's width. */
  .contact-card {
    display: flex; flex-direction: column; gap: 14px;
    width: fit-content; min-width: min(320px, 100%); max-width: min(420px, 100%); color: var(--text);
  }
  :global(.msg-m) .contact-card { width: 100%; max-width: 100%; }
  .kicker {
    display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    color: var(--text-3);
  }
  /* «Контакт» 10 above the avatar. */
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; margin-top: -4px; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub code { font-family: var(--font-mono); }
  .bio-box { min-width: 0; }
  .more {
    align-self: flex-start; border: none; background: none; padding: 0; margin-top: -4px; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2);
  }
  .more:hover { text-decoration: underline; }
  /* Site, phone and profiles one list in the bubble's tone, a line between
     rows: 40, a phone's 44; the sign 16, the text 14. A profile's chip a row. */
  .links {
    --links-line: color-mix(in srgb, currentColor 10%, transparent);
    display: flex; flex-direction: column; border-radius: var(--radius-md); background: var(--bubble-tone, var(--surface-2)); overflow: hidden;
  }
  .links :global(.socials) { display: contents; }
  .fact, .links :global(.social) {
    display: flex; align-items: center; gap: 10px; width: 100%; max-width: none; min-width: 0; min-height: var(--control-h);
    margin: 0; padding: 0 var(--sp-3); border: 0; border-radius: 0; background: none; color: inherit;
    font: inherit; font-size: var(--fs-base); text-align: left; cursor: pointer; box-shadow: inset 0 1px 0 var(--links-line);
  }
  .links > .fact:first-child, .links > :global(.socials:first-child > .social:first-child) { box-shadow: none; }
  .fact:hover, .links :global(.social:hover) { background: var(--bubble-tone-hover, var(--surface-hover)); }
  .fact:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  .links :global(.social:focus-visible) { outline-offset: -2px; }
  .links :global(svg) { flex: none; }
  .links :global(.social > svg) { width: 16px; height: 16px; }
  .links :global(.handle) { flex: 1; min-width: 0; }
  .value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .copy { display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; font-size: var(--fs-xs); color: var(--text-3); }
  .server { display: flex; flex-direction: column; align-items: stretch; gap: 4px; padding-top: var(--sp-2); border-top: 1px solid var(--border); font-size: var(--fs-xs); color: var(--text-2); line-height: var(--lh-snug); overflow-wrap: anywhere; }
  .offer { display: inline-flex; align-items: center; gap: 5px; color: var(--text); font-weight: var(--fw-semibold); }
  .server .btn { align-self: flex-start; }
  .status { display: flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-2); }
  .status.closed { color: var(--warn-text); }
  /* Its buttons the card's width, one under the other, the main one first;
     «В контактах» under them in the middle. */
  .row { display: grid; grid-template-columns: 1fr; gap: var(--sp-2); }
  .row .btn { width: 100%; min-width: 0; min-height: var(--touch, var(--control-h)); gap: var(--sp-2); justify-content: center; white-space: nowrap; }
  .known { display: inline-flex; flex: none; align-items: center; justify-content: center; gap: 4px; min-height: 32px; font-size: var(--fs-sm); color: var(--success-text, var(--success)); white-space: nowrap; }
  .damaged { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); font-style: italic; }
  @media (pointer: coarse) {
    .fact, .links :global(.social) { min-height: var(--touch); }
    .more { padding: 4px 0; }
  }
</style>

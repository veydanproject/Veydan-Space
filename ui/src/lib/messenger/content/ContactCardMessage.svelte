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
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import BioView from '../contacts/BioView.svelte';
  import SocialLinks from '../contacts/SocialLinks.svelte';
  import { messengerStore } from '../store.svelte';
  import { dmErrorCode, messengerError, profileErrorCode, type MessengerMessage } from '../api';
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
  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

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

  function copyPhone() {
    if (!card?.phone) return;
    linkActions.copy(card.phone).then(() => {
      copied = true;
      if (copyTimer) clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copied = false), 1500);
    }).catch(() => {});
  }

  function openSite() {
    if (website) linkActions.openExternal(website as ExternalUrl).catch(() => {});
  }
</script>

{#if !card}
  <span class="damaged"><Icon name="alert-triangle" size={12} />{$t('msg_card_damaged')}</span>
{:else}
  <div class="contact-card">
    <span class="kicker"><Icon name="user" size={11} />{$t('msg_card_contact_title')}</span>
    <div class="top">
      <Avatar url={null} src={card.avatar} {label} seed={card.pubkey} size={48} />
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

    {#if card.socials.length}<SocialLinks socials={card.socials} compact />{/if}

    {#if website || card.phone}
      <div class="facts">
        {#if website}
          <button type="button" class="fact" onclick={openSite} title={website}>
            <Icon name="globe" size={13} /><span class="value">{websiteText}</span><Icon name="external-link" size={11} />
          </button>
        {/if}
        {#if card.phone}
          <button type="button" class="fact" onclick={copyPhone}
            title={ownCard ? $t('msg_card_phone_hint') : $t('msg_card_copy')} aria-label="{$t('msg_card_phone')}: {card.phone}. {$t('msg_card_copy')}">
            <Icon name="phone" size={13} /><span class="value">{card.phone}</span>
            <span class="copy" class:done={copied}><Icon name={copied ? 'check' : 'copy'} size={12} />{copied ? $t('msg_card_copied') : ''}</span>
          </button>
        {/if}
      </div>
    {/if}

    {#if card.is_me}
      <div class="status"><Icon name="user" size={12} />{$t('msg_card_this_is_you')}</div>
    {:else if card.blocked}
      <div class="status closed"><Icon name="ban" size={12} />{$t('msg_card_blocked')}</div>
    {:else}
      {#if error}<div class="error">{error}</div>{/if}
      <div class="row">
        {#if known}
          <button class="btn btn-primary btn-sm" disabled={busy} onclick={write}>
            <Icon name="message-circle" size={13} />{$t('msg_card_write')}
          </button>
          <span class="known"><Icon name="check" size={12} />{added ? $t('msg_card_added') : $t('msg_card_already_contact')}</span>
        {:else}
          <button class="btn btn-primary btn-sm" disabled={busy} onclick={accept}>
            <Icon name="user-plus" size={13} />{$t('msg_card_add_contact')}
          </button>
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={write}>
            <Icon name="message-circle" size={13} />{$t('msg_card_write')}
          </button>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .contact-card {
    display: flex; flex-direction: column; gap: var(--sp-2);
    width: 320px; max-width: 100%;
    padding: var(--sp-3); border-radius: var(--radius-md); border: 1px solid var(--border);
    background: var(--surface); color: var(--text);
  }
  .kicker {
    display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); font-weight: var(--fw-semibold);
    color: var(--text-3); text-transform: uppercase; letter-spacing: 0.5px;
  }
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub code { font-family: var(--font-mono); }
  .bio-box { min-width: 0; }
  .more {
    align-self: flex-start; border: none; background: none; padding: 0; margin-top: -4px; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2);
  }
  .more:hover { text-decoration: underline; }
  .facts { display: flex; flex-direction: column; gap: 2px; }
  .fact {
    display: flex; align-items: center; gap: 7px; width: 100%; min-width: 0; text-align: left;
    border: none; background: none; color: var(--text-2); font: inherit; font-size: var(--fs-xs);
    padding: 4px 6px; margin-inline: -6px; border-radius: var(--radius-sm); cursor: pointer;
  }
  .fact:hover { background: var(--surface-row-hover, var(--surface-2)); color: var(--text); }
  .fact:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .fact :global(svg) { flex-shrink: 0; }
  .value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .copy { display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; font-size: var(--fs-2xs); color: var(--text-3); }
  .copy.done { color: var(--success-text, var(--success)); }
  .status { display: flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-2); }
  .status.closed { color: var(--warn-text); }
  .row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .known { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--text-3); padding: 0 4px; }
  .error { font-size: var(--fs-xs); color: var(--danger-text); line-height: 1.4; }
  .damaged { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); font-style: italic; }
  @media (pointer: coarse) {
    .fact { padding: 8px 6px; font-size: var(--fs-sm); }
    .more { padding: 4px 0; }
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My profile as others see it (avatar, name, bio, links, website), my
  phone as only I see it, "Send my card", and the editor.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { profileLabel } from '../api';
  import { linkActions } from '../content/actions';
  import type { ExternalUrl } from '../content/types';
  import ShareDialog from '../content/ShareDialog.svelte';
  import Avatar from './Avatar.svelte';
  import BioView from './BioView.svelte';
  import SocialLinks from './SocialLinks.svelte';
  import ProfileEditor from './ProfileEditor.svelte';
  import { profileDraft } from './profileDraft';

  interface Props {
    /** Shows the media servers' settings (from the avatar's "no server" hint). */
    onopenmedia?: () => void;
  }
  let { onopenmedia }: Props = $props();

  // An edit left unpublished (another chat, the media settings) opens again as it was.
  let editing = $state(profileDraft.has(messengerStore.identity?.pubkey));
  let sharing = $state(false);
  let saved = $state(false);
  let savedTimer: ReturnType<typeof setTimeout> | null = null;
  let copied = $state(false);

  const p = $derived(messengerStore.ownProfile);
  const own = $derived(messengerStore.ownPrivate);
  const canAct = $derived(!!messengerStore.status?.runtime?.session_active);
  const label = $derived(profileLabel(p));
  /** The short name under the display name, when both are set and differ. */
  const handle = $derived(p?.name && p.display_name && p.name.trim() !== p.display_name.trim() ? p.name.trim() : null);
  const website = $derived(p?.website && /^https?:\/\/\S+$/i.test(p.website) ? p.website : null);

  $effect(() => {
    if (!messengerStore.ownPrivate && canAct) messengerStore.loadOwnPrivate().catch(() => {});
  });

  function closed(ok: boolean) {
    editing = false;
    if (!ok) return;
    saved = true;
    if (savedTimer) clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (saved = false), 2500);
  }

  function openSite() {
    if (website) linkActions.openExternal(website as ExternalUrl).catch(() => {});
  }

  function copyPhone() {
    const v = own?.phone;
    if (!v) return;
    linkActions.copy(v).then(() => {
      copied = true;
      setTimeout(() => (copied = false), 1500);
    }).catch(() => {});
  }

  const shortSite = (u: string) => u.replace(/^https?:\/\//i, '').replace(/\/$/, '');
</script>

<div class="card own">
  <div class="title-row">
    <div class="card-title"><Icon name="user" size={16} /> {$t('msg_own_title')}</div>
    {#if !editing}
      <button class="btn btn-ghost btn-sm" disabled={!canAct} onclick={() => { editing = true; saved = false; }}>
        <Icon name="pencil" size={12} />{$t('msg_own_edit')}
      </button>
    {/if}
  </div>

  {#if !editing}
    <div class="head">
      <Avatar url={p?.picture ?? null} label={label || (messengerStore.identity?.npub ?? '?')} seed={messengerStore.identity?.pubkey} size={72} />
      <div class="who">
        <div class="name" class:unnamed={!label}>{label || $t('msg_own_unnamed')}</div>
        {#if handle}<div class="meta">@{handle}</div>{/if}
        {#if p?.nip05}
          <div class="meta nip05" class:ok={p.nip05_verified}><Icon name={p.nip05_verified ? 'check-circle' : 'globe'} size={12} />{p.nip05}</div>
        {/if}
      </div>
    </div>

    {#if p?.bio?.length}<BioView spans={p.bio} />{/if}
    {#if p?.socials?.length}<SocialLinks socials={p.socials} />{/if}

    {#if website || p?.lud16 || own?.phone}
      <ul class="facts">
        {#if website}
          <li>
            <span class="fact-icon"><Icon name="globe" size={14} /></span>
            <button type="button" class="link" onclick={openSite} title={website}>{shortSite(website)}</button>
          </li>
        {/if}
        {#if p?.lud16}
          <li>
            <span class="fact-icon"><Icon name="zap" size={14} /></span>
            <span class="value">{p.lud16}</span>
          </li>
        {/if}
        {#if own?.phone}
          <li class="phone">
            <span class="fact-icon"><Icon name="phone" size={14} /></span>
            <div class="phone-text">
              <button type="button" class="link plain" onclick={copyPhone} title={$t('msg_copy')}>{copied ? $t('msg_card_copied') : own.phone}</button>
              <span class="private-note"><Icon name="lock" size={11} />{$t('msg_profile_phone_view_hint')}</span>
            </div>
          </li>
        {/if}
      </ul>
    {/if}

    {#if saved}<div class="saved" role="status"><Icon name="check" size={13} />{$t('msg_profile_saved')}</div>{/if}

    <div class="send">
      <button class="btn btn-primary btn-sm" disabled={!canAct} onclick={() => (sharing = true)}>
        <Icon name="send" size={13} />{$t('msg_profile_send_card')}
      </button>
      <p class="hint">{$t('msg_profile_send_card_hint')}</p>
    </div>
  {:else}
    <ProfileEditor onclose={closed} {onopenmedia} />
  {/if}
</div>

<ShareDialog bind:open={sharing} card={{ pubkey: null }} />

<style>
  .own { min-width: 0; max-width: 100%; display: flex; flex-direction: column; gap: var(--sp-3); }
  .title-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .btn :global(svg) { margin-right: 2px; }
  .head { display: flex; gap: var(--sp-3); align-items: center; min-width: 0; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); overflow-wrap: anywhere; word-break: break-word; }
  .name.unnamed { color: var(--text-3); font-weight: var(--fw-semibold); }
  .meta { color: var(--text-3); font-size: var(--fs-xs); overflow-wrap: anywhere; word-break: break-word; }
  .nip05 { display: inline-flex; align-items: center; gap: 4px; }
  .nip05.ok { color: var(--success-text); }
  .facts { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .facts li { display: flex; align-items: flex-start; gap: 10px; min-width: 0; font-size: var(--fs-sm); }
  .fact-icon {
    width: 28px; height: 28px; flex-shrink: 0; border-radius: var(--radius-sm); background: var(--surface-3); color: var(--text-2);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .value { padding-top: 5px; overflow-wrap: anywhere; word-break: break-word; color: var(--text-body); }
  .link {
    padding: 5px 0 0; border: none; background: none; cursor: pointer; font: inherit; text-align: left;
    color: var(--accent-text-3); text-decoration: underline; text-underline-offset: 2px; overflow-wrap: anywhere; word-break: break-word; min-width: 0;
  }
  .link.plain { color: var(--text); text-decoration: none; font-variant-numeric: tabular-nums; }
  .link:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .phone-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .private-note { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--success-text); }
  .saved { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-sm); color: var(--success-text); }
  .send { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; padding-top: var(--sp-3); border-top: 1px solid var(--border); }
  .hint { margin: 0; flex: 1; min-width: 200px; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  @media (pointer: coarse) {
    .fact-icon { width: 32px; height: 32px; }
    .link { min-height: 40px; padding-top: 6px; }
    .send .btn { width: 100%; }
  }
</style>

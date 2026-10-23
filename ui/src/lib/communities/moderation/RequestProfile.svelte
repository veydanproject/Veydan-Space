<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/RequestProfile.svelte at 8ce09c39, then changed for channels. -->

<!--
  The profile of one who asks to be let into a channel, over the requests:
  what they say of themselves, their key and their words to the moderators. Only what
  this device already holds; nothing is asked of the network for it.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { Avatar, BioView, CopyField, SocialLinks } from '$lib/messenger/seams/shared';
  import { nameStore } from '../names.svelte';
  import type { RequestView } from '../api';
  import type { RequestFacts } from './requests';

  interface Props {
    open: boolean;
    request: RequestView | null;
    facts: RequestFacts | null;
  }
  let { open = $bindable(), request, facts }: Props = $props();

  const profile = $derived(request ? nameStore.profile(request.requester) : null);
  const own = $derived(profile?.display_name?.trim() || profile?.name?.trim() || '');
</script>

<Dialog bind:open title={$t('chan_req_profile_title')} onclose={() => (open = false)}>
  {#if request && facts}
    <div class="body">
      <section class="top">
        <Avatar url={facts.picture} label={facts.name} seed={request.requester} size={64} />
        <div class="name">{facts.name}</div>
        {#if own && own !== facts.name}<div class="sub">{own}</div>{/if}
        {#if facts.nip05}
          <div class="sub nip05" class:ok={facts.nip05Verified}>
            <Icon name={facts.nip05Verified ? 'rosette-discount-check' : 'world'} size={12} />{facts.nip05}
          </div>
        {/if}
        {#if facts.missing}<div class="sub">{$t('chan_req_no_profile')}</div>{/if}
      </section>

      {#if profile?.bio?.length}
        <section class="field"><span class="label">{$t('chan_req_bio')}</span><BioView spans={profile.bio} /></section>
      {:else if profile?.about}
        <section class="field"><span class="label">{$t('chan_req_bio')}</span><p class="about">{profile.about}</p></section>
      {/if}

      {#if profile?.socials?.length}
        <section class="field"><span class="label">{$t('chan_req_links')}</span><SocialLinks socials={profile.socials} /></section>
      {/if}

      <section class="field">
        <span class="label">{$t('chan_req_note_label')}</span>
        {#if request.note}<blockquote class="note">{request.note}</blockquote>{:else}<p class="none">{$t('chan_req_no_note')}</p>{/if}
      </section>

      <CopyField label={$t('chan_req_key')} value={request.requester} mono />
    </div>
  {/if}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-4); min-width: 0; }
  .top { display: flex; flex-direction: column; align-items: center; gap: var(--sp-1); text-align: center; }
  .name { margin-top: var(--sp-1); font-size: var(--fs-md); font-weight: var(--fw-extrabold); overflow-wrap: anywhere; }
  .sub { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; }
  .nip05.ok { color: var(--success-text, var(--success)); }
  .field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .label { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .about { margin: 0; font-size: var(--fs-sm); color: var(--text-body); white-space: pre-wrap; overflow-wrap: anywhere; }
  .note {
    margin: 0; padding: var(--sp-2) var(--sp-3); border-left: 3px solid var(--accent-border); border-radius: var(--radius-xs);
    background: var(--surface-2); font-size: var(--fs-sm); line-height: 1.45; color: var(--text-body); white-space: pre-wrap; overflow-wrap: anywhere;
  }
  .none { margin: 0; font-size: var(--fs-sm); color: var(--text-3); font-style: italic; }
</style>

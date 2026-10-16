<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One request of a stranger: who wrote (what their profile says of them,
  never their picture, and that they are not a contact) and the one text
  they could send, as plain words with nothing to press. No composer: the
  only answers are the three buttons. Accepting makes it an ordinary chat,
  which the page opens.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import CopyField from '../identity/CopyField.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import PanelHead from '../shared/PanelHead.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { reportStore } from '../shared/report.svelte';
  import { stamp } from '../shared/time';
  import { chatStore } from '../chats/chatStore.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { dmErrorCode, messengerError, type RequestAction, type RequestView } from '../api';
  import { requestName, requestStore } from './requestStore.svelte';

  interface Props {
    request: RequestView;
    onback?: () => void;
    /** Accepted: the chat to open in place of the request. */
    onaccepted: (chatId: string) => void;
    /** Declined or blocked: the request is gone. */
    ondone: () => void;
  }
  let { request, onback, onaccepted, ondone }: Props = $props();

  let error = $state('');

  const name = $derived(requestName(request));
  /** The key as people pass it on; the row of the chat has it, the request only the hex. */
  const key = $derived(chatStore.chats.find((c) => c.id === request.chat_id)?.peer_npub ?? request.peer);
  // A profile of a stranger is read like anyone's; its picture is not (Avatar gets no address).
  const nip05 = $derived(nameStore.profile(request.peer)?.nip05?.trim() || null);
  const busy = $derived(requestStore.acting === request.peer);

  // Another request shown here: the words of the last one's failure go, and
  // this one counts as read.
  $effect(() => {
    const chatId = request.chat_id;
    untrack(() => {
      error = '';
      chatStore.markRead(chatId).catch(() => {});
    });
  });

  /** The request is the only thing known of the person; a report that blocked them takes the request away like Block does. */
  function report() {
    reportStore.open({ accused: request.peer, name, context: { t: 'request' }, canBlock: true, onsent: (r) => { if (r.blocked) ondone(); } });
  }

  async function act(action: RequestAction) {
    // The request leaves the list while the answer goes: what is needed after it is kept now.
    const { peer, chat_id: chatId } = request;
    const who = name;
    if (action === 'block' && !(await confirmStore.ask($t('msg_rel_confirm_block', { name: who }), $t('msg_rel_cta_block'), true))) return;
    error = '';
    try {
      await requestStore.act(peer, action);
    } catch (e) {
      const code = dmErrorCode(e);
      error = code ? $t(`msg_err_${code}` as 'msg_err_dm_blocked', { name: who }) : messengerError(e);
      return;
    }
    if (action === 'accept') onaccepted(chatId);
    else ondone();
  }
</script>

<div class="request">
  <PanelHead title={$t('msg_req_title')} {onback} />

  <div class="scroll">
    <section class="who">
      <Avatar url={null} label={name} seed={request.peer} size={64} />
      <div class="name">{name}</div>
      {#if nip05}
        <div class="sub"><Icon name="globe" size={12} /><span class="nip05">{nip05}</span> · {$t('msg_req_nip05_unverified')}</div>
      {/if}
      <span class="tag"><Icon name="user" size={12} />{$t('msg_req_not_contact')}</span>
    </section>

    <CopyField label={$t('msg_peer_key')} value={key} mono />

    <section class="message">
      <div class="bubble"><MessageContent text={request.text} plain /></div>
      <span class="time">{stamp(request.at)}</span>
    </section>

    {#if request.held > 0}<p class="note">{$t('msg_req_held', { n: String(request.held) })}</p>{/if}
    <p class="note">{$t('msg_req_hint', { name })}</p>
  </div>

  <div class="bottom">
    {#if error}<p class="error-line">{error}</p>{/if}
    <div class="actions">
      <button class="btn btn-primary" disabled={busy} onclick={() => act('accept')}><Icon name="check" size={14} />{$t('msg_rel_cta_accept')}</button>
      <button class="btn btn-ghost" disabled={busy} onclick={() => act('decline')}>{$t('msg_rel_cta_decline')}</button>
      <button class="btn btn-ghost danger" disabled={busy} onclick={() => act('block')}><Icon name="ban" size={14} />{$t('msg_rel_cta_block')}</button>
      <button class="btn btn-ghost danger" disabled={busy} onclick={report}><Icon name="alert-triangle" size={14} />{$t('msg_report')}</button>
    </div>
  </div>
</div>

<style>
  .request { flex: 1; min-height: 0; height: 100%; display: flex; flex-direction: column; background: var(--surface); }
  .scroll {
    flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-5) var(--sp-4);
    display: flex; flex-direction: column; gap: var(--sp-4); width: 100%; max-width: 560px; margin-inline: auto;
  }
  .who { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-extrabold); letter-spacing: -0.2px; overflow-wrap: anywhere; }
  .sub { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; }
  .nip05 { color: var(--text-2); }
  .tag {
    display: inline-flex; align-items: center; gap: 5px; padding: 2px 10px; border-radius: var(--radius-pill);
    background: var(--warn-bg); color: var(--warn-text); border: 1px solid var(--warn-border); font-size: var(--fs-xs); font-weight: var(--fw-semibold);
  }
  .message { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; }
  .bubble {
    max-width: 100%; padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md);
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-body);
  }
  /* The text is read and maybe copied (a name, an address): selectable on touch too. */
  .request .bubble :global(.text) { user-select: text; -webkit-user-select: text; }
  .time { font-size: var(--fs-2xs); color: var(--text-3); padding-left: var(--sp-1); }
  .note { margin: 0; font-size: var(--fs-xs); line-height: 1.5; color: var(--text-3); }
  .bottom {
    flex-shrink: 0; border-top: 1px solid var(--border); background: var(--surface);
    padding: var(--sp-3) var(--sp-4) calc(var(--sp-3) + var(--sab, 0px));
  }
  .error-line { margin: 0 auto var(--sp-2); max-width: 560px; font-size: var(--fs-sm); color: var(--danger-text); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); max-width: 560px; margin-inline: auto; }
  .actions .btn { flex: 1 1 auto; justify-content: center; }
  .danger { color: var(--danger-text); }
</style>

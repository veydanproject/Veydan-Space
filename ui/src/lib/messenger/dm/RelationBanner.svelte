<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What the relationship with the peer means right now and what can be done
  about it. One row per screen mode; `full_chat` shows nothing.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { onPhone } from '../shared/phone';
  import type { ChatMode, DmAction, MessengerChat } from '../api';

  interface Props {
    chat: MessengerChat;
    busy: boolean;
    onaction: (a: DmAction) => void;
  }
  let { chat, busy, onaction }: Props = $props();

  type Tone = 'info' | 'ask' | 'warn';
  interface Cta { action: DmAction; label: string; kind: 'primary' | 'ghost' | 'danger' }
  interface Spec { tone: Tone; icon: string; ctas: Cta[] }

  const specs: Record<Exclude<ChatMode, 'full_chat' | 'group'>, Spec> = {
    first_contact: { tone: 'info', icon: 'info', ctas: [] },
    mutual_reconnect: { tone: 'info', icon: 'info', ctas: [] },
    both_removed: { tone: 'info', icon: 'info', ctas: [] },
    request_revoked_by_peer: { tone: 'info', icon: 'info', ctas: [] },
    request_sent: { tone: 'info', icon: 'clock', ctas: [{ action: 'remove', label: 'msg_rel_cta_withdraw', kind: 'ghost' }] },
    request_received: {
      tone: 'ask', icon: 'user-plus',
      ctas: [
        { action: 'accept', label: 'msg_rel_cta_accept', kind: 'primary' },
        { action: 'decline', label: 'msg_rel_cta_decline', kind: 'ghost' },
        { action: 'block', label: 'msg_rel_cta_block', kind: 'danger' },
      ],
    },
    request_declined: { tone: 'warn', icon: 'alert-triangle', ctas: [{ action: 'remove', label: 'msg_rel_cta_remove', kind: 'ghost' }] },
    request_declined_by_me: {
      tone: 'info', icon: 'info',
      ctas: [
        { action: 'request', label: 'msg_rel_cta_add', kind: 'primary' },
        { action: 'block', label: 'msg_rel_cta_block', kind: 'danger' },
      ],
    },
    removed_by_peer: {
      tone: 'warn', icon: 'alert-triangle',
      ctas: [
        { action: 'request', label: 'msg_rel_cta_request', kind: 'primary' },
        { action: 'remove', label: 'msg_rel_cta_remove', kind: 'ghost' },
      ],
    },
    blocked: { tone: 'warn', icon: 'user-cancel', ctas: [{ action: 'unblock', label: 'msg_rel_cta_unblock', kind: 'primary' }] },
    blocked_by_peer: { tone: 'warn', icon: 'user-cancel', ctas: [] },
  };

  // A key the network banned cannot be unblocked, so the banner offers nothing.
  const spec = $derived.by((): Spec | null => {
    if (chat.mode === 'full_chat' || chat.mode === 'group') return null;
    if (chat.mode === 'blocked' && chat.banned) return { ...specs.blocked, ctas: [] };
    return specs[chat.mode];
  });
  const phone = onPhone();
  // A computer: the main action last, the plate reads to it. A phone (design 15): the main one first and filled, the
  // others after it, the red ones last, in the order of the request's screen (RequestView).
  const ordered = $derived.by((): Cta[] => {
    if (!spec) return [];
    const of = (kind: Cta['kind']) => spec.ctas.filter((c) => c.kind === kind);
    return phone ? [...of('primary'), ...of('ghost'), ...of('danger')] : [...spec.ctas.filter((c) => c.kind !== 'primary'), ...of('primary')];
  });
</script>

{#snippet acts()}
  {#each ordered as c (c.action)}
    <button class="btn btn-sm {c.kind === 'danger' ? 'btn-danger-soft' : phone && c.kind === 'primary' ? 'btn-primary' : 'btn-ghost'}"
      disabled={busy} onclick={() => onaction(c.action)}>
      {$t(c.label as 'msg_rel_cta_accept')}
    </button>
  {/each}
{/snippet}

{#if spec}
  <!-- A plate of the page (core/ui/Notice): on a computer the buttons secondary, the main one last; on a phone the main
       one first and filled; red only for the block. -->
  <div class="rel" class:m={phone}>
    <Notice tone={spec.tone === 'warn' ? 'warn' : 'info'} icon={spec.icon} role="status" action={ordered.length ? acts : undefined}>
      {chat.mode === 'blocked' && chat.banned ? $t('msg_rel_banned') : $t(chat.mode === 'request_declined' ? 'msg_err_dm_request_declined' : `msg_rel_${chat.mode}` as 'msg_rel_blocked', { name: chat.title })}
    </Notice>
  </div>
{/if}

<style>
  .rel { padding: var(--sp-2) var(--sp-4) 0; flex-shrink: 0; }
  /* A phone: the buttons 40 high (their zone 44, mobile.css), the main one too, in the words of the others. */
  .rel.m .btn { min-height: 40px; font-size: var(--fs-sm); }
</style>

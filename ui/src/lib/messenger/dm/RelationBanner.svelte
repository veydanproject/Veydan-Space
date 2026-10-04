<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What the relationship with the peer means right now and what can be done
  about it. One row per screen mode; `full_chat` shows nothing.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
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
    blocked: { tone: 'warn', icon: 'ban', ctas: [{ action: 'unblock', label: 'msg_rel_cta_unblock', kind: 'primary' }] },
    blocked_by_peer: { tone: 'warn', icon: 'ban', ctas: [] },
  };

  const spec = $derived(chat.mode === 'full_chat' || chat.mode === 'group' ? null : specs[chat.mode]);
</script>

{#if spec}
  <div class="banner {spec.tone}" role="status">
    <span class="ico"><Icon name={spec.icon} size={16} /></span>
    <span class="text">{$t(`msg_rel_${chat.mode}` as 'msg_rel_blocked', { name: chat.title })}</span>
    {#if spec.ctas.length}
      <span class="ctas">
        {#each spec.ctas as c (c.action)}
          <button class="btn btn-sm {c.kind === 'primary' ? 'btn-primary' : 'btn-ghost'}" class:danger={c.kind === 'danger'}
            disabled={busy} onclick={() => onaction(c.action)}>
            {$t(c.label as 'msg_rel_cta_accept')}
          </button>
        {/each}
      </span>
    {/if}
  </div>
{/if}

<style>
  .banner {
    display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap;
    padding: var(--sp-2) var(--sp-4); border-bottom: 1px solid var(--border);
    background: var(--surface-2); font-size: var(--fs-xs); color: var(--text-body); line-height: 1.45;
  }
  .banner.ask { background: var(--accent-tint); border-bottom-color: var(--accent-tint-border); }
  .banner.warn { background: var(--warn-bg); border-bottom-color: var(--warn-border); }
  .ico { display: inline-flex; color: var(--text-2); flex-shrink: 0; }
  .ask .ico { color: var(--accent-text-2); }
  .warn .ico { color: var(--warn-text); }
  .text { flex: 1; min-width: 200px; }
  .ctas { display: inline-flex; gap: var(--sp-2); flex-wrap: wrap; }
  .btn.danger { color: var(--danger-text); }
</style>

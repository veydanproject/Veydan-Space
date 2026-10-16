<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The details of a row of the journal: who (or which group) it is, «Аудио»,
  «Видео» (a group: «Позвонить») and «Написать», then every call with them
  that the journal holds, newest first, and «Удалить из истории» for them
  all. The pane of the section on a computer, a sheet on a phone.
-->
<script lang="ts">
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { onPhone } from '../shared/phone';
  import { shortStamp } from '../shared/time';
  import type { CallLogEntry, CallMedia } from '../api';
  import CallIcon, { type CallGlyph } from './CallIcon.svelte';
  import { callLog } from './callLog.svelte';
  import { dialable, dialBlock, dialer, targetOf } from './dialer.svelte';
  import { isLogMissed, lineOf } from './log';
  import { callLineWords, duration, groupLineWords } from './words';

  interface Props {
    chatId: string;
    /** «Написать». */
    onchat: (chatId: string) => void;
    /** A call starts from here (a phone closes the sheet under the question «Позвонить?»). */
    oncall?: () => void;
    /** The calls went out of the journal: nothing is left to show. */
    ongone?: () => void;
  }
  let { chatId, onchat, oncall, ongone }: Props = $props();

  const phone = onPhone();
  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);
  const howLong = (s: number) => duration(s, localeTag($locale));

  const calls = $derived(callLog.entries.filter((e) => e.chat_id === chatId));
  const head = $derived(calls[0] ?? null);
  const chat = $derived(chatStore.chats.find((c) => c.id === chatId) ?? null);
  const group = $derived(head?.kind === 'group' || chatId.startsWith('group:'));
  const name = $derived(chat?.title || (group ? $t('msg_gcall_line') : head ? nameStore.label(head.peer) : ''));
  const picture = $derived(chat?.picture ?? (head && !group ? nameStore.picture(head.peer) : null));
  const can = $derived(dialable(chat));
  const block = $derived(can ? dialBlock() : '');

  function call(media: CallMedia) {
    if (!chat || !can) return;
    // Whom to call is taken first: the sheet closes, and its chat with it.
    const to = targetOf(chat);
    oncall?.();
    dialer.call(to, media, phone);
  }

  function glyph(e: CallLogEntry): CallGlyph {
    if (e.kind === 'group') return e.media === 'video' ? 'video' : 'phone';
    if (isLogMissed(e)) return 'missed';
    return e.direction === 'in' ? 'incoming' : 'outgoing';
  }

  function words(e: CallLogEntry): { title: string; detail: string | null } {
    if (e.kind !== 'group') return callLineWords(lineOf(e), tr, false, howLong);
    const starter = e.direction === 'out' || !e.started_by ? null : nameStore.label(e.started_by);
    const people = (n: number) => $t(countKey('msg_gcall_people', n, $locale), { n: String(n) });
    return groupLineWords(lineOf(e), tr, { live: false, starter, people, howLong });
  }

  function forget() {
    const ids = calls.map((c) => c.call_id);
    callLog.hide(ids).then(() => ongone?.()).catch(() => toast.error($t('msg_action_failed'), { onAction: forget }));
  }
</script>

{#if head}
  <div class="details" class:phone>
    <div class="who">
      <Avatar url={picture} label={name} seed={group ? chatId : head.peer} size={72} />
      <h2 class="name">{name}</h2>
      {#if !can}<span class="note">{$t(chat ? 'msg_calls_not_callable' : 'msg_calls_chat_gone')}</span>{/if}
    </div>
    <div class="actions">
      {#if group}
        <button class="act call" disabled={!can || !!block} title={block || undefined} onclick={() => call('audio')}>
          <span class="disc"><CallIcon name="phone" size={18} /></span>{$t('msg_calls_call')}
        </button>
      {:else}
        <button class="act call" disabled={!can || !!block} title={block || undefined} onclick={() => call('audio')}>
          <span class="disc"><CallIcon name="phone" size={18} /></span>{$t('msg_calls_audio_short')}
        </button>
        <button class="act call" disabled={!can || !!block} title={block || undefined} onclick={() => call('video')}>
          <span class="disc"><CallIcon name="video" size={18} /></span>{$t('msg_calls_video_short')}
        </button>
      {/if}
      <button class="act" disabled={!chat} onclick={() => onchat(chatId)}>
        <span class="disc"><Icon name="message-circle" size={18} /></span>{$t('msg_calls_write')}
      </button>
    </div>

    <h3 class="sub">{$t('msg_calls_history')}</h3>
    <ul class="calls">
      {#each calls as e (e.call_id)}
        {@const w = words(e)}
        <li class="call-row" class:missed={isLogMissed(e)}>
          <span class="glyph"><CallIcon name={glyph(e)} size={14} /></span>
          <span class="text">
            <span class="title">{w.title}</span>
            {#if w.detail}<span class="detail">{w.detail}</span>{/if}
          </span>
          <span class="when">{shortStamp(e.started_at)}</span>
        </li>
      {/each}
    </ul>
    <button class="forget" onclick={forget}><Icon name="trash-2" size={13} />{$t('msg_calls_forget')}</button>
  </div>
{/if}

<style>
  .details { display: flex; flex-direction: column; gap: var(--sp-4); padding: var(--sp-6) var(--sp-5); max-width: 520px; width: 100%; margin: 0 auto; }
  .details.phone { padding: 0 0 var(--sp-3); }
  .who { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; }
  .name { margin: var(--sp-2) 0 0; font-size: var(--fs-xl); font-weight: var(--fw-bold); overflow-wrap: anywhere; }
  .note { font-size: var(--fs-xs); color: var(--text-3); }
  .actions { display: flex; justify-content: center; gap: var(--sp-3); flex-wrap: wrap; }
  .act {
    display: flex; flex-direction: column; align-items: center; gap: 6px; min-width: 84px; padding: var(--sp-2) var(--sp-3);
    border: none; border-radius: var(--radius-lg); background: none; color: var(--text-2); font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); cursor: pointer;
  }
  .disc { width: 48px; height: 48px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; background: var(--surface-3); color: var(--text); }
  .act.call .disc { background: var(--success-bg); color: var(--success-text); }
  @media (hover: hover) { .act:hover:not(:disabled) { background: var(--surface-hover); color: var(--text); } }
  .act:disabled { opacity: 0.45; cursor: default; }
  .sub { margin: var(--sp-2) 0 0; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); }
  .calls { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .call-row { display: flex; align-items: center; gap: var(--sp-3); min-height: 48px; padding: var(--sp-2) var(--sp-3); border-radius: var(--radius); background: var(--surface-2); }
  .glyph { display: inline-flex; color: var(--text-3); flex-shrink: 0; }
  .text { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .title { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .detail { font-size: var(--fs-xs); color: var(--text-2); font-variant-numeric: tabular-nums; }
  .missed .glyph, .missed .title { color: var(--danger-text); }
  .when { font-size: var(--fs-xs); color: var(--text-3); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .forget {
    align-self: center; display: inline-flex; align-items: center; gap: 6px; padding: var(--sp-2) var(--sp-3);
    border: none; border-radius: var(--radius); background: none; color: var(--text-3); font: inherit; font-size: var(--fs-xs); cursor: pointer;
  }
  .forget:hover { color: var(--danger-text); background: var(--danger-bg); }
</style>

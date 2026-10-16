<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The journal of calls (the section «Звонки»), as a phone's journal: «Все» or
  «Пропущенные», under the headings of their days; the calls of one chat in
  a row of a day and of one kind are one row with their count. A missed
  call is red. The handset at the end of a row calls back the same way
  (a group: joins its call, or starts one); a press on the row opens the
  details (`onselect`), a right click or a long press its menu: call,
  video call, write, out of the journal. «Очистить историю» at the end.
-->
<script lang="ts">
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { longpress } from '../shared/longpress';
  import { onPhone } from '../shared/phone';
  import { clock as timeOfDay, dayKey, dayLabel } from '../shared/time';
  import type { CallLogEntry, CallMedia } from '../api';
  import CallIcon, { type CallGlyph } from './CallIcon.svelte';
  import { callLog } from './callLog.svelte';
  import { dialable, dialer, targetOf, type DialTarget } from './dial.svelte';
  import { foldLog, isLogMissed, lineOf, logSections, type LogRow } from './log';
  import { callLineWords, duration, groupLineWords } from './words';

  interface Props {
    /** The row whose details are open (its `key`). */
    selected?: string | null;
    onselect: (row: LogRow) => void;
    /** «Написать»: the chat of the row. */
    onchat: (chatId: string) => void;
    /** «Позвонить» of an empty journal: a new call. */
    onnew?: () => void;
  }
  let { selected = null, onselect, onchat, onnew }: Props = $props();

  const phone = onPhone();
  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);
  const howLong = (s: number) => duration(s, localeTag($locale));

  let filter = $state<'all' | 'missed'>('all');
  const entries = $derived(filter === 'missed' ? callLog.entries.filter(isLogMissed) : callLog.entries);
  const sections = $derived(logSections(foldLog(entries, dayKey), dayKey));

  const chatOf = (e: CallLogEntry) => chatStore.chats.find((c) => c.id === e.chat_id) ?? null;

  /** Who the row is with: the chat's title and picture, else what the names know of the key. */
  function who(e: CallLogEntry): { name: string; picture: string | null; seed: string } {
    const chat = chatOf(e);
    if (e.kind === 'group') return { name: chat?.title || $t('msg_gcall_line'), picture: chat?.picture ?? null, seed: e.chat_id };
    return { name: chat?.title || nameStore.label(e.peer), picture: chat?.picture ?? nameStore.picture(e.peer), seed: e.peer };
  }

  function glyph(e: CallLogEntry): CallGlyph {
    if (e.kind === 'group') return e.media === 'video' ? 'video' : 'phone';
    if (isLogMissed(e)) return 'missed';
    return e.direction === 'in' ? 'incoming' : 'outgoing';
  }

  function words(e: CallLogEntry): string {
    const line = lineOf(e);
    if (e.kind === 'group') {
      const starter = e.direction === 'out' || !e.started_by ? null : nameStore.label(e.started_by);
      const people = (n: number) => $t(countKey('msg_gcall_people', n, $locale), { n: String(n) });
      const w = groupLineWords(line, tr, { live: false, starter, people, howLong });
      return w.detail ? `${w.title} · ${w.detail}` : w.title;
    }
    const w = callLineWords(line, tr, false, howLong);
    return w.detail ? `${w.title} · ${w.detail}` : w.title;
  }

  function heading(at: number): string {
    const l = dayLabel(at);
    return l.key ? $t(`msg_day_${l.key}` as 'msg_day_today') : l.text;
  }

  /** Whom a row calls back, when it can be called. */
  function target(e: CallLogEntry): DialTarget | null {
    const chat = chatOf(e);
    return dialable(chat) ? targetOf(chat) : null;
  }

  function back(e: CallLogEntry, media: CallMedia = e.media) {
    const to = target(e);
    if (to) dialer.call(to, media, phone);
  }

  let menu = $state<{ open: boolean; x: number; y: number; row: LogRow | null }>({ open: false, x: 0, y: 0, row: null });

  function forget(row: LogRow) {
    callLog.hide(row.calls.map((c) => c.call_id)).catch(() => toast.error($t('msg_action_failed'), { onAction: () => forget(row) }));
  }

  const items = $derived.by((): MenuEntry[] => {
    const row = menu.row;
    if (!row) return [];
    const e = row.head;
    const to = target(e);
    const calls: MenuEntry[] = !to ? []
      : e.kind === 'group'
        ? [{ label: $t('msg_calls_call'), icon: 'phone', onselect: () => back(e, 'audio') }]
        : [
            { label: $t('msg_calls_audio'), icon: 'phone', onselect: () => back(e, 'audio') },
            { label: $t('msg_calls_video'), icon: 'video', onselect: () => back(e, 'video') },
          ];
    return [
      ...calls,
      ...(chatOf(e) ? [{ label: $t('msg_calls_write'), icon: 'message-circle', onselect: () => onchat(e.chat_id) } as MenuEntry] : []),
      { type: 'separator' },
      { label: $t('msg_calls_forget'), icon: 'trash-2', danger: true, onselect: () => forget(row) },
    ];
  });

  async function clearAll() {
    if (!(await confirmStore.ask($t('msg_calls_clear_confirm'), $t('msg_calls_clear'), true))) return;
    callLog.clear().catch(() => toast.error($t('msg_action_failed'), { onAction: () => void clearAll() }));
  }

  /** The next page comes as the end of the list nears. */
  function nearEnd(e: Event) {
    const el = e.currentTarget as HTMLElement;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 200) callLog.more().catch(() => {});
  }
</script>

{#snippet row(r: LogRow)}
  {@const e = r.head}
  {@const p = who(e)}
  {@const to = target(e)}
  <li>
    <div class="row" class:active={selected === r.key} class:missed={r.missed}>
      <button class="main" onclick={() => onselect(r)} oncontextmenu={(ev) => { ev.preventDefault(); menu = { open: true, x: ev.clientX, y: ev.clientY, row: r }; }}
        use:longpress={{ onpress: (pt) => (menu = { open: true, x: pt.x, y: pt.y, row: r }) }}>
        <Avatar url={p.picture} label={p.name} seed={p.seed} size={42} />
        <span class="body">
          <span class="top">
            {#if e.kind === 'group'}<span class="dim"><Icon name="users" size={12} /></span>{/if}
            <span class="name">{p.name}</span>
            {#if r.calls.length > 1}<span class="count">({r.calls.length})</span>{/if}
            <span class="time">{timeOfDay(e.started_at)}</span>
          </span>
          <span class="bottom">
            <span class="glyph"><CallIcon name={glyph(e)} size={13} /></span>
            <span class="what">{words(e)}</span>
          </span>
        </span>
      </button>
      <button class="dial" disabled={!to} onclick={() => back(e)} title={$t('msg_call_back')} aria-label={$t('msg_call_back')}>
        <CallIcon name={e.media === 'video' && e.kind === 'dm' ? 'video' : 'phone'} size={18} />
      </button>
    </div>
  </li>
{/snippet}

<div class="filter">
  <div class="tab-bar" role="tablist">
    <button role="tab" class="tab" class:active={filter === 'all'} aria-selected={filter === 'all'} onclick={() => (filter = 'all')}>{$t('msg_calls_all')}</button>
    <button role="tab" class="tab" class:active={filter === 'missed'} aria-selected={filter === 'missed'} onclick={() => (filter = 'missed')}>{$t('msg_calls_missed')}</button>
  </div>
</div>

<div class="list-wrap" onscroll={nearEnd}>
  {#if !callLog.loaded}
    <Skeleton height={64} size={42} trailing />
  {:else if callLog.failed && !callLog.entries.length}
    <div class="empty">
      <span class="empty-icon"><Icon name="alert-triangle" size={26} /></span>
      <p>{$t('msg_calls_load_failed')}</p>
      {#if callLog.reason}<span class="reason">{callLog.reason}</span>{/if}
      <button class="btn btn-ghost btn-sm" onclick={() => callLog.load()}><Icon name="refresh-cw" size={13} />{$t('msg_calls_retry')}</button>
    </div>
  {:else if sections.length === 0}
    <div class="empty">
      <span class="empty-icon"><CallIcon name={filter === 'missed' ? 'missed' : 'phone'} size={26} /></span>
      <p>{filter === 'missed' ? $t('msg_calls_empty_missed') : $t('msg_calls_empty')}</p>
      {#if filter === 'all'}
        <span>{$t('msg_calls_empty_hint')}</span>
        {#if onnew}<button class="btn btn-primary btn-sm" onclick={onnew}><CallIcon name="phone" size={14} />{$t('msg_calls_call')}</button>{/if}
      {/if}
    </div>
  {:else}
    {#each sections as s (s.day)}
      <h3 class="day">{heading(s.at)}</h3>
      <ul class="list">
        {#each s.rows as r (r.key)}{@render row(r)}{/each}
      </ul>
    {/each}
    {#if callLog.complete && filter === 'all'}
      <button class="clear" onclick={clearAll}><Icon name="trash-2" size={13} />{$t('msg_calls_clear')}</button>
    {/if}
  {/if}
</div>

<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />

<style>
  .filter { padding: 0 var(--sp-3) var(--sp-2); flex-shrink: 0; }
  .filter .tab-bar { display: flex; }
  .filter .tab { flex: 1; justify-content: center; }
  .list-wrap { flex: 1; min-height: 0; overflow-y: auto; padding: 0 var(--sp-2) var(--sp-2); }
  .day { margin: var(--sp-3) var(--sp-3) var(--sp-1); font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); text-transform: none; }
  .day:first-child { margin-top: var(--sp-1); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; border-radius: var(--radius-lg); transition: background var(--dur-fast) var(--ease); }
  .row:hover { background: var(--surface-hover); }
  .row.active { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-border); }
  .main {
    flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-3); min-height: 64px; padding: var(--sp-2) 0 var(--sp-2) var(--sp-3);
    border: none; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer;
  }
  .body { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .top, .bottom { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .name { font-weight: var(--fw-semibold); font-size: var(--fs-body); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .missed .name, .missed .count { color: var(--danger-text); }
  .count { font-size: var(--fs-sm); color: var(--text-2); flex-shrink: 0; }
  .dim { color: var(--text-3); display: inline-flex; flex-shrink: 0; }
  .time { margin-left: auto; font-size: var(--fs-xs); color: var(--text-2); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .glyph { display: inline-flex; color: var(--text-3); flex-shrink: 0; }
  .missed .glyph { color: var(--danger-text); }
  .what { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .dial {
    flex-shrink: 0; width: 40px; height: 40px; margin: 0 var(--sp-2); border: none; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center; background: none; color: var(--success-text); cursor: pointer;
  }
  @media (hover: hover) { .dial:hover:not(:disabled) { background: var(--success-bg); } }
  .dial:disabled { color: var(--text-3); opacity: 0.5; cursor: default; }
  .empty { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; padding: var(--sp-8) var(--sp-4); color: var(--text-3); }
  .empty p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .empty > span:not(.empty-icon) { font-size: var(--fs-xs); line-height: 1.5; max-width: 260px; }
  .reason { font-family: var(--font-mono); font-size: var(--fs-2xs) !important; color: var(--text-3); overflow-wrap: anywhere; }
  .empty-icon { width: 56px; height: 56px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; background: var(--accent-tint); color: var(--accent-text-2); margin-bottom: var(--sp-1); }
  .empty .btn { margin-top: var(--sp-2); }
  .clear {
    display: flex; align-items: center; justify-content: center; gap: 6px; width: 100%; margin-top: var(--sp-3);
    padding: var(--sp-2) var(--sp-3); border: none; background: none; cursor: pointer; color: var(--text-3); font: inherit; font-size: var(--fs-xs);
  }
  .clear:hover { color: var(--danger-text); }
  @media (pointer: coarse) { .dial { width: 44px; height: 44px; } }
</style>

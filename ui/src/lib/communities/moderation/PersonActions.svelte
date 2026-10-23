<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A moderator's acts on a person that need a choice or a word more: a mute
  for a span, a ban that hides what they wrote, the hiding of everything
  they wrote in a span, spam, and a change of role that leaves a moderator
  without the role (their decisions are offered to be signed again). The
  same from the team in «О канале» and from a post or a comment: the page
  calls the functions of this component (`bind:this`), it asks and acts.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import { confirmStore } from '$lib/messenger/seams/shared';
  import ChoiceDialog from './ChoiceDialog.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { nameStore } from '../names.svelte';
  import { channelsApi, type ChannelDetail } from '../api';
  import { channelError } from '../errors';
  import { losesModeration, staffOf } from '../permissions';
  import { HIDE_SPANS, MUTE_SPANS, hideFrom, muteUntil, type HideSpan, type MuteSpan } from './schedule';

  interface Props {
    detail: ChannelDetail;
    /** A refusal of an act asked without a dialog (spam, the offer to sign again). */
    onerror?: (text: string) => void;
  }
  let { detail, onerror }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const now = () => Math.floor(Date.now() / 1000);

  let mute = $state<{ open: boolean; who: string; span: string }>({ open: false, who: '', span: 'hour' });
  let ban = $state<{ open: boolean; who: string; hide: boolean }>({ open: false, who: '', hide: true });
  let hiding = $state<{ open: boolean; who: string; span: string }>({ open: false, who: '', span: 'hour' });
  let busy = $state(false);
  let error = $state('');

  const muteOptions = $derived(MUTE_SPANS.map((v) => ({ value: v, label: $t(`chan_mod_mute_span_${v}` as 'chan_mod_mute_span_hour') })));
  const hideOptions = $derived(HIDE_SPANS.map((v) => ({ value: v, label: $t(`chan_mod_hide_span_${v}` as 'chan_mod_hide_span_hour') })));
  const nameOf = (who: string) => (who ? nameStore.label(who) : '');

  // ── A moderator who stops being one ────────────────────────────────────

  const MARKS = new Set(['hide', 'approve']);

  /** How many of the marks of the log are theirs; `null` when the log cannot be read. */
  async function decisionsOf(who: string): Promise<number | null> {
    try {
      let n = 0;
      let before: number | undefined;
      for (let i = 0; i < 10; i++) {
        const p = await channelsApi.moderation.log(cid, before, 100);
        n += p.items.filter((e) => e.by === who && MARKS.has(e.action)).length;
        if (!p.next_before) break;
        before = p.next_before;
      }
      return n;
    } catch {
      return null;
    }
  }

  async function offerInherit(who: string, n: number | null) {
    if (n === 0) return;
    const nm = nameOf(who);
    const text = n === null ? tr('chan_mod_inherit_confirm_plain', { name: nm }) : tr('chan_mod_inherit_confirm', { name: nm, n: String(n) });
    if (!(await confirmStore.ask(text, tr('chan_mod_inherit_cta'), false))) return;
    try { await channelsApi.moderation.inherit(cid, who); }
    catch (e) { onerror?.(channelError(e, tr)); }
  }

  /**
   * A change of `who`'s role to `after` (`null`: out of the channel) by `op`:
   * a moderator who loses the role is offered to have their decisions
   * signed again, so they keep their force.
   */
  export async function demoting(who: string, after: string | null, op: () => Promise<boolean>) {
    const losing = losesModeration(staffOf(detail, who).role, after);
    const n = losing ? await decisionsOf(who) : 0;
    if ((await op()) && losing) await offerInherit(who, n);
  }

  // ── The acts ───────────────────────────────────────────────────────────

  export function askMute(who: string) {
    error = '';
    mute = { open: true, who, span: 'hour' };
  }

  export function askBan(who: string) {
    error = '';
    ban = { open: true, who, hide: true };
  }

  export function askHide(who: string) {
    error = '';
    hiding = { open: true, who, span: 'hour' };
  }

  /** Spam: a ban and everything they wrote hidden, after a yes. `target`: the message it was seen on. */
  export async function spam(who: string, target?: string) {
    if (!(await confirmStore.ask(tr('chan_mod_spam_confirm', { names: nameOf(who) }), tr('chan_mod_spam'), true))) return;
    await demoting(who, null, async () => {
      try {
        await channelsApi.moderation.mod(cid, target ? { t: 'spam', author: who, target } : { t: 'spam', author: who });
        await channelStore.refresh(cid);
        return true;
      } catch (e) {
        onerror?.(channelError(e, tr));
        return false;
      }
    });
  }

  async function step(fn: () => Promise<unknown>): Promise<boolean> {
    busy = true; error = '';
    try { await fn(); return true; }
    catch (e) { error = channelError(e, tr); return false; }
    finally { busy = false; }
  }

  async function confirmMute() {
    const until = muteUntil(mute.span as MuteSpan, now());
    const who = mute.who;
    if (await step(() => channelsApi.moderation.mod(cid, until === undefined ? { t: 'mute', who } : { t: 'mute', who, until }).then(() => channelStore.refresh(cid)))) {
      mute.open = false;
    }
  }

  /** A ban, and with it, unless unticked, the hiding of everything they wrote until now. */
  async function confirmBan() {
    const { who, hide } = ban;
    await demoting(who, null, async () => {
      const ok = await step(async () => {
        await channelStore.act(cid, { op: 'ban', who });
        if (hide) await channelsApi.moderation.mod(cid, { t: 'hide_author', author: who, from: 0, to: now() });
      });
      if (ok) ban.open = false;
      return ok;
    });
  }

  async function confirmHide() {
    const t0 = now();
    const { who, span } = hiding;
    if (await step(() => channelsApi.moderation.mod(cid, { t: 'hide_author', author: who, from: hideFrom(span as HideSpan, t0), to: t0 }))) hiding.open = false;
  }
</script>

<ChoiceDialog bind:open={mute.open} bind:value={mute.span} title={$t('chan_mod_mute_title', { name: nameOf(mute.who) })}
  options={muteOptions} cta={$t('chan_mod_mute_cta')} danger {busy} {error} onconfirm={confirmMute} />
<ChoiceDialog bind:open={ban.open} bind:checked={ban.hide} title={$t('chan_mod_ban_title', { name: nameOf(ban.who) })}
  text={$t('chan_mod_ban_text')} check={$t('chan_mod_ban_hide')} cta={$t('chan_member_ban')} danger {busy} {error} onconfirm={confirmBan} />
<ChoiceDialog bind:open={hiding.open} bind:value={hiding.span} title={$t('chan_mod_hide_author_title', { name: nameOf(hiding.who) })}
  text={$t('chan_mod_hide_author_text')} options={hideOptions} cta={$t('chan_menu_hide')} danger {busy} {error} onconfirm={confirmHide} />

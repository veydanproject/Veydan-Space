<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A reader's own levels in the card of a channel (communities-wire §14.1,
  spec §5): the chips of the levels they have, in plain words which
  sections are open to them and which are closed, and the way to ask for
  a level that gives more. A request sent is said until the channel
  changes.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { channelStore } from '../channelStore.svelte';
  import type { ChannelDetail } from '../api';
  import { BASE_LEVEL, EVERYONE, levelsOf } from './sections';
  import { requestable } from './request';
  import { orderedLevels } from './editor';
  import LevelRequestDialog from './LevelRequestDialog.svelte';
  import Box from '../info/Box.svelte';

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

  const cid = $derived(detail.channel.id);
  const lv = $derived(levelsOf(detail));
  const mine = $derived(orderedLevels(lv.levels).filter((l) => l.id === BASE_LEVEL || lv.my_levels.includes(l.id)));
  const titleOf = (s: { scope_id: string; title: string }) => (s.scope_id === EVERYONE && !s.title ? $t('chan_scope_everyone') : s.title);
  const open = $derived(lv.scopes.filter((s) => s.mine || s.scope_id === EVERYONE).map((s) => `«${titleOf(s)}»`));
  const closed = $derived(lv.scopes.filter((s) => !s.mine && s.scope_id !== EVERYONE).map((s) => `«${titleOf(s)}»`));
  const options = $derived(requestable(lv.levels, lv.my_levels, lv.scopes));
  const asked = $derived((channelStore.levelAsked[cid] ?? []).map((id) => lv.levels.find((l) => l.id === id)?.name).filter(Boolean) as string[]);

  let asking = $state(false);
</script>

<section class="block" aria-label={$t('chan_my_levels')}>
  <div class="label">{$t('chan_my_levels')}</div>
  <Box kind="text" start>
    <div class="chips">
      {#each mine as l (l.id)}<span class="lvl" class:base={l.id === BASE_LEVEL}><Icon name={l.id === BASE_LEVEL ? 'users' : 'layers'} size={14} />{l.name}</span>{/each}
    </div>
    <p class="hint">
      {$t('chan_levels_explain')}
      {#if open.length}{$t('chan_levels_open', { list: open.join(', ') })}{/if}
      {#if closed.length}{$t('chan_levels_closed', { list: closed.join(', ') })}{/if}
    </p>
    {#if asked.length}
      <p class="sent" role="status"><Icon name="clock" size={14} />{$t('chan_level_request_sent_text', { name: asked.join('», «') })}</p>
    {/if}
    {#if options.length}
      <button class="btn btn-ghost btn-sm" onclick={() => (asking = true)}><Icon name="layers" size={18} />{$t('chan_level_request')}</button>
    {/if}
  </Box>
</section>

<LevelRequestDialog bind:open={asking} {detail} />

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-3); }
  .label { display: flex; align-items: center; min-height: 24px; font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .lvl {
    display: inline-flex; align-items: center; gap: var(--sp-1); height: 28px; padding: 0 var(--sp-3); border-radius: var(--radius-pill);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text); background: var(--accent-bg); border: 1px solid var(--accent-border);
  }
  .lvl.base { color: var(--text-2); background: var(--surface-3); border-color: var(--border); }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.5; }
  .sent {
    margin: 0; display: flex; align-items: flex-start; gap: var(--sp-2); padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md);
    font-size: var(--fs-xs); line-height: 1.45; color: var(--accent-text-2); background: var(--accent-bg); border: 1px solid var(--accent-border);
  }
  .sent :global(svg) { flex-shrink: 0; margin-top: 2px; }
  @media (pointer: coarse) { .btn { min-height: var(--control-h-lg); } }
</style>

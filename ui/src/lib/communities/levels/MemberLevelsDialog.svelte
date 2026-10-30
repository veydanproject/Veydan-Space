<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The levels of one person, given or taken by a moderator (communities-wire
  §14.1, `set_levels`): a box per level, the base one always ticked. The
  keeper moves them into the sections the new levels open and out of
  those nothing else gives them; only those sections get a new key.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { nameStore } from '../names.svelte';
  import type { ChannelDetail } from '../api';
  import { BASE_LEVEL, EVERYONE, levelsOf, levelsOfPerson } from './sections';
  import { orderedLevels } from './editor';

  interface Props {
    open: boolean;
    detail: ChannelDetail;
    /** Hex pubkey of the person. */
    who: string;
  }
  let { open = $bindable(), detail, who }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const lv = $derived(levelsOf(detail));
  const levels = $derived(orderedLevels(lv.levels));

  let picked = $state<number[]>([]);
  let busy = $state(false);
  let error = $state('');

  let openedFor = '';
  $effect(() => {
    if (!open) { openedFor = ''; return; }
    if (openedFor === who) return;
    openedFor = who;
    picked = levelsOfPerson(detail, who);
    error = ''; busy = false;
  });

  const titleOf = (id: string) => (id === EVERYONE ? lv.scopes.find((s) => s.scope_id === id)?.title || $t('chan_scope_everyone') : (lv.scopes.find((s) => s.scope_id === id)?.title ?? ''));

  function toggle(id: number) {
    if (id === BASE_LEVEL) return;
    picked = picked.includes(id) ? picked.filter((x) => x !== id) : [...picked, id].sort((a, b) => a - b);
  }

  async function save() {
    busy = true; error = '';
    try {
      await channelStore.setMemberLevels(cid, who, [...new Set([BASE_LEVEL, ...picked])].sort((a, b) => a - b));
      open = false;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title={$t('chan_member_levels_title', { name: nameStore.label(who) })} width="min(420px, calc(100vw - 24px))">
  <div class="body">
    <p class="hint">{$t('chan_member_levels_hint')}</p>
    <ul class="levels">
      {#each levels as l (l.id)}
        <li>
          <label class="check">
            <input type="checkbox" checked={l.id === BASE_LEVEL || picked.includes(l.id)} disabled={busy || l.id === BASE_LEVEL} onchange={() => toggle(l.id)} />
            <span class="text">
              <span class="name">{l.name}{#if l.id === BASE_LEVEL}<span class="tag">{$t('chan_level_base')}</span>{/if}</span>
              <span class="sub">{l.scopes.map(titleOf).filter(Boolean).join(', ') || $t('chan_level_no_scopes')}</span>
            </span>
          </label>
        </li>
      {/each}
    </ul>
    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('chan_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy} onclick={save}>{$t('chan_save')}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .levels { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .check { display: flex; align-items: flex-start; gap: var(--sp-2); padding: 6px 4px; border-radius: var(--radius-sm); cursor: pointer; user-select: none; }
  .check:hover { background: var(--surface-row-hover); }
  .check input { margin-top: 3px; flex-shrink: 0; }
  .text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .name { display: flex; align-items: center; gap: 6px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .tag { font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 0 var(--sp-2); border-radius: var(--radius-pill); border: 1px solid var(--border-strong); color: var(--text-2); }
  @media (pointer: coarse) { .check { padding: 10px 4px; } }
</style>

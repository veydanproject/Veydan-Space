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
  import Notice from '$lib/core/ui/Notice.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
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
    <!-- Several of a few: ToggleRow lines; the base level is always on. -->
    <div class="levels">
      {#each levels as l (l.id)}
        <ToggleRow surface="row" title={l.name} on={l.id === BASE_LEVEL || picked.includes(l.id)} disabled={busy || l.id === BASE_LEVEL} onclick={() => toggle(l.id)}>
          {#snippet hintContent()}
            {#if l.id === BASE_LEVEL}<span class="tag">{$t('chan_level_base')}</span>{/if}
            {l.scopes.map(titleOf).filter(Boolean).join(', ') || $t('chan_level_no_scopes')}
          {/snippet}
        </ToggleRow>
      {/each}
    </div>
    {#if error}<Notice tone="danger">{error}</Notice>{/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy} onclick={save}>{$t('common_save')}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hint { margin: 0; line-height: var(--lh-snug); }
  .levels { display: flex; flex-direction: column; }
  .tag { margin-right: var(--sp-1-5); font-size: var(--fs-xs); font-weight: var(--fw-bold); padding: 0 var(--sp-2); border-radius: var(--radius-pill); border: 1px solid var(--border-strong); color: var(--text-2); }
</style>

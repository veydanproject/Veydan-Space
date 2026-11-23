<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The notes a TOTP entry is linked from. Only where the notes are (Space): the
  callers check `directory.owned('note')`.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import Suggest, { type SuggestItem } from '$lib/core/ui/Suggest.svelte';

  interface NoteOpt {
    id: string;
    title: string;
  }

  interface Props {
    notes: NoteOpt[];
    selected: string[];
    onchange: (ids: string[]) => void;
    /** The phone's field and label (.m-field) instead of the dialog's (.form-group). */
    mobile?: boolean;
  }

  let { notes, selected, onchange, mobile = false }: Props = $props();

  const inputId = `totp-notes-${Math.random().toString(36).slice(2, 8)}`;

  let query = $state('');
  let open = $state(false);
  let field = $state<HTMLInputElement | null>(null);
  let list = $state<Suggest | null>(null);

  const q = $derived(query.trim().toLowerCase());
  const hits = $derived(
    q ? notes.filter((n) => !selected.includes(n.id) && n.title.toLowerCase().includes(q)) : [],
  );
  const items = $derived(hits.map((n): SuggestItem => ({ key: n.id, text: n.title, mark: 'note' })));

  function titleOf(id: string): string {
    return notes.find((n) => n.id === id)?.title ?? id;
  }

  function add(id: string) {
    onchange([...selected, id]);
    query = '';
    open = false;
  }

  function remove(id: string) {
    onchange(selected.filter((item) => item !== id));
  }
</script>

<div class={mobile ? 'm-field links' : 'form-group links'}>
  <label for={inputId}>{$t('pw_bind_note')}</label>
  {#if selected.length}
    <div class="chips">
      {#each selected as id (id)}
        <span class="chip">
          <ChipMark kind="note" />
          <span class="chip-text">{titleOf(id)}</span>
          <button type="button" class="x" onclick={() => remove(id)} aria-label={$t('pass_remove')}>×</button>
        </span>
      {/each}
    </div>
  {/if}
  <input
    id={inputId}
    type="text"
    bind:this={field}
    bind:value={query}
    placeholder={$t('pw_bind_note_search')}
    autocomplete="off"
    onfocus={() => (open = true)}
    onblur={() => (open = false)}
    oninput={() => (open = true)}
    onkeydown={(e) => { if (open) list?.keydown(e); }}
  />
  {#if open && items.length}
    <Suggest bind:this={list} {items} anchor={field} label={$t('pw_bind_note')} onpick={(item) => add(item.key)} onclose={() => (open = false)} />
  {/if}
</div>

<style>
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-1); }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    max-width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    padding: 2px 2px 2px var(--sp-2);
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  .chip-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .x {
    min-width: 24px;
    min-height: 24px;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font-size: var(--fs-md);
    line-height: var(--lh-icon);
  }
  .m-field .x { min-width: 32px; min-height: 32px; }
</style>

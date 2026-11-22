<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The note's context: Veydan entities bound to or mentioned in a note, a row each with its actions. -->
<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  // «mentioned» and the empty line are the phone's words; its t() reads them in every language.
  import { t as mt } from '$lib/core/mobile/i18n';
  import { formatError } from '$lib/core/utils';
  import { parseBinding, isEntityKind, type EntityKind } from '$lib/core/bindings';
  import { directory, foreignName } from '$lib/core/directory';
  import { entityKinds, entitySummary, type EntityAction, type EntityStatus } from '$lib/notes/context';
  import { onMount, tick } from 'svelte';

  onMount(() => {
    void directory.ensureLoaded();
  });

  const STATUS_LABEL: Record<EntityStatus, TranslationKey> = {
    ok: 'ctx_status_ok',
    bad: 'ctx_status_bad',
    unknown: 'ctx_status_unknown',
  };

  interface Props {
    /** Bindings stored on the note */
    bindings: string[];
    /** When set, passwords tagged `note:id` also appear as context cards. */
    noteId?: string | null;
    /** `[[kind:id]]` mentions found in the body */
    mentions: string[];
    /** Binding to select and scroll into view, e.g. after clicking a mention in the text */
    focus?: string | null;
    readonly?: boolean;
    onbind: (binding: string) => void;
    onunbind: (binding: string) => void;
  }

  let { bindings, mentions, focus = null, readonly = false, noteId = null, onbind, onunbind }: Props = $props();

  interface Card {
    binding: string;
    kind: EntityKind;
    id: string;
    bound: boolean;
    /** Link stored on the password tags, not on the note. */
    fromTag: boolean;
  }

  const cards = $derived.by((): Card[] => {
    const out: Card[] = [];
    const seen = new Set<string>();
    const push = (b: string, bound: boolean, fromTag = false) => {
      const p = parseBinding(b);
      if (!p || !isEntityKind(p.kind) || !p.value || seen.has(b)) return;
      seen.add(b);
      out.push({ binding: b, kind: p.kind, id: p.value, bound, fromTag });
    };
    for (const b of bindings) push(b, true);
    for (const b of mentions) push(b, false);
    if (noteId) {
      // Entities of other modules that refer to the note (passwords tagged `note:id`).
      for (const def of entityKinds()) {
        for (const id of def.referring?.(`note:${noteId}`) ?? []) push(`${def.kind}:${id}`, true, true);
      }
    }
    return out;
  });

  let listEl = $state<HTMLElement>();
  $effect(() => {
    const target = focus;
    if (!target || !listEl) return;
    void tick().then(() => {
      const row = [...(listEl?.querySelectorAll<HTMLElement>('[data-binding]') ?? [])].find((el) => el.dataset.binding === target);
      row?.scrollIntoView({ block: 'nearest' });
    });
  });

  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function unbind(card: Card, name?: string) {
    if (!(await ask({ title: $t('ctx_unbind_confirm', { name: name ?? card.id }), confirmLabel: $t('ctx_unbind') }))) return;
    const def = directory.get(card.kind);
    // The owner of an entity that refers to the note removes the reference on both sides.
    if (def?.link && noteId) {
      void def.link.remove(card.id, `note:${noteId}`);
      return;
    }
    onunbind(card.binding);
  }

  async function run(card: Card, action: EntityAction) {
    busy = `${card.binding}/${action.id}`;
    error = null;
    try {
      await action.run(card.id);
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = null;
    }
  }
</script>

<div class="context" bind:this={listEl}>
  {#each cards as card (card.binding)}
    {@const def = directory.get(card.kind)}
    {@const entity = entitySummary(card.kind, card.id)}
    {#if def && !def.fallback}
      {@const name = entity?.name ?? card.id}
      <div class="row" class:focus={focus === card.binding} class:mention={!card.bound} data-binding={card.binding}>
        <span class="mark" style:color={entity?.color || undefined}><Icon name={def.icon} size={18} /></span>
        <div class="text">
          <div class="head">
            <span class="name" title={name}>{entity?.name ?? $t('ctx_missing')}</span>
            {#if entity?.status}
              <span class="status-dot {entity.status === 'unknown' ? 'off' : entity.status}" title={$t(STATUS_LABEL[entity.status])}></span>
            {/if}
          </div>
          <span class="sub">{$t(def.label)}{#if entity?.subtitle} · {entity.subtitle}{/if}{#if !card.bound && !card.fromTag} · {$mt('notes_context_mentioned')}{/if}</span>
        </div>
        {#if !readonly && (card.bound && (def.link ? !!noteId : !card.fromTag))}
          <button type="button" class="icon-btn x" onclick={() => unbind(card, entity?.name)} title={$t('ctx_unbind')} aria-label={$t('ctx_unbind')}>
            <Icon name="x" size={18} />
          </button>
        {/if}
        {#if entity}
          {#snippet bind()}
            {#if !card.bound && !card.fromTag && !readonly}
              <button type="button" class="btn btn-sm btn-ghost bind" onclick={() => onbind(card.binding)}>
                <Icon name="plus" size={16} />
                {$t('ctx_bind')}
              </button>
            {/if}
          {/snippet}
          {#if def.inline}
            <!-- The owner shows its own entity: its actions (.foot) on the first line, tags, live codes and «more» under the name. -->
            <div class="owned"><def.inline id={card.id}>{@render bind()}</def.inline></div>
          {:else}
            <div class="acts">
              {#if def.open}
                <button type="button" class="icon-btn" title={$t('common_open')} onclick={() => def.open?.(card.id)}>
                  <Icon name="external-link" size={18} />
                </button>
              {/if}
              {#each def.actions.filter((a) => a.id !== 'open') as action (action.id)}
                <button
                  type="button"
                  class="icon-btn"
                  title={$t(action.label)}
                  disabled={busy !== null}
                  onclick={() => run(card, action)}
                >
                  <Icon name={action.icon} size={18} />
                </button>
              {/each}
            </div>
            {#if !card.bound && !card.fromTag && !readonly}<div class="more">{@render bind()}</div>{/if}
          {/if}
        {/if}
      </div>
    {:else}
      {@const foreign = directory.foreign(card.kind, card.id)}
      {#if foreign}
        <!-- No owner here: the kind's icon, the name and the color from the labels, no actions (10.3). -->
        <div class="row" class:focus={focus === card.binding} class:mention={!card.bound} data-binding={card.binding}>
          <span class="mark" style:color={foreign.color || undefined}><Icon name={foreign.icon} size={18} /></span>
          <div class="text">
            <div class="head"><span class="name" title={foreignName(foreign, $t)}>{foreignName(foreign, $t)}</span></div>
            <span class="sub">{$t(foreign.kindLabel)}{#if !card.bound && !card.fromTag} · {$mt('notes_context_mentioned')}{/if}</span>
          </div>
          {#if !readonly && card.bound && !card.fromTag}
            <button type="button" class="icon-btn x" onclick={() => unbind(card, foreignName(foreign, $t))} title={$t('ctx_unbind')} aria-label={$t('ctx_unbind')}>
              <Icon name="x" size={18} />
            </button>
          {/if}
        </div>
      {/if}
    {/if}
  {:else}
    <p class="empty-state sm">{$mt('notes_context_empty')}</p>
  {/each}
  {#if error}
    <Notice tone="danger">{error}</Notice>
  {/if}
</div>

<style>
  /* Rows in one column under the drawer's head; the drawer's body scrolls. */
  .context { display: flex; flex-direction: column; }
  /* The icon, the name over «kind · subtitle», the actions and the ×; under the name what the owner shows. */
  .row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto auto;
    column-gap: var(--sp-3);
    row-gap: var(--sp-1);
    align-items: center;
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-md);
    position: relative;
  }
  /* The hairline between rows, under the row's rounded hover. */
  .row + .row::before {
    content: '';
    position: absolute;
    top: 0; left: var(--sp-3); right: var(--sp-3);
    border-top: 1px solid var(--divider);
  }
  .row:hover { background: var(--surface-hover); }
  .row.focus { background: var(--accent-bg); }
  .row:hover::before, .row:hover + .row::before, .row.focus::before, .row.focus + .row::before { border-color: transparent; }
  .mark { grid-column: 1; grid-row: 1; display: inline-flex; color: var(--text-2); }
  .text { grid-column: 2; grid-row: 1; display: flex; flex-direction: column; min-width: 0; }
  .head { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .name {
    font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--text);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .row.mention .name { color: var(--text-2); }
  .sub { font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .acts, .owned > :global(.foot) { grid-column: 3; grid-row: 1; display: flex; align-items: center; gap: 2px; }
  .owned > :global(.foot .pw-actions) { gap: 2px; }
  .x { grid-column: 4; grid-row: 1; }
  /* The owner's parts are the row's own cells: its actions go right of the name, the rest under it. */
  .owned { display: contents; }
  .owned > :global(*:not(.foot)), .more { grid-column: 2 / -1; min-width: 0; }
  .owned > :global(.pw-tags) { flex-wrap: wrap; overflow: visible; }
  .bind { justify-self: start; }
  .context > :global(.notice) { margin-top: var(--sp-2); }
</style>

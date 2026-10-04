<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Context cards: Veydan entities bound to or mentioned in a note, with actions. -->
<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { parseBinding, isEntityKind, type EntityKind } from '$lib/core/bindings';
  import { directory, foreignName } from '$lib/core/directory';
  import { entityKinds, entitySummary, type EntityAction, type EntityStatus } from '$lib/notes/context';
  import { onMount } from 'svelte';

  function markCopied(key: string) {
    copied = '';
    requestAnimationFrame(() => (copied = key));
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = ''), 1600);
  }

  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    void directory.ensureLoaded();
    return () => clearTimeout(copyTimer);
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
    /** Binding to highlight, e.g. after clicking a mention in the text */
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

  let busy = $state<string | null>(null);
  let copied = $state('');
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
    const key = `${card.binding}/${action.id}`;
    busy = key;
    error = null;
    try {
      await action.run(card.id);
      if (action.id.startsWith('copy')) markCopied(key);
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = null;
    }
  }
</script>

{#if cards.length > 0}
  <div class="context">
    {#each cards as card (card.binding)}
      {@const def = directory.get(card.kind)}
      {@const entity = entitySummary(card.kind, card.id)}
      {#if def && !def.fallback}
      <div class="card tinted" class:focus={focus === card.binding} class:mention={!card.bound} style:--chip={entity?.color ?? def.color}>
        <div class="head">
          <span class="mark"><Icon name={def.icon} size={13} /></span>
          <span class="name" title={entity?.name ?? card.id}>{entity?.name ?? $t('ctx_missing')}</span>
          {#if entity?.status}
            <span class="dot {entity.status}" title={$t(STATUS_LABEL[entity.status])}></span>
          {/if}
          {#if !readonly && (def.link ? !!noteId : card.bound && !card.fromTag)}
            <button class="x" onclick={() => unbind(card, entity?.name)} title={$t('ctx_unbind')}>×</button>
          {/if}
        </div>
        <div class="sub">
          <span class="kind">{$t(def.label)}</span>
          {#if entity?.subtitle}<span class="sep">·</span><span class="subtitle">{entity.subtitle}</span>{/if}
        </div>
        {#if entity}
          <div class="actions">
            {#snippet bind()}
              {#if !card.bound && !card.fromTag && !readonly}
                <button class="btn btn-ghost btn-xs bind" onclick={() => onbind(card.binding)}>
                  <Icon name="plus" size={11} />
                  {$t('ctx_bind')}
                </button>
              {/if}
            {/snippet}
            {#if def.inline}
              <!-- The owner shows its own entity: tags, live codes, copy buttons. -->
              <def.inline id={card.id}>{@render bind()}</def.inline>
            {:else}
              {@render bind()}
              <div class="foot">
                <div class="pw-actions">
                  {#if def.open}
                    <button type="button" class="icon-btn" title={$t('ctx_action_open')} onclick={() => def.open?.(card.id)}>
                      <Icon name="external-link" size={13} />
                    </button>
                  {/if}
                  {#each def.actions.filter((a) => a.id !== 'open') as action (action.id)}
                    {@const key = `${card.binding}/${action.id}`}
                    <button
                      type="button"
                      class="icon-btn"
                      class:success={copied === key}
                      class:pop={copied === key}
                      title={$t(action.label)}
                      disabled={busy !== null}
                      onclick={() => run(card, action)}
                    >
                      <Icon name={copied === key ? 'check' : action.icon} size={13} />
                    </button>
                  {/each}
                </div>
              </div>
            {/if}
          </div>
        {/if}
      </div>
      {:else}
        {@const foreign = directory.foreign(card.kind, card.id)}
        {#if foreign}
          <!-- No owner here: the kind's icon, the name and the color from the labels, no actions (10.3). -->
          <div class="card tinted" class:focus={focus === card.binding} class:mention={!card.bound} style:--chip={foreign.color}>
            <div class="head">
              <span class="mark"><Icon name={foreign.icon} size={13} /></span>
              <span class="name" title={foreignName(foreign, $t)}>{foreignName(foreign, $t)}</span>
              {#if !readonly && card.bound && !card.fromTag}
                <button class="x" onclick={() => unbind(card, foreignName(foreign, $t))} title={$t('ctx_unbind')}>×</button>
              {/if}
            </div>
            <div class="sub">
              <span class="kind">{$t(foreign.kindLabel)}</span>
            </div>
          </div>
        {/if}
      {/if}
    {/each}
    {#if error}
      <div class="error">{error}</div>
    {/if}
  </div>
{/if}

<style>
  /* The cards are bounded and scroll on their own: the editor below keeps its height. */
  .context {
    flex-shrink: 0;
    max-height: min(38%, 240px);
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-4);
    border-top: 1px solid var(--border);
    background: var(--bg-2);
  }
  .card {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 0.25rem;
    padding: 0.45rem 0.6rem;
    border-width: 1px;
    border-style: solid;
    border-radius: var(--radius-sm);
    min-width: 0;
  }
  .card.focus { border-color: var(--accent); }
  .card.mention { border-style: dashed; }
  .head { display: flex; align-items: center; gap: 0.35rem; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .card:not([style*='--chip']) .name { color: var(--text); }
  .mark { display: inline-flex; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
  .dot.ok { background: var(--success); }
  .dot.bad { background: var(--danger-text); }
  .dot.unknown { background: var(--text-3); }
  .x {
    background: none; border: 0; padding: 0; cursor: pointer;
    color: inherit; opacity: var(--chip-fade); font-size: var(--fs-sm); line-height: 1;
  }
  .x:hover { opacity: 1; }
  .sub { display: flex; align-items: center; gap: 0.3rem; font-size: var(--fs-2xs); min-width: 0; }
  /* The kind's name is in the card's ink, as its name and icon. */
  .kind { text-transform: uppercase; letter-spacing: 0.06em; }
  /* The × and the subtitle fade the card's ink: a fixed --text-3 does not follow the tint. */
  .sep, .subtitle { color: inherit; opacity: var(--chip-fade); }
  .subtitle { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* What the owner shows inside keeps the ordinary text, not the card's ink. */
  .actions { display: flex; flex-direction: column; align-items: flex-start; gap: 0.25rem; margin-top: 0.15rem; flex: 1; color: var(--text); }
  .foot {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    width: 100%;
    margin-top: auto;
  }
  .btn-xs { font-size: var(--fs-2xs); padding: 0.15rem 0.45rem; gap: 0.25rem; }
  .bind { color: var(--accent); }
  .spinning { opacity: 0.6; }
  .error { grid-column: 1 / -1; font-size: var(--fs-xs); color: var(--danger-text); }
  .pw-actions { display: flex; gap: 0.25rem; margin-left: auto; }
  .icon-btn.pop { animation: icon-pop 0.35s ease; }
  @keyframes icon-pop {
    0% { transform: scale(1); }
    40% { transform: scale(1.12); }
    100% { transform: scale(1); }
  }
</style>

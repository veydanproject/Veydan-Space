<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Veydan objects bound to or mentioned in a note. Tap a row to see the other
  notes about that object; long-press for bind / unbind.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError, type BindingSummary, type NoteListItem } from '$lib/notes/mobile/api';
  import { t, type MobileKey } from '$lib/core/mobile/i18n';
  import { longpress } from '$lib/core/mobile/longpress';
  import { portal } from '$lib/core/portal';
  import type { Attachment } from 'svelte/attachments';
  import { parseBinding, type EntityKind } from '$lib/core/bindings';
  import { directory } from '$lib/core/directory';
  import { entityKinds } from '$lib/notes/context';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    /** Current note; excluded from "other notes" */
    noteId: string;
    /** Entity bindings stored on the note */
    bindings: string[];
    /** `[[kind:id]]` mentions found in the body */
    mentions: string[];
    /** Binding to highlight, e.g. after tapping a mention */
    focus?: string | null;
    busy?: boolean;
    onclose: () => void;
    onopen: (noteId: string) => void;
    onbind: (binding: string) => void;
    onunbind: (binding: string) => void;
  }

  let { open, noteId, bindings, mentions, focus = null, busy = false, onclose, onopen, onbind, onunbind }: Props = $props();

  const ICON: Record<EntityKind, string> = {
    workspace: 'layers',
    profile: 'globe',
    proxy: 'shield',
    ssh: 'terminal',
    totp: 'key',
    password: 'lock',
  };
  const KIND_LABEL: Record<EntityKind, MobileKey> = {
    workspace: 'notes_context_kind_workspace',
    profile: 'notes_context_kind_profile',
    proxy: 'notes_context_kind_proxy',
    ssh: 'notes_context_kind_ssh',
    totp: 'notes_context_kind_totp',
    password: 'notes_context_kind_password',
  };
  /** "Other notes of this workspace / profile / ..." in the right grammatical form */
  const RELATED_LABEL: Record<EntityKind, MobileKey> = {
    workspace: 'notes_context_related_workspace',
    profile: 'notes_context_related_profile',
    proxy: 'notes_context_related_proxy',
    ssh: 'notes_context_related_ssh',
    totp: 'notes_context_related_totp',
    password: 'notes_context_related_password',
  };

  interface Row {
    binding: string;
    kind: EntityKind;
    id: string;
    bound: boolean;
    /** Link stored on the password tags, not on the note. */
    fromTag: boolean;
    summary: BindingSummary | null;
  }

  let summaries = $state<Map<string, BindingSummary>>(new Map());
  let error = $state('');
  /** Row whose other notes are shown */
  let expanded = $state<string | null>(null);
  let related = $state<NoteListItem[]>([]);
  let loadingRelated = $state(false);
  /** Row the long-press menu is open for */
  let menuRow = $state<Row | null>(null);

  /** Entities of other modules that refer to the note (passwords tagged `note:id`). */
  const linked = $derived(
    entityKinds().flatMap((def) => (def.referring?.(`note:${noteId}`) ?? []).map((id) => `${def.kind}:${id}`)),
  );
  const all = $derived([...new Set([...bindings, ...mentions, ...linked])]);

  const rows = $derived.by((): Row[] =>
    all.flatMap((b) => {
      const p = parseBinding(b);
      if (!p || !p.value || !(p.kind in ICON)) return [];
      return [{
        binding: b,
        kind: p.kind as EntityKind,
        id: p.value,
        bound: bindings.includes(b),
        fromTag: linked.includes(b) && !bindings.includes(b),
        summary: summaries.get(b) ?? null,
      }];
    }),
  );

  $effect(() => {
    if (!open) { menuRow = null; return; }
    void directory.ensureLoaded();
    const wanted = all;
    error = '';
    expanded = null;
    api.notes
      .bindingSummaries(wanted)
      .then((list) => (summaries = new Map(list.map((s) => [s.binding, s]))))
      .catch((e) => (error = formatError(e)));
  });

  async function toggle(row: Row) {
    if (expanded === row.binding) { expanded = null; return; }
    expanded = row.binding;
    loadingRelated = true;
    try {
      related = (await api.notes.entityNotes(row.binding)).filter((n) => n.id !== noteId);
    } catch (e) {
      error = formatError(e);
    } finally {
      loadingRelated = false;
    }
  }

  /**
   * Stable attachment: an inline `longpress(() => ...)` would be recreated on every
   * re-render (e.g. the 1s TOTP tick), which cancels the 500ms timer before it fires.
   * The row is read back from `data-binding` instead.
   */
  const pressMenu: Attachment<HTMLElement> = (el) =>
    longpress(() => openMenu(rows.find((r) => r.binding === el.dataset.binding) ?? null))(el);

  /** The release after a long press lands as a click; ignore it if it hits the freshly opened menu. */
  let menuOpenedAt = 0;

  function openMenu(row: Row | null) {
    menuRow = row;
    menuOpenedAt = Date.now();
  }

  function menuAction(action: 'bind' | 'unbind' | 'open') {
    if (Date.now() - menuOpenedAt < 400) return;
    const row = menuRow;
    menuRow = null;
    if (!row) return;
    const def = directory.get(row.kind);
    if (action === 'open') {
      onclose();
      def?.open?.(row.id);
      return;
    }
    // The owner of an entity that refers to the note removes the reference on both sides.
    if (def?.link && action === 'unbind') {
      void def.link.remove(row.id, `note:${noteId}`);
      return;
    }
    if (action === 'bind') onbind(row.binding);
    else onunbind(row.binding);
  }
</script>

<BottomSheet {open} title={$t('notes_context')} {onclose}>
  {#if error}<div class="m-error">{error}</div>{/if}
  {#if rows.length === 0}
    <p class="empty">{$t('notes_context_empty')}</p>
  {:else}
    <p class="hint">{$t('notes_context_hint')}</p>
    <div class="m-list">
      {#each rows as row (row.binding)}
        {@const isOpen = expanded === row.binding}
        {@const def = directory.get(row.kind)}
        <!-- A kind without an owner here and without a name in the labels yet: its kind and a short id (10.3). -->
        {@const foreign = row.summary ? undefined : directory.foreign(row.kind, row.id)}
        <div class="entity" class:focus={focus === row.binding} class:mention={!row.bound}>
          <!-- Row is a div so the live TOTP chip (a button itself) can sit inline -->
          <div class="m-row entity-row" data-binding={row.binding} {@attach pressMenu}>
            <button type="button" class="main" onclick={() => toggle(row)}>
              <Icon name={ICON[row.kind]} size={18} />
              <div class="text">
                <span class="m-row-label">{row.summary?.name ?? (foreign ? `${$t(KIND_LABEL[row.kind])} ${foreign.shortId}` : $t('notes_context_missing'))}</span>
                <span class="sub">
                  {$t(KIND_LABEL[row.kind])}{#if row.summary?.subtitle} · {row.summary.subtitle}{/if}{#if !row.bound} · {$t('notes_context_mentioned')}{/if}
                </span>
              </div>
            </button>
            {#if row.summary && def?.inline}
              <!-- The owner's controls for its entity: the live code, the copy buttons. -->
              <def.inline id={row.id} />
            {/if}
            <button type="button" class="chev" onclick={() => toggle(row)} aria-label={$t(RELATED_LABEL[row.kind])}>
              <Icon name={isOpen ? 'chevron-down' : 'chevron-right'} size={18} />
            </button>
            {#if !row.fromTag || def?.link}
            <button type="button" class="chev" onclick={() => openMenu(row)} aria-label={$t('notes_context_more')}>
              <Icon name="more-vertical" size={18} />
            </button>
            {/if}
          </div>
          {#if row.summary && def?.details}
            <def.details id={row.id} />
          {/if}
          {#if isOpen}
            <div class="related">
              <span class="sub related-title">{$t(RELATED_LABEL[row.kind])}</span>
              {#if loadingRelated}
                <span class="sub pad">{$t('loading')}</span>
              {:else if related.length === 0}
                <span class="sub pad">{$t('notes_context_related_empty')}</span>
              {:else}
                {#each related as n (n.id)}
                  <button type="button" class="m-row nested" onclick={() => onopen(n.id)}>
                    <Icon name="file-text" size={16} />
                    <span class="m-row-label">{n.title || $t('notes_untitled')}</span>
                  </button>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</BottomSheet>

<!-- Long-press menu: bind / unbind. Portalled: a fixed sheet inside the transformed parent sheet would be clipped. -->
<div use:portal>
<BottomSheet open={menuRow !== null} title={menuRow?.summary?.name ?? $t('notes_context')} onclose={() => (menuRow = null)}>
  <div class="m-list">
    {#if menuRow && directory.get(menuRow.kind)?.open}
      <button type="button" class="m-row" onclick={() => menuAction('open')}>
        <Icon name="external-link" size={20} /><span class="m-row-label">{$t('ctx_action_open')}</span>
      </button>
    {/if}
    {#if menuRow && (menuRow.bound || (menuRow.fromTag && directory.get(menuRow.kind)?.link))}
      <button type="button" class="m-row" disabled={busy} onclick={() => menuAction('unbind')}>
        <Icon name="x" size={20} /><span class="m-row-label danger-text">{$t('notes_context_unbind')}</span>
      </button>
      <p class="hint">{$t('notes_context_unbind_hint')}</p>
    {:else}
      <button type="button" class="m-row" disabled={busy} onclick={() => menuAction('bind')}>
        <Icon name="plus" size={20} /><span class="m-row-label">{$t('notes_context_bind')}</span>
      </button>
      <p class="hint">{$t('notes_context_bind_hint')}</p>
    {/if}
  </div>
</BottomSheet>
</div>

<style>
  .empty { margin: var(--sp-4); color: var(--text-3); text-align: center; }
  .hint { margin: 0 var(--sp-2) var(--sp-3); color: var(--text-3); font-size: var(--fs-xs); }
  .entity { display: flex; flex-direction: column; }
  .entity + .entity { border-top: 1px solid var(--border); }
  .entity.focus { background: var(--accent-bg); border-radius: 12px; }
  .entity.mention .m-row-label { color: var(--text-2); }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .sub { font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .entity-row { padding-right: 0; gap: var(--sp-2); }
  .main {
    flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-3);
    min-height: 52px; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; text-align: left;
  }
  .chev {
    display: inline-flex; align-items: center; justify-content: center;
    width: 40px; height: 44px; border: 0; background: transparent; color: var(--text-3);
  }
  .related { display: flex; flex-direction: column; padding: 0 0 var(--sp-2) var(--sp-5); }
  .related-title { padding: 0 var(--sp-2) var(--sp-1); }
  .pad { padding: 0 var(--sp-2); }
  .m-row.nested { min-height: 44px; }
</style>

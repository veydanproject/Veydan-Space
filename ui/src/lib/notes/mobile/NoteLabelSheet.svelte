<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { api, type BindingSummary, type NavChild, type NoteTag } from '$lib/notes/mobile/api';
  import { t, type MobileKey } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { labelPickerText } from '$lib/notes/label-kinds';

  /** Entity kinds looked up in the backend; workspace/profile come from nav props. */
  const ENTITY_SEARCH = ['proxy', 'ssh', 'totp', 'password'] as const;
  const ENTITY_LABEL: Record<(typeof ENTITY_SEARCH)[number], MobileKey> = {
    proxy: 'notes_context_kind_proxy',
    ssh: 'notes_context_kind_ssh',
    totp: 'notes_context_kind_totp',
    password: 'notes_context_kind_password',
  };

  interface Props {
    open: boolean;
    tags: NoteTag[];
    selectedTags: string[];
    folders: NavChild[];
    folderIds: string[];
    workspaces: NavChild[];
    profiles: NavChild[];
    bindings: string[];
    /** Notes that can be stored as `note:id` tags. Empty hides the hits. */
    notes?: { id: string; title: string }[];
    busy?: boolean;
    onclose: () => void;
    onaddTag: (name: string, color?: string) => void;
    onaddFolder: (folderId: string) => void;
    onaddBinding: (binding: string) => void;
  }

  let {
    open, tags, selectedTags, folders, folderIds, workspaces, profiles, bindings, notes = [],
    busy = false, onclose, onaddTag, onaddFolder, onaddBinding,
  }: Props = $props();

  const TAG_COLORS = [
    '#8b7bff', '#60a5fa', '#2dd4bf', '#f472b6',
    '#f5c451', '#34d399', '#f26d6d', '#f97316',
  ];

  let query = $state('');
  let color = $state(TAG_COLORS[0]);

  $effect(() => {
    if (open) {
      query = '';
      color = TAG_COLORS[0];
    }
  });

  const q = $derived(query.trim().toLowerCase());
  const tagHits = $derived(
    tags.filter((t) => !selectedTags.includes(t.name) && t.name.toLowerCase().includes(q)),
  );
  const folderHits = $derived(
    q ? folders.filter((f) => !folderIds.includes(f.id) && f.name.toLowerCase().includes(q)) : [],
  );
  const workspaceHits = $derived(
    q ? workspaces.filter((w) => !bindings.includes(`workspace:${w.id}`) && w.name.toLowerCase().includes(q)) : [],
  );
  const profileHits = $derived(
    q ? profiles.filter((p) => !bindings.includes(`profile:${p.id}`) && p.name.toLowerCase().includes(q)) : [],
  );
  // Notes are offered right away like tags; the empty query shows the first few.
  const noteHits = $derived(
    notes.filter((n) => !bindings.includes(`note:${n.id}`) && n.title.toLowerCase().includes(q)).slice(0, 6),
  );
  const canCreate = $derived(query.trim().length > 0 && !tags.some((t) => t.name === query.trim()));

  // Proxy / SSH / TOTP hits come from the backend, debounced
  let entityHits = $state<BindingSummary[]>([]);
  let entityTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const term = q;
    if (entityTimer) clearTimeout(entityTimer);
    const kinds = term ? ENTITY_SEARCH : (['totp', 'password'] as const);
    entityTimer = setTimeout(async () => {
      const lists = await Promise.all(kinds.map((kind) => api.notes.entitySearch(kind, term).catch(() => [])));
      if (term !== q) return;
      entityHits = lists.flat().filter((s) => !bindings.includes(s.binding)).slice(0, 12);
    }, term ? 200 : 0);
  });

  // The kinds the backend has entities of (Notes alone: none of them), probed once per opening
  let backendKinds = $state<string[]>([]);
  $effect(() => {
    if (!open) return;
    void Promise.all(ENTITY_SEARCH.map((kind) => api.notes.entitySearch(kind, '').then((r) => (r.length ? kind : null)).catch(() => null)))
      .then((kinds) => (backendKinds = kinds.filter((k): k is (typeof ENTITY_SEARCH)[number] => k !== null)));
  });
  /** Placeholder and hint name only what this product can bind a note to. */
  const pickerText = $derived(
    labelPickerText(
      [...(workspaces.length ? ['workspace'] : []), ...(profiles.length ? ['profile'] : []), ...backendKinds],
      (key, vars) => $t(key, vars),
    ),
  );

  const hasHits = $derived(
    tagHits.length + folderHits.length + workspaceHits.length + profileHits.length + noteHits.length + entityHits.length > 0,
  );

  function submitTag() {
    const name = query.trim().replace(/^#/, '');
    if (!name) return;
    const matched = notes.filter((n) => n.title.toLowerCase() === name.toLowerCase());
    if (matched.length === 1) {
      onaddBinding(`note:${matched[0].id}`);
      return;
    }
    onaddTag(name, canCreate ? color : undefined);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      submitTag();
    }
  }
</script>

<BottomSheet {open} title={$t('notes_label_title')} {onclose}>
  <div class="find">
    <Icon name="search" size={16} />
    <input
      type="text"
      bind:value={query}
      placeholder={pickerText.placeholder}
      onkeydown={onKeydown}
      {@attach (el: HTMLInputElement) => el.focus()}
    />
  </div>
  {#if !q && entityHits.length === 0}
    <p class="hint">{pickerText.hint}</p>
  {/if}

  {#if hasHits}
    <div class="hits">
      {#each tagHits as tag (tag.id)}
        <button type="button" class="hit" disabled={busy} onpointerdown={(e) => e.preventDefault()} onclick={() => onaddTag(tag.name)}>
          <span class="dot" style:background-color={tag.color}></span>
          <span class="name tinted-text" style:--chip={tag.color}>{tag.name}</span>
        </button>
      {/each}
      {#each folderHits as f (f.id)}
        <button type="button" class="hit" disabled={busy} onpointerdown={(e) => e.preventDefault()} onclick={() => onaddFolder(f.id)}>
          <span class="dot" style:background={f.color}></span>
          <span class="name">{f.name}</span>
          <span class="kind">{$t('notes_kind_folder')}</span>
        </button>
      {/each}
      {#each workspaceHits as w (w.id)}
        <button type="button" class="hit" disabled={busy} onpointerdown={(e) => e.preventDefault()} onclick={() => onaddBinding(`workspace:${w.id}`)}>
          <span class="dot" style:background={w.color}></span>
          <span class="name">{w.name}</span>
          <span class="kind">{$t('notes_kind_workspace')}</span>
        </button>
      {/each}
      {#each profileHits as p (p.id)}
        <button type="button" class="hit" disabled={busy} onpointerdown={(e) => e.preventDefault()} onclick={() => onaddBinding(`profile:${p.id}`)}>
          <span class="dot"></span>
          <span class="name">{p.name}</span>
          <span class="kind">{$t('notes_kind_profile')}</span>
        </button>
      {/each}
      {#each noteHits as n (n.id)}
        <button type="button" class="hit" disabled={busy} onpointerdown={(e) => e.preventDefault()} onclick={() => onaddBinding(`note:${n.id}`)}>
          <span class="dot muted"></span>
          <span class="name">{n.title}</span>
          <span class="kind">{$t('notes_kind_note')}</span>
        </button>
      {/each}
      {#each entityHits as s (s.binding)}
        <button type="button" class="hit" disabled={busy} onpointerdown={(e) => e.preventDefault()} onclick={() => onaddBinding(s.binding)}>
          <span class="dot muted"></span>
          <span class="name">{s.name}{#if s.subtitle}<span class="sub"> · {s.subtitle}</span>{/if}</span>
          <span class="kind">{$t(ENTITY_LABEL[s.kind as (typeof ENTITY_SEARCH)[number]])}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if canCreate}
    <div class="colors">
      {#each TAG_COLORS as c (c)}
        <button type="button" class="swatch" class:on={color === c} style:background={c} aria-label={c} onclick={() => (color = c)}></button>
      {/each}
      <label class="swatch custom" class:on={!TAG_COLORS.includes(color)}>
        <input type="color" bind:value={color} />
      </label>
    </div>
    <button type="button" class="m-btn-grad" disabled={busy} onclick={submitTag}>{$t('notes_tags_add')}</button>
  {/if}
</BottomSheet>

<style>
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 12px;
    border-radius: 12px;
    background: var(--m-field);
    flex-shrink: 0;
  }
  .find input {
    flex: 1;
    min-width: 0;
    width: auto;
    min-height: 44px;
    margin: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    font-size: 15px;
  }
  .find input:focus { border: 0; box-shadow: none; }
  .hint { margin: var(--sp-3) var(--sp-2) 0; color: var(--text-3); font-size: var(--fs-xs); }
  .hits {
    max-height: min(280px, 42vh);
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    touch-action: pan-y;
    display: flex;
    flex-direction: column;
  }
  .hit {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 44px;
    padding: 0 var(--sp-2);
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 15px;
    text-align: left;
  }
  .hit:disabled { opacity: 0.5; }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--accent);
    flex-shrink: 0;
  }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { color: var(--text-3); font-size: 13px; }
  .dot.muted { background: var(--text-3); }
  .kind { color: var(--text-3); font-size: 12px; }
  .colors { display: flex; gap: 8px; padding: var(--sp-2) 0; }
  .swatch {
    width: 28px;
    height: 28px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid transparent;
  }
  .swatch.on { border-color: var(--text); }
  .custom { position: relative; background: conic-gradient(red, yellow, lime, cyan, blue, magenta, red); overflow: hidden; }
  .custom input {
    position: absolute;
    inset: 0;
    opacity: 0;
    border: 0;
    padding: 0;
  }
</style>

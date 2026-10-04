<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import NotesNav from './NotesNav.svelte';
  import { api, type NoteListFilter, type NoteNav } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';

  let open = $state(false);
  let nav = $state<NoteNav | null>(null);

  const filter = $derived<NoteListFilter>({
    kind: page.url.searchParams.get('kind') || 'all',
    id: page.url.searchParams.get('id') ?? undefined,
  });

  function load() {
    api.notes.nav().then((x) => (nav = x)).catch(() => {});
  }

  function go(next: NoteListFilter) {
    const p = new URLSearchParams();
    if (next.kind !== 'all') p.set('kind', next.kind);
    if (next.id) p.set('id', next.id);
    const qs = p.toString();
    void goto(qs ? `/notes?${qs}` : '/notes');
  }

  onMount(load);
</script>

<button type="button" class="m-ibtn" onclick={() => (open = true)} aria-label={$t('notes_nav_open')}>
  <Icon name="menu" size={22} />
</button>
<NotesNav {open} {nav} {filter} onclose={() => (open = false)} onselect={go} onchange={load} />

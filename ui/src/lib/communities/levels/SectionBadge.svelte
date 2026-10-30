<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The mark of a section (communities-wire §14.7): its photo, or its icon
  drawn on its colour (the accent when it has none). «Все» is the grid on
  the accent's tint. One mark for the head, the tiles, the editor and the
  list of sections, at the size each needs.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { lookOf } from './look';
  import { scopePhotos } from './scopePhotos.svelte';
  import type { ScopeView } from './types';

  interface Props {
    /** The section; none: «Все». */
    scope?: Pick<ScopeView, 'scope_id' | 'icon' | 'color' | 'photo'> | null;
    communityId: string;
    size?: number;
    /** A picture picked in the editor, not sent yet: shown in the photo's place. */
    preview?: string | null;
    /** The icon even where a photo is: the small marks (the head, the lists), whose photo is the background of the tile. */
    noPhoto?: boolean;
  }
  let { scope = null, communityId, size = 36, preview = null, noPhoto = false }: Props = $props();

  const look = $derived(scope ? lookOf(scope) : null);
  const src = $derived(noPhoto ? null : (preview ?? (scope && look?.photo ? scopePhotos.src(communityId, scope.scope_id, look.photo) : null)));
  const glyph = $derived(Math.round(size * 0.56));
</script>

<span class="sec-mark" class:all={!scope} style:--b-size="{size}px" style:--b-fill={look?.color ?? 'var(--accent)'} style:--b-ink={look?.ink ?? '#fff'} aria-hidden="true">
  {#if src}<img {src} alt="" />{:else}<Icon name={look?.icon ?? 'layout-grid'} size={glyph} />{/if}
</span>

<style>
  .sec-mark {
    width: var(--b-size); height: var(--b-size); flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; overflow: hidden;
    border-radius: calc(var(--b-size) * 0.3); background: var(--b-fill); color: var(--b-ink);
  }
  .sec-mark.all { background: var(--accent-tint); color: var(--accent-text-2); }
  img { width: 100%; height: 100%; object-fit: cover; display: block; }
</style>

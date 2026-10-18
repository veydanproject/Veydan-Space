<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { kindIcon } from '$lib/core/directory';

  interface Props {
    kind: string;
    /**
     * The translated name of the kind ("Profile"). The icon is hidden from
     * a screen reader, so the caption is read before the chip's text instead.
     */
    caption?: string;
  }

  let { kind, caption }: Props = $props();

  /** The kind of a chip is told by its icon, never by a colour (design kit, kind-chip); the icons of the entity kinds. */
  const ICON: Record<string, string> = {
    folder: 'folder',
    workspace: 'layers',
    profile: 'user-circle',
    proxy: 'globe',
    ssh: 'terminal',
    totp: 'shield',
    password: kindIcon('password'),
    note: 'notes',
    domain: 'link',
    tag: 'tag',
  };

  const icon = $derived(ICON[kind] ?? '');
</script>

{#if icon}
  <span class="mark" aria-hidden="true"><Icon name={icon} size={12} /></span>
{/if}
{#if caption}
  <span class="caption">{caption}: </span>
{/if}

<style>
  /* It takes the chip's text: the neutral one, or the ink of the user's
     colour (kit kind-chip: the tile of a coloured chip inherits it). */
  .mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: inherit;
    flex-shrink: 0;
  }
  /* Read out, not shown. */
  .caption {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }
</style>

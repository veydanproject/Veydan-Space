<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The tags of a password or TOTP entry as a row of small chips, named and coloured through the catalog. -->
<script lang="ts">
  import { directory, foreignName } from '$lib/core/directory';
  import { parseBinding } from '$lib/core/bindings';
  import { t } from '$lib/core/i18n';

  let { tags }: { tags: string[] } = $props();

  function chipColor(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    if (!parsed) return directory.summary('note_tag', tag)?.color;
    // The owner's color, or the color of a workspace's label where Space is absent (10.3).
    if (parsed.kind === 'workspace') return directory.color('workspace', parsed.value);
    if (parsed.kind === 'profile') return 'var(--accent)';
    if (parsed.kind === 'note') return 'var(--text-2)';
    return undefined;
  }

  function chipLabel(tag: string): string {
    const parsed = parseBinding(tag);
    if (!parsed) return tag;
    // A kind without an owner in this product: the name its label keeps, else the kind's name and a short id (10.3).
    const foreign = directory.foreign(parsed.kind, parsed.value);
    if (foreign) return foreignName(foreign, $t);
    if (parsed.kind === 'profile' || parsed.kind === 'workspace' || parsed.kind === 'note') {
      return directory.summary(parsed.kind, parsed.value)?.name ?? parsed.value;
    }
    return parsed.value;
  }

  /** The name of the kind a reference points at ("Profile"), for the chip's tooltip; none for a free label. */
  function chipKind(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    const kind = parsed ? directory.kind(parsed.kind) : undefined;
    return kind ? $t(kind.label) : undefined;
  }
</script>

{#if tags.length}
  <span class="pw-tags">
    {#each tags as tag (tag)}
      {@const color = chipColor(tag)}
      <span class="pw-tag tinted" title={chipKind(tag)} style:--chip={color}>{chipLabel(tag)}</span>
    {/each}
  </span>
{/if}

<style>
  .pw-tags {
    display: flex;
    flex-wrap: nowrap;
    gap: 4px;
    width: 100%;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .pw-tags::-webkit-scrollbar { display: none; }
  .pw-tag {
    flex-shrink: 0;
    border-width: 1px;
    border-style: solid;
    border-radius: 99px;
    padding: 0 6px;
    font-size: 0.68rem;
    white-space: nowrap;
  }
</style>

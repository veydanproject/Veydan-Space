<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The files picked for a post or a comment, above the composer (the
  composer's `attachments`), as a chat's tray: a picture as a small
  square of itself, any other file with its icon and name, each with a
  way to take it out; the words typed go as their caption. Below it, what
  it tells of them (a photo that goes as a file).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import PickedNotes from './PickedNotes.svelte';
  import { pickedIcon, type Picked } from './picked.svelte';

  let { picked }: { picked: Picked } = $props();
</script>

<div class="tray">
  {#each picked.files as p (p.key)}
    {@const preview = picked.previews[p.path]}
    <div class="pick" class:picture={!!preview} title={p.name}>
      {#if preview}<img src={preview} alt={p.name} draggable="false" onerror={() => picked.failed(p.path)} />
      {:else}<Icon name={pickedIcon(p.name)} size={20} /><span class="pick-name">{p.name}</span>{/if}
      <button class="unpick" onpointerdown={(e) => e.preventDefault()} onmousedown={(e) => e.preventDefault()} onclick={() => picked.remove(p.key)}
        aria-label={$t('msg_attach_remove')} title={$t('msg_attach_remove')}><Icon name="x" size={12} /></button>
    </div>
  {/each}
</div>
<PickedNotes {picked} />

<style>
  /* As ChatWindow's tray: small cards that scroll sideways, pictures as they look. */
  .tray { display: flex; gap: var(--sp-2); overflow-x: auto; padding: 2px 0; }
  .pick {
    position: relative; flex-shrink: 0; width: 64px; height: 64px; border-radius: var(--radius-md); overflow: hidden;
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; padding: 4px;
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-2);
  }
  .pick.picture { padding: 0; }
  .pick img { display: block; width: 100%; height: 100%; object-fit: cover; user-select: none; }
  .pick-name { max-width: 100%; font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* «Take the file out»: the round scrim is its icon's (12 in a disc of 20); on a phone the button around it is 40
     (its zone 44), the disc of 24 where it stood. */
  .unpick {
    position: absolute; top: 3px; right: 3px; width: 20px; height: 20px; border: none; border-radius: var(--radius); padding: 0; cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; background: none; color: #fff;
  }
  .unpick > :global(.icon) { width: 20px; height: 20px; padding: 4px; border-radius: 50%; background: rgba(0, 0, 0, 0.55); }
  .unpick:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  @media (pointer: coarse) {
    .pick { width: 72px; height: 72px; }
    .unpick { top: -5px; right: -5px; width: 40px; height: 40px; }
    .unpick > :global(.icon) { width: 24px; height: 24px; padding: 6px; }
  }
</style>

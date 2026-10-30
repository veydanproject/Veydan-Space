<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Pick one or more local files. Shared by every composer. On a phone the button opens the sheet of what can be attached (attach/AttachSheet, the items from seams/attach). -->
<script lang="ts" module>
  /** How the files of the sheet go: `original` without compression, `asFile` as they are (the item «Файл»). */
  export interface AttachOptions {
    original?: boolean;
    asFile?: boolean;
  }
</script>

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { MAX_SEND_BYTES } from '../api';
  import AttachSheet from '../attach/AttachSheet.svelte';
  import { mediaLibrary } from '../attach/library';
  import type { AttachContext, AttachResult, AttachSurface } from '../seams/attach.svelte';

  interface Props {
    disabled?: boolean;
    onfiles: (paths: string[], opts?: AttachOptions) => void;
    /** Where the composer is (what the items of the sheet may depend on). */
    surface?: AttachSurface;
    chatId?: string;
    /** An item of the sheet that failed, as the composer says its errors. */
    onerror?: (error: unknown) => void;
  }
  let { disabled = false, onfiles, surface = 'dm', chatId = '', onerror }: Props = $props();

  const touch = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;
  let sheet = $state(false);

  /** What this composer takes (`files`; a new kind of content is added here with its handler). */
  const ctx: AttachContext = $derived({
    surface,
    chatId,
    accepts: ['files'],
    // One album holds 64 files (messenger-dm `MAX_BATCH`); the size is the send limit.
    limits: { maxBytes: MAX_SEND_BYTES, maxCount: 64 },
  });

  // The sheet takes the place of the keyboard.
  function openSheet() {
    (document.activeElement as HTMLElement | null)?.blur?.();
    sheet = true;
  }

  // A computer: the system's chooser, as it has always been.
  async function pick() {
    const paths = await mediaLibrary().pickSystem({ multiple: true, kind: 'any' });
    if (paths.length) onfiles(paths);
  }

  function result(r: AttachResult) {
    if (r.type !== 'files') return;
    const f = r as Extract<AttachResult, { type: 'files' }>;
    onfiles(f.paths, { original: f.original, asFile: f.asFile });
  }
</script>

<button class="attach" {disabled} tabindex="-1" onpointerdown={(e) => e.preventDefault()} onmousedown={(e) => e.preventDefault()} onclick={() => (touch ? openSheet() : pick())} title={$t('msg_media_attach')}>
  <Icon name="paperclip" size={touch ? 20 : 17} />
</button>

{#if touch}
  <AttachSheet bind:open={sheet} {ctx} onresult={result} {onerror} />
{/if}

<style>
  .attach {
    width: 38px; height: 38px; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer;
    background: none; color: var(--text-2); display: inline-flex; align-items: center; justify-content: center;
  }
  .attach:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  /* Inside the field of the phone composer the field sets the size. */
  @media (pointer: coarse) { .attach { width: var(--attach-w, 44px); height: var(--attach-h, 44px); } }
  .attach:disabled { opacity: 0.4; cursor: default; }
</style>

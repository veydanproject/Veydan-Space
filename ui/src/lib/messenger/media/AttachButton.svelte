<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Pick one or more local files. Shared by every composer. On a phone the button opens a sheet of what can be attached, the picture on the clipboard too. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { isTauriHost } from '../api';
  import { clipboardHasPicture } from './clipboard';

  interface Props {
    disabled?: boolean;
    onfiles: (paths: string[]) => void;
    /** The phone: attach the picture on the clipboard (the sheet offers it while there is one). */
    onclipboard?: () => void;
  }
  let { disabled = false, onfiles, onclipboard }: Props = $props();

  const touch = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;
  let sheet = $state(false);
  /** Whether the clipboard holds a picture, as the sheet opens. */
  let clip = $state(false);

  // The composer is glass (`backdrop-filter`), and a fixed layer inside such
  // an element is laid out against it, not the screen: the sheet lives in the body.
  function toBody(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  // The sheet takes the place of the keyboard.
  function openSheet() {
    (document.activeElement as HTMLElement | null)?.blur?.();
    clip = !!onclipboard && clipboardHasPicture();
    sheet = true;
  }

  const MEDIA = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic', 'heif', 'avif', 'bmp', 'mp4', 'mov', 'm4v', 'webm', 'mkv', '3gp'];
  /** What the sheet offers. A new kind of attachment is one more entry here. */
  const kinds: { id: string; icon: string; label: 'msg_attach_media' | 'msg_attach_file'; extensions?: string[] }[] = [
    { id: 'media', icon: 'image', label: 'msg_attach_media', extensions: MEDIA },
    { id: 'file', icon: 'file', label: 'msg_attach_file' },
  ];

  async function pick(extensions?: string[]) {
    sheet = false;
    if (!isTauriHost) {
      // Browser preview: there are no paths; send a sample so the UI can be seen.
      // `huge`: a file over the send limit, refused as the app refuses it.
      onfiles(['/home/dev/Pictures/sample-1.png', '/home/dev/Pictures/sample-2.png', '/home/dev/Documents/report.pdf', '/home/dev/Videos/conference-huge.mov']);
      return;
    }
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ multiple: true, directory: false, filters: extensions ? [{ name: $t('msg_attach_media'), extensions }] : undefined });
    if (!picked) return;
    onfiles(Array.isArray(picked) ? picked : [picked]);
  }
</script>

<button class="attach" {disabled} tabindex="-1" onpointerdown={(e) => e.preventDefault()} onmousedown={(e) => e.preventDefault()} onclick={() => (touch ? openSheet() : pick())} title={$t('msg_media_attach')}>
  <Icon name="paperclip" size={touch ? 20 : 17} />
</button>

{#if touch}
  <div use:toBody>
  <Dialog bind:open={sheet} title={$t('msg_attach_title')}>
    <div class="kinds">
      {#each kinds as k (k.id)}
        <button class="kind" onclick={() => pick(k.extensions)}>
          <Icon name={k.icon} size={22} />
          <span>{$t(k.label)}</span>
        </button>
      {/each}
      {#if clip}
        <button class="kind" onclick={() => { sheet = false; onclipboard?.(); }}>
          <Icon name="copy" size={22} />
          <span>{$t('msg_attach_clipboard')}</span>
        </button>
      {/if}
    </div>
  </Dialog>
  </div>
{/if}

<style>
  .attach {
    width: 38px; height: 38px; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer;
    background: none; color: var(--text-2); display: inline-flex; align-items: center; justify-content: center;
  }
  .attach:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  /* Inside the field of the phone composer the field sets the size. */
  @media (pointer: coarse) { .attach { width: var(--attach-w, 44px); height: var(--attach-h, 44px); } }
  /* The rows of a sheet (design kit, menu as a sheet): 52 high, the icon 22, the label 16. */
  .kinds { display: flex; flex-direction: column; }
  .kind {
    display: flex; align-items: center; gap: var(--sp-4); width: 100%; min-height: 52px; padding: 0 var(--sp-2); cursor: pointer;
    background: none; border: none; border-radius: var(--radius-md);
    color: var(--text); font: inherit; font-size: var(--fs-md); text-align: left;
  }
  .kind :global(.icon) { color: var(--text-soft); }
  .kind:active { background: var(--surface-hover); }
  .attach:disabled { opacity: 0.4; cursor: default; }
</style>

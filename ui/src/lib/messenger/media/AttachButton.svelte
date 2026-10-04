<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Pick one or more local files. Shared by every composer. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isTauriHost } from '../api';

  interface Props {
    disabled?: boolean;
    onfiles: (paths: string[]) => void;
  }
  let { disabled = false, onfiles }: Props = $props();

  async function pick() {
    if (!isTauriHost) {
      // Browser preview: there are no paths; send a sample so the UI can be seen.
      onfiles(['/home/dev/Pictures/sample-1.png', '/home/dev/Pictures/sample-2.png', '/home/dev/Pictures/sample-3.png']);
      return;
    }
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ multiple: true, directory: false });
    if (!picked) return;
    onfiles(Array.isArray(picked) ? picked : [picked]);
  }
</script>

<button class="attach" {disabled} tabindex="-1" onpointerdown={(e) => e.preventDefault()} onmousedown={(e) => e.preventDefault()} onclick={pick} title={$t('msg_media_attach')}>
  <Icon name="paperclip" size={17} />
</button>

<style>
  .attach {
    width: 38px; height: 38px; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer;
    background: none; color: var(--text-2); display: inline-flex; align-items: center; justify-content: center;
  }
  .attach:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  @media (pointer: coarse) { .attach { width: 44px; height: 44px; } }
  .attach:disabled { opacity: 0.4; cursor: default; }
</style>

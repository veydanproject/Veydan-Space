<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Mount once per screen. Shows what `viewer.open` was given. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isTauriHost, messengerApi } from '../api';
  import { viewer } from './viewer.svelte';

  let zoomed = $state(false);

  $effect(() => { if (viewer.item) zoomed = false; });
  // The screen is left: what it showed does not wait for its return.
  $effect(() => () => viewer.close());

  function key(e: KeyboardEvent) {
    if (viewer.item && e.key === 'Escape') { e.stopPropagation(); viewer.close(); }
  }

  async function saveAs() {
    const it = viewer.item;
    if (!it || !isTauriHost) return;
    const { save } = await import('@tauri-apps/plugin-dialog');
    const dest = await save({ defaultPath: it.name });
    if (dest) await messengerApi.media.saveAs(it.messageId, dest).catch(() => {});
  }
</script>

<svelte:window onkeydown={key} />

{#if viewer.item}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="viewer" role="dialog" aria-modal="true" aria-label={viewer.item.name} tabindex="-1" onclick={() => viewer.close()}>
    <header role="toolbar" tabindex="-1" onclick={(e) => e.stopPropagation()}>
      <span class="name">{viewer.item.name}</span>
      <button onclick={saveAs} aria-label={$t('msg_media_save_as')} title={$t('msg_media_save_as')}><Icon name="save" size={18} /></button>
      <button onclick={() => viewer.close()} aria-label={$t('msg_back')} title={$t('msg_back')}><Icon name="x" size={20} /></button>
    </header>
    <div class="stage">
      {#if viewer.item.kind === 'image'}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <img src={viewer.item.src} alt={viewer.item.name} class:zoomed
          onclick={(e) => { e.stopPropagation(); zoomed = !zoomed; }} />
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={viewer.item.src} controls autoplay playsinline onclick={(e) => e.stopPropagation()}></video>
      {/if}
    </div>
  </div>
{/if}

<style>
  .viewer { position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0); overflow: hidden; z-index: var(--z-modal, 1000); background: rgba(6, 6, 10, 0.95); display: flex; flex-direction: column; }
  header {
    display: flex; align-items: center; gap: var(--sp-2); padding: calc(var(--sp-2) + var(--sat, 0px)) var(--sp-3) var(--sp-2);
    color: #fff; flex-shrink: 0;
  }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-sm); opacity: 0.85; }
  header button { width: 44px; height: 44px; border: none; border-radius: 50%; background: none; color: #fff; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  header button:hover { background: rgba(255, 255, 255, 0.12); }
  .stage { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; overflow: auto; padding: var(--sp-2); }
  img { max-width: 100%; max-height: 100%; object-fit: contain; cursor: zoom-in; }
  img.zoomed { max-width: none; max-height: none; cursor: zoom-out; }
  video { max-width: 100%; max-height: 100%; background: #000; }
</style>

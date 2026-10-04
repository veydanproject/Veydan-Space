<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Sheet body for one media kind: actions, quick settings, playable list. -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { formatBytes } from '$lib/core/utils';
  import type { mediaT } from '$lib/notes/media/strings';
  import type { MediaItem, MediaKind, MediaPrefs } from '$lib/notes/media/types';
  import MediaPrefsBar from './MediaPrefsBar.svelte';

  interface Props {
    kind: MediaKind;
    items: MediaItem[];
    prefs: MediaPrefs;
    t: ReturnType<typeof mediaT>;
    onprefs: (prefs: MediaPrefs) => void;
    onrecord: () => void;
    onpick: () => void;
    /** Opens the per-item action menu owned by the parent. */
    onmenu: (item: MediaItem) => void;
    resolveSrc: (item: MediaItem) => Promise<string | null>;
  }

  let { kind, items, prefs, t, onprefs, onrecord, onpick, onmenu, resolveSrc }: Props = $props();

  let playing = $state<string | null>(null);
  let playSrc = $state<string | null>(null);

  async function togglePlay(item: MediaItem) {
    if (playing === item.name) {
      stopPlay();
      return;
    }
    const src = await resolveSrc(item);
    if (!src) return;
    playing = item.name;
    playSrc = src;
  }

  function stopPlay() {
    playing = null;
    playSrc = null;
  }
</script>

<div class="actions">
  <button type="button" class="act" onclick={onrecord}>
    <span class="m-doc lg"><Icon name={kind === 'audio' ? 'mic' : 'video'} size={20} /></span>
    {t(kind === 'audio' ? 'record' : 'camera')}
  </button>
  <button type="button" class="act" onclick={onpick}>
    <span class="m-doc lg"><Icon name="paperclip" size={20} /></span>
    {t('attach')}
  </button>
</div>

<MediaPrefsBar {kind} {prefs} {t} {onprefs} />

{#if items.length}
  <h3>{t('recordings', { n: String(items.length) })}</h3>
  <div class="list">
    {#each items as item (item.name)}
      <div class="row" class:remote={!item.present}>
        <button type="button" class="open" disabled={!item.present} onclick={() => togglePlay(item)} aria-label={playing === item.name ? t('stop_play') : t('play')}>
          <span class="m-doc badge"><Icon name={playing === item.name ? 'square' : 'play'} size={16} /></span>
          <span class="meta">
            <span class="name">{item.name}</span>
            <span class="size">{item.present ? formatBytes(item.size) : t('not_downloaded')}</span>
          </span>
        </button>
        <button type="button" class="x" onclick={() => onmenu(item)} aria-label={t('actions')}>
          <Icon name="more-horizontal" size={14} />
        </button>
      </div>
      {#if playing === item.name && playSrc}
        {#if kind === 'audio'}
          <audio class="player" controls autoplay src={playSrc} onended={stopPlay}></audio>
        {:else}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video class="player" controls autoplay playsinline src={playSrc} onended={stopPlay}></video>
        {/if}
      {/if}
    {/each}
  </div>
{/if}

<style>
  .actions { display: flex; gap: var(--sp-2); }
  .act {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    min-height: 88px;
    padding: var(--sp-3);
    border: 0;
    border-radius: var(--m-radius);
    background: var(--m-field);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
  }
  h3 { margin: var(--sp-2) 0 0; font-size: 15px; font-weight: 700; }
  .list { display: flex; flex-direction: column; gap: 8px; }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 4px 4px 8px;
    border-radius: var(--m-radius);
    background: var(--m-card);
    border: 1px solid var(--m-card-border);
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 0;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
  }
  .open:disabled { opacity: 0.6; }
  .meta { display: flex; flex-direction: column; min-width: 0; gap: 2px; }
  .name { font-size: 13px; font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .size { font-size: 11px; color: var(--text-3); }
  .row.remote .badge { border: 1px dashed var(--m-card-border); background: transparent; color: var(--text-3); }
  .x {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    margin-right: 4px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--m-seg);
    color: var(--text-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .player { width: 100%; border-radius: var(--m-radius); }
  video.player { max-height: 240px; background: #000; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Audio and video trigger buttons with their sheets and the recorder screen.
     Host-agnostic: files come in as MediaItem, recordings go out as CaptureFile. -->

<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { longpress } from '$lib/core/mobile/longpress';
  import { DEFAULT_STORAGE_KEY, loadPrefs, savePrefs } from '$lib/notes/media/prefs';
  import { mediaT, type MediaLocale } from '$lib/notes/media/strings';
  import type { CaptureFile, MediaItem, MediaKind, MediaPrefs } from '$lib/notes/media/types';
  import CaptureScreen from './CaptureScreen.svelte';
  import MediaPanel from './MediaPanel.svelte';

  interface Props {
    items: MediaItem[];
    disabled?: boolean;
    locale?: MediaLocale;
    storageKey?: string;
    /** A finished take; the host stores it and refreshes `items`. */
    oncapture: (file: CaptureFile) => void | Promise<void>;
    /** "Attach" pressed for this kind; the host runs its own picker. */
    onpick: (kind: MediaKind) => void | Promise<void>;
    oninsert: (item: MediaItem) => void;
    onremove: (item: MediaItem) => void;
    onsave: (item: MediaItem) => void;
    /** Playable URL for an item, or null when it cannot be played. */
    resolveSrc: (item: MediaItem) => Promise<string | null>;
  }

  let {
    items, disabled = false, locale = 'en', storageKey = DEFAULT_STORAGE_KEY,
    oncapture, onpick, oninsert, onremove, onsave, resolveSrc,
  }: Props = $props();

  let sheet = $state<MediaKind | null>(null);
  let capturing = $state<MediaKind | null>(null);
  /** Long-press opens the recorder already running; a tap only opens the sheet. */
  let autostart = $state(false);
  let menu = $state<MediaItem | null>(null);
  let prefs = $state<MediaPrefs>(loadPrefs(untrack(() => storageKey)));

  const t = $derived(mediaT(locale));
  const shown = $derived(items.filter((i) => i.kind === sheet));

  function setPrefs(next: MediaPrefs) {
    prefs = next;
    savePrefs(next, storageKey);
  }

  function record(kind: MediaKind, immediate = false) {
    if (disabled) return;
    sheet = null;
    autostart = immediate;
    capturing = kind;
  }

  async function done(file: CaptureFile) {
    capturing = null;
    await oncapture(file);
    sheet = file.kind;
  }

  function act(fn: (item: MediaItem) => void) {
    const item = menu;
    menu = null;
    if (item) fn(item);
  }
</script>

<button type="button" class="trigger" {disabled} {@attach longpress(() => record('audio', true))} onclick={() => (sheet = 'audio')} aria-label={t('audio')}>
  <Icon name="mic" size={22} />
</button>
<button type="button" class="trigger" {disabled} {@attach longpress(() => record('video', true))} onclick={() => (sheet = 'video')} aria-label={t('video')}>
  <Icon name="video" size={22} />
</button>

<BottomSheet open={sheet !== null} title={sheet ? t(sheet) : ''} onclose={() => (sheet = null)}>
  {#if sheet}
    {@const kind = sheet}
    <MediaPanel
      {kind}
      items={shown}
      {prefs}
      {t}
      onprefs={setPrefs}
      onrecord={() => record(kind)}
      onpick={() => onpick(kind)}
      onmenu={(item) => (menu = item)}
      {resolveSrc}
    />
  {/if}
</BottomSheet>

<BottomSheet open={menu !== null} title={menu?.name ?? ''} onclose={() => (menu = null)}>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => act((i) => { sheet = null; oninsert(i); })}>
      <Icon name="link" size={20} /><span class="m-row-label">{t('insert')}</span>
    </button>
    <button type="button" class="m-row" onclick={() => act(onsave)}>
      <Icon name="save" size={20} /><span class="m-row-label">{t('save')}</span>
    </button>
  </div>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => act(onremove)}>
      <Icon name="trash-2" size={20} /><span class="m-row-label danger">{t('delete')}</span>
    </button>
  </div>
</BottomSheet>

{#if capturing}
  <CaptureScreen
    kind={capturing}
    {prefs}
    {t}
    {autostart}
    ondone={done}
    oncancel={() => { sheet = capturing; capturing = null; }}
    onfacing={(facing) => setPrefs({ ...prefs, facing })}
  />
{/if}

<style>
  .trigger {
    width: 48px;
    height: 48px;
    padding: 0;
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--text-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .trigger:active { background: var(--m-seg); }
  .trigger:disabled { opacity: 0.35; }
  .danger { color: var(--danger-text); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Quick settings for one media kind; self-styled so it works outside the mobile shell. -->

<script lang="ts">
  import type { mediaT } from '$lib/notes/media/strings';
  import { AUDIO_KBPS, VIDEO_HEIGHTS, type MediaKind, type MediaPrefs } from '$lib/notes/media/types';

  interface Props {
    kind: MediaKind;
    prefs: MediaPrefs;
    t: ReturnType<typeof mediaT>;
    onprefs: (prefs: MediaPrefs) => void;
  }

  let { kind, prefs, t, onprefs }: Props = $props();
</script>

{#if kind === 'audio'}
  <div class="pref">
    <span class="label">{t('quality')}</span>
    <div class="presets">
      {#each AUDIO_KBPS as k (k)}
        <button type="button" class="chip" class:active={prefs.audioKbps === k} onclick={() => onprefs({ ...prefs, audioKbps: k })}>{k} kbps</button>
      {/each}
    </div>
  </div>
{:else}
  <div class="pref">
    <span class="label">{t('resolution')}</span>
    <div class="presets">
      {#each VIDEO_HEIGHTS as h (h)}
        <button type="button" class="chip" class:active={prefs.videoHeight === h} onclick={() => onprefs({ ...prefs, videoHeight: h })}>{h}p</button>
      {/each}
    </div>
  </div>
  <div class="pref">
    <span class="label">{t('camera')}</span>
    <div class="presets">
      <button type="button" class="chip" class:active={prefs.facing === 'environment'} onclick={() => onprefs({ ...prefs, facing: 'environment' })}>{t('camera_back')}</button>
      <button type="button" class="chip" class:active={prefs.facing === 'user'} onclick={() => onprefs({ ...prefs, facing: 'user' })}>{t('camera_front')}</button>
    </div>
  </div>
{/if}

<style>
  .pref { display: flex; flex-direction: column; gap: var(--sp-1-5); }
  .label { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); }
  .presets { display: flex; gap: 8px; flex-wrap: wrap; }
</style>

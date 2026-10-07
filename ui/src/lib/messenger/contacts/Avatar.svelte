<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { avatarStore, safeDataImage } from './avatars.svelte';

  interface Props {
    /** A picture's address (kind 0 `picture`): never loaded here; the runtime fetches it and gives a `data:` URL. */
    url: string | null;
    /** A picture the runtime already made (a card's): a `data:image/…` URL, used as it is. */
    src?: string | null;
    label: string;
    size?: number;
    /** Stable value (public key) that picks the colour; the label otherwise. */
    seed?: string | null;
    /** A dot at the bottom right: the person is online now. */
    online?: boolean;
  }
  let { url, src = null, label, size = 36, seed = null, online = false }: Props = $props();
  let failed = $state<string | null>(null);

  // Two letters: first letters of the first two words, or the first two
  // letters of a single word. Keys (npub1…) get a neutral glyph.
  const initials = $derived.by(() => {
    const s = label.trim();
    if (!s) return '?';
    if (/^npub1/i.test(s) || /^[0-9a-f]{16,}/i.test(s)) return '#';
    const words = s.split(/\s+/).filter(Boolean);
    const pick = words.length > 1 ? [...words[0]][0] + [...words[1]][0] : [...words[0]].slice(0, 2).join('');
    return pick.toUpperCase();
  });

  // Only a `data:` picture the runtime made; initials until there is one.
  const picture = $derived(safeDataImage(src) ?? avatarStore.get(url));
  const shown = $derived(picture && picture !== failed ? picture : null);

  const hue = $derived.by(() => {
    const s = seed || label;
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
    return h % 360;
  });
</script>

{#snippet face()}
  {#if shown}
    <img class="avatar" src={shown} alt="" width={size} height={size} onerror={() => (failed = shown)} />
  {:else}
    <span class="avatar initials" style="width:{size}px;height:{size}px;font-size:{Math.round(size * 0.38)}px;--h:{hue}">{initials}</span>
  {/if}
{/snippet}

<!-- The wrapper is there only while the dot is: a list of offline people stays one element per avatar. -->
{#if online}
  <span class="holder">{@render face()}<span class="dot" style="--d:{Math.max(8, Math.round(size * 0.26))}px"></span></span>
{:else}
  {@render face()}
{/if}

<style>
  .holder { position: relative; display: inline-flex; flex-shrink: 0; }
  .dot {
    position: absolute; right: 0; bottom: 0; width: var(--d); height: var(--d); box-sizing: border-box;
    border-radius: 50%; background: var(--success); border: 2px solid var(--surface);
  }
  .avatar { border-radius: 50%; flex-shrink: 0; object-fit: cover; background: var(--surface-2); }
  .initials {
    display: inline-flex; align-items: center; justify-content: center; font-weight: var(--fw-bold); letter-spacing: 0.3px;
    /* Mixed with theme tokens so one rule reads well on dark and light. */
    --tone: hsl(var(--h) 68% 56%);
    background: color-mix(in srgb, var(--tone) 24%, var(--surface-2));
    color: color-mix(in srgb, var(--tone) 62%, var(--text));
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A bio as the runtime parsed it: text in its styles, links, line breaks.
  Text nodes and <a> only; a link opens in the system browser.

  `compact`: at most `lines` lines, the last fading out (a card, a list).
-->
<script lang="ts">
  import { BIO_COLORS, type Span } from '../api';
  import { linkActions } from '../content/actions';
  import type { ExternalUrl } from '../content/types';

  interface Props {
    spans: Span[];
    compact?: boolean;
    lines?: number;
  }
  let { spans, compact = false, lines = 3 }: Props = $props();

  let box = $state<HTMLElement | null>(null);
  let over = $state(false);

  /** A colour of the palette as its class (rules below, one per token); anything else, none. */
  const toneOf = (c: string | null) => (c && (BIO_COLORS as readonly string[]).includes(c) ? `tone-${c}` : undefined);
  /** The runtime gives https links only; anything else stays text. */
  const linkable = (url: string) => /^https:\/\/[^\s]+$/.test(url);

  function follow(e: MouseEvent, url: string) {
    e.preventDefault();
    linkActions.openExternal(url as ExternalUrl).catch(() => {});
  }

  // Whether the clamped text is cut: only then the last line fades.
  $effect(() => {
    const el = box;
    if (!el || !compact) { over = false; return; }
    void spans;
    const measure = () => { over = el.scrollHeight > el.clientHeight + 1; };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });
</script>

<!-- One line: the text keeps its spaces (pre-wrap), so no whitespace may stand between the tags. -->
<div class="bio" class:clamp={compact} class:over bind:this={box} style:--lines={lines}>{#each spans as s, i (i)}{#if s.kind === 'break'}<br />{:else if s.kind === 'link' && linkable(s.url)}<a href={s.url} rel="noopener noreferrer" title={s.url} class:b={s.style.bold} class:i={s.style.italic} class:s={s.style.strike} class:code={s.style.code} class={toneOf(s.style.color)} onclick={(e) => follow(e, s.url)}>{s.text}</a>{:else}<span class:b={s.style.bold} class:i={s.style.italic} class:s={s.style.strike} class:code={s.style.code} class={toneOf(s.style.color)}>{s.text}</span>{/if}{/each}</div>

<style>
  .bio {
    font-size: var(--fs-sm); color: var(--text-body, var(--text)); line-height: 1.45;
    white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word; min-width: 0;
  }
  .clamp { max-height: calc(var(--lines) * 1.45em); overflow: hidden; }
  .clamp.over { mask-image: linear-gradient(to bottom, #000 calc(100% - 1.45em), transparent); }
  .b { font-weight: var(--fw-bold); }
  .i { font-style: italic; }
  .s { text-decoration: line-through; }
  .code {
    font-family: var(--font-mono); font-size: 0.92em; padding: 0 3px; border-radius: 4px;
    background: var(--surface-3); border: 1px solid var(--border);
  }
  a { color: var(--accent-text-2); text-decoration: underline; text-underline-offset: 2px; }
  /* The bio palette: tokens checked for contrast on every surface, chosen by name, never a colour from data. */
  .tone-red { color: var(--bio-red); }
  .tone-orange { color: var(--bio-orange); }
  .tone-yellow { color: var(--bio-yellow); }
  .tone-green { color: var(--bio-green); }
  .tone-teal { color: var(--bio-teal); }
  .tone-blue { color: var(--bio-blue); }
  .tone-purple { color: var(--bio-purple); }
  .tone-pink { color: var(--bio-pink); }
  .tone-gray { color: var(--bio-gray); }
  a.s { text-decoration: underline line-through; }
</style>

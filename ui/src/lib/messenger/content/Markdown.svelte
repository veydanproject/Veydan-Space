<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A message written in Markdown, as the runtime parsed it
  (messenger-richtext::md): blocks and inlines drawn as elements and text
  nodes, never as HTML. A link shows the host it opens beside its words;
  a picture is a link to it, never loaded. `words` draws a run of plain
  words with its bare links (MessageContent's own).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import type { MdBlock, MdInline } from '../generated/markdown';
  import { linkActions } from './actions';
  import { showsHost, wordsOf } from './markdown';
  import type { ExternalUrl } from './types';

  interface Props {
    blocks: MdBlock[];
    words: Snippet<[string]>;
  }
  let { blocks, words }: Props = $props();

  function follow(e: MouseEvent, url: string) {
    e.preventDefault();
    void linkActions.follow(url as ExternalUrl);
  }
</script>

{#snippet inlines(list: MdInline[], linked: boolean)}{#each list as i}{#if i.kind === 'text'}{#if i.style?.bold || i.style?.italic || i.style?.strike}<span class:b={i.style?.bold} class:i={i.style?.italic} class:s={i.style?.strike}>{#if linked}{i.text}{:else}{@render words(i.text)}{/if}</span>{:else if linked}{i.text}{:else}{@render words(i.text)}{/if}{:else if i.kind === 'code'}<code class="ic">{i.text}</code>{:else if i.kind === 'break'}<br />{:else if i.host !== null}<a href={i.url} title={i.url} rel="noreferrer noopener" onclick={(e) => follow(e, i.url)}>{@render inlines(i.children, true)}</a>{#if !showsHost(wordsOf(i.children), i.host)}<span class="host">{i.host}</span>{/if}{:else}<span class="inside" title={i.url}>{@render inlines(i.children, true)}</span>{/if}{/each}{/snippet}

{#snippet render(list: MdBlock[])}
  {#each list as b, n (n)}
    {#if b.kind === 'para'}
      <p>{@render inlines(b.inlines, false)}</p>
    {:else if b.kind === 'heading'}
      <p class="h h{b.level}" role="heading" aria-level={b.level}>{@render inlines(b.inlines, false)}</p>
    {:else if b.kind === 'code'}
      <div class="pre">
        {#if b.lang}<span class="lang">{b.lang}</span>{/if}
        <pre><code>{b.text}</code></pre>
      </div>
    {:else if b.kind === 'quote'}
      <blockquote>{@render render(b.blocks)}</blockquote>
    {:else if b.kind === 'list'}
      <svelte:element this={b.ordered ? 'ol' : 'ul'} start={b.ordered && b.start !== 1 ? b.start : undefined}>
        {#each b.items as item, k (k)}
          <li class:task={item.task !== null}>
            {#if item.task !== null}<span class="box" class:done={item.task} aria-hidden="true">{#if item.task}<Icon name="check" size={10} />{/if}</span>{/if}
            {@render render(item.blocks)}
          </li>
        {/each}
      </svelte:element>
    {:else if b.kind === 'table'}
      <div class="table">
        <table>
          {#if b.head.length}<thead><tr>{#each b.head as cell, k (k)}<th>{@render inlines(cell, false)}</th>{/each}</tr></thead>{/if}
          <tbody>
            {#each b.rows as row, r (r)}<tr>{#each row as cell, k (k)}<td>{@render inlines(cell, false)}</td>{/each}</tr>{/each}
          </tbody>
        </table>
      </div>
    {:else}
      <hr />
    {/if}
  {/each}
{/snippet}

<div class="md">{@render render(blocks)}</div>

<style>
  .md {
    font-size: var(--fs-body); line-height: var(--lh-body); overflow-wrap: anywhere; user-select: text; min-width: 0; max-width: 100%;
    display: flex; flex-direction: column; gap: 6px;
  }
  .md :global(:is(p, blockquote, ul, ol, pre, table, hr)) { margin: 0; }
  blockquote { display: flex; flex-direction: column; gap: 4px; }
  li > :global(* + *) { margin-top: 4px; }
  .b { font-weight: var(--fw-bold); }
  .i { font-style: italic; }
  .s { text-decoration: line-through; }
  .h { font-weight: var(--fw-bold); line-height: 1.3; }
  .h1 { font-size: 1.25em; }
  .h2 { font-size: 1.12em; }
  .h3 { font-size: 1em; }
  a { color: var(--accent-text-2); text-decoration: underline; text-underline-offset: 2px; }
  /* Marks of the direction of writing in the words cannot move the words over the host beside them. */
  a, .host, .inside { unicode-bidi: isolate; }
  /* Where a link leads, whatever its words say. */
  .host {
    display: inline-block; margin-left: 4px; padding: 0 5px; border-radius: var(--radius-xs); vertical-align: baseline;
    font-size: 0.8em; line-height: 1.5; color: var(--text-3); background: color-mix(in srgb, currentColor 8%, transparent); white-space: nowrap;
  }
  .inside { color: var(--accent-text-2); font-weight: var(--fw-semibold); }
  .ic, pre {
    font-family: var(--font-mono); font-size: 0.88em;
    background: color-mix(in srgb, currentColor 9%, transparent); border-radius: var(--radius-xs);
  }
  .ic { padding: 1px 5px; overflow-wrap: anywhere; }
  .pre { position: relative; display: flex; flex-direction: column; min-width: 0; max-width: 100%; }
  .lang { align-self: flex-start; font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--text-3); padding: 0 2px 2px; }
  pre {
    padding: 8px 10px; overflow-x: auto; white-space: pre; overflow-wrap: normal; line-height: 1.45;
    max-width: 100%; touch-action: pan-x pan-y;
    border: 1px solid color-mix(in srgb, currentColor 12%, transparent);
  }
  pre code { font: inherit; background: none; padding: 0; }
  blockquote { padding: 2px 0 2px 10px; border-left: 3px solid color-mix(in srgb, var(--accent) 70%, transparent); color: inherit; opacity: 0.9; }
  ul, ol { padding-left: 1.4em; display: flex; flex-direction: column; gap: 2px; }
  ul { list-style: disc; }
  ol { list-style: decimal; }

  li.task { list-style: none; position: relative; }
  .box {
    position: absolute; left: -1.25em; top: 0.3em; width: 13px; height: 13px; border-radius: 3px;
    border: 1.5px solid color-mix(in srgb, currentColor 45%, transparent); display: inline-flex; align-items: center; justify-content: center;
  }
  .box.done { background: var(--accent); border-color: var(--accent); color: #fff; }
  .table { max-width: 100%; overflow-x: auto; touch-action: pan-x pan-y; }
  table { border-collapse: collapse; font-size: 0.94em; }
  th, td { padding: 4px 8px; border: 1px solid color-mix(in srgb, currentColor 15%, transparent); text-align: left; vertical-align: top; }
  th { font-weight: var(--fw-semibold); background: color-mix(in srgb, currentColor 6%, transparent); }
  hr { border: none; border-top: 1px solid color-mix(in srgb, currentColor 18%, transparent); width: 100%; }
  /* Touch: a long press opens the menu, so it must not start a selection. */
  @media (pointer: coarse) { .md { user-select: none; -webkit-user-select: none; } }
</style>

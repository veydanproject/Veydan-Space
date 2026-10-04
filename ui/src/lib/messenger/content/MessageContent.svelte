<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The text of a message, of a description, of anything a person wrote:
  words, links, and a card for what a link leads to. Every place that
  shows such a text shows it through here.

  `plain`: words only, nothing to press (a quoted message, a line of a list).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { ICONS } from './cards';
  import LinkCard from './LinkCard.svelte';
  import { linkActions } from './actions';
  import { linkStore } from './linkStore.svelte';
  import { linksOf, tokenize } from './tokenize';
  import type { InternalLinkText } from './types';

  /** More links than this in one message get no cards of their own. */
  const MAX_CARDS = 3;

  interface Props {
    text: string;
    plain?: boolean;
    /** Links are shown; cards are not. */
    cards?: boolean;
  }
  let { text, plain = false, cards = true }: Props = $props();

  const segments = $derived(tokenize(text));
  const links = $derived(plain || !cards ? [] : linksOf(segments).slice(0, MAX_CARDS));
  /** A message that is a link and nothing else is shown as its card alone. */
  const bare = $derived(links.length === 1 && links[0].type === 'internal' && segments.every((s) => s.type !== 'text' || !s.text.trim()));

  /** What stands in the text in place of a link inside. */
  function label(link: InternalLinkText): { icon: string; text: string } {
    const v = linkStore.view(link);
    const short = link.length > 28 ? `${link.slice(0, 24)}…` : link;
    if (!v) return { icon: 'link', text: short };
    if (v.kind === 'group') return { icon: ICONS.group, text: v.name.trim() || $t('msg_group_unnamed') };
    if (v.kind === 'contact') return { icon: ICONS.contact, text: v.name.trim() || `${v.npub.slice(0, 12)}…` };
    return { icon: ICONS[v.kind], text: short };
  }

  function follow(e: MouseEvent, url: Parameters<typeof linkActions.openExternal>[0]) {
    e.preventDefault();
    linkActions.openExternal(url).catch(() => {});
  }
</script>

{#if !bare}
  <span class="text">{#each segments as s}{#if s.type === 'text'}{s.text}{:else if s.type === 'external'}{#if plain}{s.text}{:else}<a href={s.url} rel="noreferrer noopener" onclick={(e) => follow(e, s.url)}>{s.text}</a>{/if}{:else}{@const l = label(s.link)}<span class="chip" title={s.text}><Icon name={l.icon} size={11} />{l.text}</span>{/if}{/each}</span>
{/if}

{#if links.length}
  <div class="cards">
    {#each links as l (l.text)}<LinkCard link={l} />{/each}
  </div>
{/if}

<style>
  .text { font-size: var(--fs-sm); line-height: 1.45; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .text a { color: var(--accent-text-2); text-decoration: underline; text-underline-offset: 2px; overflow-wrap: anywhere; }
  .chip {
    display: inline-flex; align-items: center; gap: 4px; vertical-align: baseline; max-width: 100%;
    padding: 0 6px; border-radius: var(--radius-pill); background: var(--surface-3); color: var(--accent-text-2);
    font-weight: var(--fw-semibold); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .cards { display: flex; flex-direction: column; gap: 6px; width: min(320px, 100%); min-width: min(260px, 100%); }
  /* Touch: a long press opens the menu, so it must not start a selection. */
  @media (pointer: coarse) { .text { user-select: none; -webkit-user-select: none; } }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «печатает…» as the kit draws it in a line (typing-indicator, inline;
  motion M09): the words, then three dots that rise in turn. `who`: the
  names of those who type, none for the peer of a direct chat (the header
  names them). The accent of the header's status; `muted` takes the colour
  of the line around it. A screen reader hears the words once, not the dots.
-->
<script lang="ts">
  import { locale, t } from '$lib/core/i18n';
  import { typingText } from './labels';

  interface Props {
    who?: readonly string[];
    muted?: boolean;
  }
  let { who = [], muted = false }: Props = $props();
</script>

<span class="typing" class:muted role="status">
  <span class="words">{typingText(who, $t, $locale)}</span>
  <span class="dots" aria-hidden="true"><span></span><span></span><span></span></span>
</span>

<style>
  .typing { display: inline-flex; align-items: center; gap: var(--sp-1); min-width: 0; max-width: 100%; color: var(--accent-text); white-space: nowrap; }
  .typing.muted { color: inherit; }
  .words { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .dots { display: inline-flex; align-items: center; gap: 2px; flex: none; }
  .dots > span { width: 4px; height: 4px; border-radius: var(--radius-pill); background: currentColor; opacity: 0.4; animation: typing-dot 1.2s ease-in-out infinite; }
  .dots > span:nth-child(2) { animation-delay: 150ms; }
  .dots > span:nth-child(3) { animation-delay: 300ms; }
  @keyframes typing-dot {
    0%, 60%, 100% { transform: translateY(0); opacity: 0.4; }
    30% { transform: translateY(-4px); opacity: 1; }
  }
  /* Less motion: the dots rest, the words say it. */
  @media (prefers-reduced-motion: reduce) { .dots > span { animation: none; } }
</style>

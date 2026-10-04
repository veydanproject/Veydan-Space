<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The in-page twin of `Drawer`: the same header, subheader, scrolling body and
  footer, laid out as a card that fills its place in a page instead of
  sliding over the window. A component written for a drawer renders into a
  page by switching its frame (`as="pane"`), with no second copy of its body.
  `open` is accepted so both frames take the same props; a pane is always shown.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open?: boolean;
    title?: string;
    /** Accepted for parity with Drawer; a pane takes the width of its place. */
    width?: string;
    titleBadge?: Snippet;
    actions?: Snippet;
    subheader?: Snippet;
    children: Snippet;
    footer?: Snippet;
  }

  let { open = $bindable(true), title, titleBadge, actions, subheader, children, footer }: Props = $props();
</script>

<section class="pane">
  {#if title || actions}
    <header class="pane-header">
      {#if title}<h2 class="pane-title">{title}</h2>{/if}
      {@render titleBadge?.()}
      <div class="pane-actions">
        {@render actions?.()}
      </div>
    </header>
  {/if}

  {#if subheader}
    <div class="pane-subheader">{@render subheader()}</div>
  {/if}

  <div class="pane-body">
    {@render children()}
  </div>

  {#if footer}
    <footer class="pane-footer">{@render footer()}</footer>
  {/if}
</section>

<style>
  .pane {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  .pane-header {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    height: 58px;
    padding: 0 var(--sp-4) 0 var(--sp-5);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .pane-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-extrabold);
    letter-spacing: -0.3px;
    color: var(--text);
  }
  .pane-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-left: auto;
  }

  .pane-subheader {
    flex-shrink: 0;
  }

  .pane-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-gutter: stable;
    padding: var(--sp-4);
  }

  .pane-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    border-top: 1px solid var(--border);
    background: var(--surface-drawer-footer);
    flex-shrink: 0;
  }
</style>

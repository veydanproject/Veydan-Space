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
    /** Small grey text on the title's line, e.g. the number of entries. */
    subtitle?: Snippet;
    titleBadge?: Snippet;
    actions?: Snippet;
    subheader?: Snippet;
    children: Snippet;
    footer?: Snippet;
  }

  let { open = $bindable(true), title, subtitle, titleBadge, actions, subheader, children, footer }: Props = $props();
</script>

<section class="pane on-glass">
  {#if title || actions}
    <header class="pane-header">
      <div class="pane-titles">
        <div class="pane-title-row">
          {#if title}<h2 class="pane-title">{title}</h2>{/if}
          {#if subtitle}<span class="pane-sub">{@render subtitle()}</span>{/if}
        </div>
        {@render titleBadge?.()}
      </div>
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
    background: var(--pane);
    border: 1px solid var(--pane-border);
    border-radius: var(--radius-pane);
    overflow: hidden;
    box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter);
    backdrop-filter: var(--pane-filter);
  }

  /* The column's head: a line of 36, 16 from the top, no band and no rule under it. */
  .pane-header {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-4) var(--sp-3) 0 var(--sp-4);
    flex-shrink: 0;
  }
  .pane-titles {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    min-height: 36px;
  }
  .pane-title-row {
    display: flex;
    align-items: baseline;
    gap: var(--sp-2);
    min-width: 0;
  }
  .pane-title {
    font-size: var(--fs-lg);
    font-weight: var(--fw-semibold);
    line-height: var(--lh-tight);
    color: var(--text);
  }
  .pane-sub {
    font-size: var(--fs-xs);
    color: var(--text-2);
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

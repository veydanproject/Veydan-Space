<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The toast of the app (toast.svelte.ts), in each shell. A computer: bottom
  right, 24 from the window's edges, 360 wide. A phone (the owner's choice,
  2026-10-16): at the top under the status bar, the whole width less 16 on
  each side, where nothing at the bottom (keyboard, composer, sheet) hides it.
-->
<script lang="ts">
  import { portal } from '$lib/core/portal';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { toast } from './toast.svelte';

  const ICON = { success: 'check', secret: 'copy', undo: 'trash', error: 'alert-triangle', info: 'info' } as const;

  const cur = $derived(toast.current);
  const action = $derived.by(() => {
    if (!cur || cur.action === false) return null;
    if (cur.action) return cur.action;
    if (cur.type === 'undo') return $t('toast_undo');
    if (cur.type === 'error' && cur.onAction) return $t('toast_retry');
    if (cur.type === 'secret' && cur.onAction) return $t('toast_clear_now');
    return null;
  });
  const countdown = $derived(!!cur && (cur.type === 'secret' || cur.type === 'undo') && cur.total > 0);

</script>

{#if cur}
  <div class="region" class:phone={isMobile} use:portal>
    {#key cur.id}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="vtoast {cur.type}" role={cur.type === 'error' ? 'alert' : 'status'}
        onpointerenter={() => (toast.paused = true)} onpointerleave={() => (toast.paused = false)}
        onfocusin={() => (toast.paused = true)} onfocusout={() => (toast.paused = false)}>
        <span class="ic"><Icon name={cur.icon ?? ICON[cur.type]} size={18} /></span>
        <span class="text">
          <span class="title">{cur.title}</span>
          {#if cur.type === 'secret' && !cur.sub}
            <span class="sub">{$t('toast_clipboard_clears', { n: String(Math.ceil(cur.left / 1000)) })}</span>
          {:else if cur.sub}
            <span class="sub">{cur.sub}</span>
          {/if}
        </span>
        {#if action}<button class="action" onclick={() => toast.act()}>{action}</button>{/if}
        {#if cur.type === 'error'}
          <button class="close" onclick={() => toast.close(cur.id)} aria-label={$t('toast_close')} title={$t('toast_close')}><Icon name="x" size={16} /></button>
        {/if}
        {#if countdown}<span class="bar" style:width="{(cur.left / cur.total) * 100}%"></span>{/if}
      </div>
    {/key}
  </div>
{/if}

<style>
  .region {
    position: fixed; right: var(--sp-6); bottom: var(--sp-6); z-index: var(--z-toast);
    width: 360px; max-width: calc(100vw - 2 * var(--sp-6)); pointer-events: none;
  }
  .region.phone { left: var(--sp-4); right: var(--sp-4); top: calc(var(--sat, 0px) + var(--sp-2)); bottom: auto; width: auto; max-width: none; }
  .vtoast {
    position: relative; display: flex; align-items: center; gap: var(--sp-3); min-height: 44px; overflow: hidden;
    padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-4); pointer-events: auto;
    background: var(--glass-bg-raised); -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
    border: 1px solid var(--glass-border); border-radius: var(--radius); box-shadow: var(--shadow-lg), var(--glass-highlight);
    color: var(--text);
  }
  .ic { display: inline-flex; flex-shrink: 0; color: var(--success-text); }
  .undo .ic { color: var(--text-soft); }
  .error .ic { color: var(--danger-text); }
  .info .ic { color: var(--accent-text); }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding: 4px 0; }
  .title { font-size: var(--fs-sm); font-weight: var(--fw-semibold); line-height: 1.4; overflow-wrap: anywhere; }
  .sub { font-size: var(--fs-xs); font-weight: var(--fw-medium); color: var(--text-2); overflow-wrap: anywhere; }
  .action {
    flex-shrink: 0; height: 32px; padding: 0 var(--sp-2); border: 0; border-radius: var(--radius-sm); background: none;
    color: var(--accent-text); font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-bold); white-space: nowrap; cursor: pointer;
  }
  .action:hover { background: var(--surface-hover); color: var(--text); }
  .close {
    flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; width: 32px; height: 32px; padding: 0;
    border: 0; border-radius: var(--radius-sm); background: none; color: var(--text-2); cursor: pointer;
  }
  .close:hover { background: var(--surface-hover); color: var(--text); }
  .bar { position: absolute; left: 0; bottom: 0; height: 2px; background: var(--accent); transition: width 250ms linear; }
  .phone .action { height: 44px; margin: -6px 0; }
  @media (prefers-reduced-motion: reduce) { .bar { transition: none; } }
</style>

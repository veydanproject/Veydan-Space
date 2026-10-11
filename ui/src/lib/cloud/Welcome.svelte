<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Veydan Cloud for an identity that has no account yet (vcloud-plan.md 4.3, 4.5): what the free plan gives and its activation. The proof of work runs on the device for a few seconds, and the line under the button shows it moving. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import type { CloudActivation } from '$lib/cloud/api';
  import { activating } from '$lib/cloud/status.svelte';
  import { errorKeyOf, formatBytes } from '$lib/cloud/account';
  import Notice from '$lib/cloud/Notice.svelte';

  let {
    activation,
    freeFiles = null,
    onActivate,
    onKey,
  }: {
    activation: CloudActivation;
    /** The room for files of the free plan, when the plans are known. */
    freeFiles?: number | null;
    onActivate: () => void;
    onKey: () => void;
  } = $props();

  const running = $derived(activating(activation));
  /** How far the work is, by the attempts it is expected to take; never full before the server answered. */
  const progress = $derived(
    activation.phase === 'activating' ? 96 : activation.expected > 0 ? Math.min(92, Math.round((activation.attempts / activation.expected) * 100)) : 4,
  );
  const failure = $derived.by(() => {
    const e = activation.phase === 'failed' ? activation.error : null;
    if (!e) return '';
    const key = errorKeyOf(e.code);
    return key ? $t(key) : e.message || e.code;
  });
</script>

<section class="welcome" class:m={isMobile} aria-labelledby="cloud-free-title">
  <span class="tile"><Icon name="cloud" size={26} /></span>
  <h2 id="cloud-free-title">{$t('cloud_free_title')}</h2>
  {#if freeFiles}
    <!-- The room is the server's number: the sentence that names its own is kept for a host without the list of plans. -->
    <p class="text">{$t('cloud_plan_free_price')} · {$t('cloud_plan_free_forever')}</p>
  {:else}
    <p class="text">{$t('cloud_free_text')}</p>
  {/if}
  <ul class="gives">
    {#if freeFiles}<li><Icon name="folder" size={16} /><span>{$t('cloud_plan_f_files', { size: formatBytes(freeFiles) })}</span></li>{/if}
    <li><Icon name="refresh-cw" size={16} /><span>{$t('cloud_plan_f_sync')}</span></li>
    <li><Icon name="key" size={16} /><span>{$t('cloud_plan_f_recovery')}</span></li>
  </ul>
  <div class="actions">
    <button class="btn btn-primary" type="button" disabled={running} onclick={onActivate}>
      {running ? $t('cloud_activating') : $t('cloud_activate')}
    </button>
    <button class="btn btn-ghost" type="button" disabled={running} onclick={onKey}><Icon name="key" size={14} />{$t('cloud_key_have')}</button>
  </div>
  {#if running}
    <div class="work" role="status">
      <div class="bar" aria-hidden="true"><span class="fill" style:width="{progress}%"></span></div>
      <span>{$t('cloud_activating_hint')}</span>
    </div>
  {/if}
  {#if failure}
    <Notice tone="danger">{$t('cloud_activation_failed', { error: failure })}</Notice>
  {/if}
</section>

<style>
  .welcome { display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-3); }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: 52px; height: 52px;
    border-radius: var(--radius-md); background: var(--accent-grad); color: #fff; box-shadow: var(--shadow-accent);
  }
  h2 { margin: 0; font-size: var(--fs-xl); font-weight: var(--fw-extrabold); letter-spacing: -0.01em; color: var(--text); }
  .text { margin: 0; font-size: var(--fs-base); color: var(--text-2); line-height: 1.5; max-width: 520px; }
  .gives { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .gives li { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); color: var(--text-body); }
  .gives li :global(svg) { color: var(--accent-text); flex-shrink: 0; }
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; margin-top: var(--sp-1); }
  .m .actions { align-self: stretch; flex-direction: column; }
  .m .actions .btn { min-height: 48px; font-size: 15px; }
  .work { align-self: stretch; display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); color: var(--text-2); }
  .bar { height: 4px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .fill { display: block; height: 100%; background: var(--accent); transition: width 0.4s ease-out; }
</style>

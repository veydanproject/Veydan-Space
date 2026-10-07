<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- One-time view of a recovery key: copy, optional file save, acknowledge, done. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { product } from '$lib/core/product';

  interface Props {
    code: string;
    title?: string;
    intro?: string;
    ondone: () => void;
  }

  let { code, title, intro, ondone }: Props = $props();

  // Two halves of whole groups: a card too narrow for the key on one line
  // shows it as two rows of three groups, never a group split in two.
  const halves = $derived.by(() => {
    const groups = code.split('-');
    const cut = Math.ceil(groups.length / 2);
    return [groups.slice(0, cut).join('-'), groups.slice(cut).join('-')].filter(Boolean);
  });

  let copied = $state(false);
  let acknowledged = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    await navigator.clipboard.writeText(code);
    copied = true;
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = false), 2000);
  }

  async function saveToFile() {
    const { save } = await import('@tauri-apps/plugin-dialog');
    const dest = await save({ defaultPath: `${product.id === 'space' ? 'veydan' : `veydan-${product.id}`}-recovery-key.txt` });
    if (!dest) return;
    const { writeTextFile } = await import('@tauri-apps/plugin-fs');
    await writeTextFile(dest, `${$t('lock_recovery_file_title')}\n${code}\n`);
  }
</script>

<div class="recovery" class:mobile={isMobile}>
  <div class="head">
    <Icon name="key" size={24} />
    <h3>{title ?? $t('lock_recovery_title')}</h3>
  </div>
  <p class="intro">{intro ?? $t('lock_recovery_intro')}</p>
  <div class="code mono">{#each halves as half, i (i)}<span class="half">{half}{i < halves.length - 1 ? '-' : ''}</span>{/each}</div>
  <!-- The phone stacks full-width buttons; the desktop keeps the compact row of its cards. -->
  <div class="actions">
    <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" onclick={copy}>
      <Icon name={copied ? 'check' : 'copy'} size={16} />
      {copied ? $t('lock_recovery_copied') : $t('lock_recovery_copy')}
    </button>
    {#if !isMobile}
      <button class="btn btn-ghost btn-sm" type="button" onclick={saveToFile}>
        <Icon name="download" size={16} />
        {$t('lock_recovery_save_file')}
      </button>
    {/if}
  </div>
  <div class="finish">
    <label class="ack">
      <input type="checkbox" bind:checked={acknowledged} />
      <span>{$t('lock_recovery_ack')}</span>
    </label>
    <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" disabled={!acknowledged} onclick={ondone}>
      {$t('lock_recovery_done')}
    </button>
  </div>
</div>

<style>
  .recovery { display: flex; flex-direction: column; gap: var(--sp-3); max-width: 560px; }
  .recovery.mobile { max-width: none; }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .finish { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); }
  .finish .ack { flex: 1; min-width: 0; }
  .mobile .actions,
  .mobile .finish { flex-direction: column; align-items: stretch; }
  .mobile .actions .btn,
  .mobile .finish .btn { width: 100%; min-height: 44px; justify-content: center; }
  .head { display: flex; align-items: center; gap: var(--sp-2); color: var(--text); }
  .head h3 { margin: 0; font-size: var(--fs-md); font-weight: 600; }
  .intro { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .code {
    padding: var(--sp-3);
    border: 1px dashed var(--accent-border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: 1.05rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-align: center;
    user-select: all;
    color: var(--text);
  }
  /* A narrow card breaks the key between its halves only (three groups each). */
  .code .half { display: inline-block; white-space: nowrap; }
  .ack { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text); cursor: pointer; }
  /* Global input rules set width 100% and a touch min-height, which turns the box into a full-width slab. */
  .ack input[type="checkbox"] {
    /* The mark of base.css */
    margin: 2px 0 0;
    flex: 0 0 auto;
  }
  .ack span { flex: 1; min-width: 0; line-height: 1.4; }
</style>

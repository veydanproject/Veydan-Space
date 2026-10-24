<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One-time view of a recovery key. The Recovery Key of 12 words (platform-spec
  8.1): the words numbered, «Я записал(а)», then three words typed back by
  their numbers before Done. The code of 24 characters of an older lock:
  copy, optional file save, acknowledge, done.
-->
<script lang="ts">
  import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { product } from '$lib/core/product';
  import { pickPositions, splitWords, wordsMatch, RECOVERY_WORDS } from '$lib/core/lock/words';

  interface Props {
    /** The 12 words, one space between, or the code of 24 characters in groups. */
    code: string;
    title?: string;
    intro?: string;
    ondone: () => void;
  }

  let { code, title, intro, ondone }: Props = $props();

  const words = $derived(splitWords(code));
  const isWords = $derived(words.length === RECOVERY_WORDS);

  // Two halves of whole groups: a card too narrow for the code on one line
  // shows it as two rows of three groups, never a group split in two.
  const halves = $derived.by(() => {
    const groups = code.split('-');
    const cut = Math.ceil(groups.length / 2);
    return [groups.slice(0, cut).join('-'), groups.slice(cut).join('-')].filter(Boolean);
  });

  let copied = $state(false);
  let acknowledged = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  // The check of the words: three positions (1-based), typed back.
  let checking = $state(false);
  let positions = $state<number[]>([]);
  let typed = $state<string[]>(['', '', '']);
  let wrong = $state(false);

  function startCheck() {
    positions = pickPositions(words.length);
    typed = ['', '', ''];
    wrong = false;
    checking = true;
  }

  const typedOk = $derived(typed.every((w) => w.trim().length > 0));

  function confirmWords() {
    if (wordsMatch(words, positions, typed)) {
      ondone();
    } else {
      wrong = true;
    }
  }

  function showAgain() {
    checking = false;
    wrong = false;
  }

  async function copy() {
    await navigator.clipboard.writeText(code);
    copiedToast();
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
  {#if isWords && checking}
    <div class="head">
      <Icon name="key" size={24} />
      <h3>{$t('lock_recovery_check_title')}</h3>
    </div>
    <p class="intro">
      {$t('lock_recovery_check_hint', { a: String(positions[0]), b: String(positions[1]), c: String(positions[2]) })}
    </p>
    <div class="check">
      {#each positions as p, i (p)}
        <label class="check-word">
          <span>{$t('lock_recovery_check_word', { n: String(p) })}</span>
          <input
            class="mono"
            type="text"
            bind:value={typed[i]}
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            oninput={() => (wrong = false)}
          />
        </label>
      {/each}
    </div>
    {#if wrong}<p class="err">{$t('lock_recovery_check_wrong')}</p>{/if}
    <div class="finish">
      <button class="btn btn-ghost" class:btn-sm={!isMobile} type="button" onclick={showAgain}>
        {$t('lock_recovery_check_back')}
      </button>
      <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" disabled={!typedOk} onclick={confirmWords}>
        {$t('lock_recovery_done')}
      </button>
    </div>
  {:else}
    <div class="head">
      <Icon name="key" size={24} />
      <h3>{title ?? (isWords ? $t('lock_recovery_words_title') : $t('lock_recovery_title'))}</h3>
    </div>
    <p class="intro">{intro ?? (isWords ? $t('lock_recovery_words_intro') : $t('lock_recovery_intro'))}</p>
    {#if isWords}
      <ol class="words mono">
        {#each words as word, i (i)}
          <li><span class="num">{i + 1}</span>{word}</li>
        {/each}
      </ol>
    {:else}
      <div class="code mono">{#each halves as half, i (i)}<span class="half">{half}{i < halves.length - 1 ? '-' : ''}</span>{/each}</div>
    {/if}
    <!-- The phone stacks full-width buttons; the desktop keeps the compact row of its cards. -->
    <div class="actions">
      <!-- The way to keep the key, not the step's end: the end («Готово», «Продолжить») is the one primary button. -->
      <button class="btn btn-ghost" class:btn-sm={!isMobile} type="button" onclick={copy}>
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
      {#if isWords}
        <span class="ack-text">{$t('lock_recovery_words_ack')}</span>
        <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" onclick={startCheck}>
          {$t('lock_recover_continue')}
        </button>
      {:else}
        <label class="ack">
          <input type="checkbox" bind:checked={acknowledged} />
          <span>{$t('lock_recovery_ack')}</span>
        </label>
        <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" disabled={!acknowledged} onclick={ondone}>
          {$t('lock_recovery_done')}
        </button>
      {/if}
    </div>
  {/if}
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
  .mobile .finish .btn { width: 100%; min-height: var(--sp-12); justify-content: center; }
  .head { display: flex; align-items: center; gap: var(--sp-2); color: var(--text); }
  .head h3 { margin: 0; font-size: var(--fs-md); font-weight: 600; }
  .intro { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .code {
    padding: var(--sp-3);
    border: 1px dashed var(--accent-border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: var(--fs-md);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-align: center;
    user-select: all;
    color: var(--text);
  }
  /* A narrow card breaks the key between its halves only (three groups each). */
  .code .half { display: inline-block; white-space: nowrap; }
  /* The 12 words in three columns, numbered, large enough to copy by hand. */
  .words {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px var(--sp-3);
    margin: 0;
    padding: var(--sp-3);
    list-style: none;
    border: 1px dashed var(--accent-border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: var(--fs-md);
    font-weight: 600;
    color: var(--text);
    user-select: all;
  }
  .mobile .words { grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--sp-2); font-size: var(--fs-body); }
  .words li { display: flex; align-items: baseline; gap: 6px; white-space: nowrap; }
  .words .num { min-width: 1.6em; font-size: var(--fs-xs); font-weight: 400; color: var(--text-3); text-align: right; }
  .ack-text { flex: 1; min-width: 0; font-size: var(--fs-sm); color: var(--text); }
  .check { display: flex; flex-direction: column; gap: var(--sp-2); }
  .check-word { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-3); }
  .check-word input {
    width: 100%;
    min-height: 40px;
    padding: 6px 10px;
    font: inherit;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-2);
    color: var(--text);
  }
  .err { margin: 0; font-size: var(--fs-xs); color: var(--danger-text); }
  .ack { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text); cursor: pointer; }
  /* Global input rules set width 100% and a touch min-height, which turns the box into a full-width slab. */
  .ack input[type="checkbox"] {
    /* The mark of base.css */
    margin: 2px 0 0;
    flex: 0 0 auto;
  }
  .ack span { flex: 1; min-width: 0; line-height: 1.4; }
</style>

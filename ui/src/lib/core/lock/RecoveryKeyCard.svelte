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
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { saveAs } from '$lib/core/ui/saveAs';
  import { isMobile } from '$lib/core/platform';
  import { product } from '$lib/core/product';
  import { pickPositions, splitWords, wordsMatch, RECOVERY_WORDS } from '$lib/core/lock/words';

  interface Props {
    /** The 12 words, one space between, or the code of 24 characters in groups. */
    code: string;
    title?: string;
    intro?: string;
    ondone: () => void;
    /** The host keeps the buttons at its foot (a phone's step: the foot stays in sight, the words scroll): they are given in `foot`, not drawn under the key. */
    pinned?: boolean;
    foot?: Snippet;
  }

  let { code, title, intro, ondone, pinned = false, foot = $bindable() }: Props = $props();

  $effect(() => {
    if (pinned) foot = isWords && checking ? checkButtons : keyButtons;
  });

  const words = $derived(splitWords(code));
  const isWords = $derived(words.length === RECOVERY_WORDS);

  // Two halves of whole groups: a card too narrow for the code on one line
  // shows it as two rows of three groups, never a group split in two.
  const halves = $derived.by(() => {
    const groups = code.split('-');
    const cut = Math.ceil(groups.length / 2);
    return [groups.slice(0, cut).join('-'), groups.slice(cut).join('-')].filter(Boolean);
  });

  let acknowledged = $state(false);

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
  }

  async function saveToFile() {
    const text = `${$t('lock_recovery_file_title')}\n${code}\n`;
    await saveAs({ defaultPath: `${product.id === 'space' ? 'veydan' : `veydan-${product.id}`}-recovery-key.txt` }, async (dest) => {
      const { writeTextFile } = await import('@tauri-apps/plugin-fs');
      await writeTextFile(dest, text);
    });
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
    {#if wrong}<p class="error-msg" role="alert">{$t('lock_recovery_check_wrong')}</p>{/if}
    {#if !pinned}<div class="finish">{@render checkButtons()}</div>{/if}
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
    {#if !pinned}<div class="actions">{@render copyButtons()}</div>{/if}
    <div class="finish">
      {#if isWords}
        <span class="ack-text">{$t('lock_recovery_words_ack')}</span>
      {:else}
        <label class="ack">
          <input type="checkbox" bind:checked={acknowledged} />
          <span>{$t('lock_recovery_ack')}</span>
        </label>
      {/if}
      {#if !pinned}{@render doneButton()}{/if}
    </div>
  {/if}
</div>

{#snippet copyButtons()}
  <!-- The way to keep the key, not the step's end: the end («Готово», «Продолжить») is the one primary button. -->
  <button class="btn btn-ghost" class:btn-sm={!isMobile} type="button" onclick={copy}>
    <Icon name="copy" size={16} />
    {$t('lock_recovery_copy')}
  </button>
  {#if !isMobile}
    <button class="btn btn-ghost btn-sm" type="button" onclick={saveToFile}>
      <Icon name="download" size={16} />
      {$t('lock_recovery_save_file')}
    </button>
  {/if}
{/snippet}

{#snippet doneButton()}
  {#if isWords}
    <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" onclick={startCheck}>
      {$t('lock_recover_continue')}
    </button>
  {:else}
    <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" disabled={!acknowledged} onclick={ondone}>
      {$t('common_done')}
    </button>
  {/if}
{/snippet}

<!-- At the host's foot: the step's verb first (a phone's foot puts it on top), the way to keep the key under it. -->
{#snippet keyButtons()}
  {@render doneButton()}
  {@render copyButtons()}
{/snippet}

{#snippet checkButtons()}
  {#if pinned}{@render checkDone()}{/if}
  <button class="btn btn-ghost" class:btn-sm={!isMobile} type="button" onclick={showAgain}>
    {$t('lock_recovery_check_back')}
  </button>
  {#if !pinned}{@render checkDone()}{/if}
{/snippet}

{#snippet checkDone()}
  <button class="btn btn-primary" class:btn-sm={!isMobile} type="button" disabled={!typedOk} onclick={confirmWords}>
    {$t('common_done')}
  </button>
{/snippet}

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
  .head h3 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-semibold); }
  .intro { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: var(--lh-snug); }
  .code {
    padding: var(--sp-3);
    border: 1px dashed var(--accent-border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
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
    gap: var(--sp-1-5) var(--sp-3);
    margin: 0;
    padding: var(--sp-3);
    list-style: none;
    border: 1px dashed var(--accent-border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
    color: var(--text);
    user-select: all;
  }
  .mobile .words { grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--sp-2); font-size: var(--fs-body); }
  .words li { display: flex; align-items: baseline; gap: var(--sp-1-5); white-space: nowrap; }
  .words .num { min-width: 1.6em; font-size: var(--fs-xs); font-weight: var(--fw-normal); color: var(--text-3); text-align: right; }
  .ack-text { flex: 1; min-width: 0; font-size: var(--fs-sm); color: var(--text); }
  .check { display: flex; flex-direction: column; gap: var(--sp-2); }
  .check-word { display: flex; flex-direction: column; gap: var(--sp-1); font-size: var(--fs-xs); color: var(--text-3); }
  .check-word input {
    width: 100%;
    min-height: var(--field-h);
    padding: var(--sp-1-5) var(--sp-2-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    background: var(--bg-2);
    color: var(--text);
  }
  .error-msg { margin: 0; }
  .ack { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text); cursor: pointer; }
  /* Global input rules set width 100% and a touch min-height, which turns the box into a full-width slab. */
  .ack input[type="checkbox"] {
    /* The mark of base.css */
    margin: var(--sp-0-5) 0 0;
    flex: 0 0 auto;
  }
  .ack span { flex: 1; min-width: 0; line-height: var(--lh-snug); }
</style>

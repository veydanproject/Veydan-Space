<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import FilePanel from '$lib/ssh/files/FilePanel.svelte';
  import TransferStrip from '$lib/ssh/files/TransferStrip.svelte';
  import ConflictDialog from '$lib/ssh/files/ConflictDialog.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { filesStore } from '$lib/ssh/store/sftp.svelte';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import { t } from '$lib/core/i18n';

  let promptAnswer = $state('');

  onMount(() => {
    sshStore.ensureLoaded();
    filesStore.ensureLoaded();
  });

  function onKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    // Don't hijack keys inside inputs (path editing, dialogs)
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')) return;

    if (e.key === 'Tab') {
      e.preventDefault();
      filesStore.activePanel = filesStore.activePanel === 0 ? 1 : 0;
    } else if (e.key === 'F5') {
      e.preventDefault();
      filesStore.copyToOtherPanel(filesStore.activePanel);
    } else if (e.code === 'KeyA' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      filesStore.selectAll(filesStore.activePanel);
    }
  }

  function submitPrompt() {
    filesStore.respondPrompt(promptAnswer);
    promptAnswer = '';
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="page page--fill files-page">
  <div class="page-header">
    <div class="page-title-group">
      <h1>{$t('files_title')}</h1>
      <p class="page-sub">{$t('files_sub')}</p>
    </div>
  </div>

  <div class="panels">
    <FilePanel index={0} />
    <FilePanel index={1} />
  </div>

  <TransferStrip />
</div>

<ConflictDialog />

<!-- Host-key trust question or keyboard-interactive (2FA) prompt during SFTP connect -->
<Dialog
  open={filesStore.prompt !== null}
  title={filesStore.prompt?.hostKey ? $t('ssh_hostkey_title') : filesStore.prompt?.name || $t('files_prompt_title')}
  onclose={() => filesStore.cancelPrompt()}
>
  {#if filesStore.prompt?.hostKey}
    <p class="prompt-instructions">{$t('ssh_hostkey_text')}</p>
    <code class="prompt-fingerprint">{filesStore.prompt.instructions}</code>
  {:else if filesStore.prompt}
    {#if filesStore.prompt.instructions}
      <p class="prompt-instructions">{filesStore.prompt.instructions}</p>
    {/if}
    <div class="form-group">
      <label for="sftp-prompt-input">{filesStore.prompt.prompts[0]?.prompt ?? ''}</label>
      <input
        id="sftp-prompt-input"
        type={filesStore.prompt.prompts[0]?.echo ? 'text' : 'password'}
        bind:value={promptAnswer}
        onkeydown={(e) => e.key === 'Enter' && submitPrompt()}
      />
    </div>
  {/if}
  {#snippet footer()}
    {#if filesStore.prompt?.hostKey}
      <button class="btn btn-ghost" onclick={() => filesStore.respondPrompt('no')}>{$t('ssh_hostkey_reject')}</button>
      <button class="btn btn-primary" onclick={() => filesStore.respondPrompt('yes')}>{$t('ssh_hostkey_trust')}</button>
    {:else}
      <button class="btn btn-ghost" onclick={() => filesStore.cancelPrompt()}>{$t('cancel')}</button>
      <button class="btn btn-primary" onclick={submitPrompt}>{$t('files_prompt_send')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .files-page {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .panels {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--sp-3);
  }

  .files-page :global(.transfer-strip) {
    margin-top: var(--sp-3);
  }

  .prompt-fingerprint {
    display: block;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    word-break: break-all;
    padding: 0.4rem 0.6rem;
    border-radius: var(--radius-sm);
    background: var(--surface-3);
    color: var(--text);
  }
  .prompt-instructions {
    margin-bottom: var(--sp-3);
    color: var(--text-2);
    font-size: var(--fs-sm);
    white-space: pre-wrap;
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The pass module's controls in a row of a note's context sheet for a
  password: copy the username, copy the password (through the lock).
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { hasErrorCode } from '$lib/core/utils';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { api } from '$lib/pass/api';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';

  let { id }: { id: string } = $props();

  const entry = $derived(passwordStore.list.find((e) => e.id === id));
  /** `user` or `pass`: the value just copied */
  let copied = $state('');
  let copyTimer: ReturnType<typeof setTimeout>;
  let unlockOpen = $state(false);
  let pendingCopy = $state(false);

  function markCopied(key: string) {
    copied = key;
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = ''), 1600);
  }

  async function writeClipboard(value: string) {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      const el = document.createElement('textarea');
      el.value = value;
      el.setAttribute('readonly', '');
      el.style.position = 'fixed';
      el.style.left = '-9999px';
      document.body.appendChild(el);
      el.select();
      document.execCommand('copy');
      el.remove();
    }
  }

  async function copyUsername() {
    if (!entry?.username) return;
    await writeClipboard(entry.username);
    markCopied('user');
  }

  async function copyPassword() {
    if (appLock.locked) {
      pendingCopy = true;
      unlockOpen = true;
      return;
    }
    try {
      const value = (await api.passwords.reveal(id, 'password')).value;
      await writeClipboard(value);
      markCopied('pass');
      setTimeout(() => {
        navigator.clipboard.readText().then((current) => {
          if (current === value) void navigator.clipboard.writeText('');
        }).catch(() => {});
      }, 30_000);
    } catch (e) {
      if (hasErrorCode(e, 'vault_locked') || hasErrorCode(e, 'vault_mismatch') || hasErrorCode(e, 'decrypt_failed')) {
        pendingCopy = true;
        unlockOpen = true;
      }
    }
  }
</script>

{#if entry?.username}
  <button type="button" class="chev" class:ok={copied === 'user'} onclick={copyUsername} aria-label={$t('ctx_action_copy_username')}>
    <Icon name={copied === 'user' ? 'check' : 'user'} size={18} />
  </button>
{/if}
<button type="button" class="chev" class:ok={copied === 'pass'} onclick={copyPassword} aria-label={$t('ctx_action_copy_password')}>
  <Icon name={copied === 'pass' ? 'check' : appLock.locked ? 'lock' : 'copy'} size={18} />
</button>
<VaultUnlock bind:open={unlockOpen} onunlocked={() => { if (!pendingCopy) return; pendingCopy = false; void copyPassword(); }} />

<style>
  .chev {
    display: inline-flex; align-items: center; justify-content: center;
    width: 40px; height: 44px; border: 0; background: transparent; color: var(--text-3);
  }
  .chev.ok { color: var(--success-text); }
</style>

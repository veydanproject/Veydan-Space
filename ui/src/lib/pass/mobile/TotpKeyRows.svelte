<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The key of a 2FA entry in the phone's «⋮» sheets (design 16), the computer's
  TotpSecretField as rows: «Показать» (the key in groups of four until it hides
  itself), «Копировать», «Сменить ключ» (`onchange`, after the unlock). A locked
  app asks for the unlock first; the shown key lives only while it is on screen.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { formatError, hasErrorCode } from '$lib/core/utils';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { secretCopied } from '$lib/core/ui/toast.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { api } from '$lib/pass/api';
  import { AutoHide } from '$lib/pass/autohide.svelte';
  import { copySecret } from '$lib/pass/clipboard';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';

  interface Props {
    entryId: string;
    /** «Сменить ключ», once the app is unlocked. */
    onchange: () => void;
  }

  let { entryId, onchange }: Props = $props();

  let revealed = $state('');
  const hide = new AutoHide(() => (revealed = ''));
  let failure = $state('');
  let unlockOpen = $state(false);
  let afterUnlock: (() => void) | null = null;
  /** Counts the asks for the key: one answered after the key was hidden or the sheet closed is dropped. */
  let asked = 0;

  /** Four characters to a group, as a service prints a key. */
  const grouped = $derived(revealed.replace(/(.{4})/g, '$1 ').trim());

  function forget() {
    asked++;
    revealed = '';
    hide.stop();
  }

  // On another entry and when the sheet closes.
  $effect(() => {
    void entryId;
    return forget;
  });

  $effect(() => {
    if (appLock.locked) forget();
  });

  function askUnlock(then: () => void) {
    afterUnlock = then;
    unlockOpen = true;
  }

  function onUnlocked() {
    const next = afterUnlock;
    afterUnlock = null;
    next?.();
  }

  async function reveal() {
    failure = '';
    if (revealed) return forget();
    if (appLock.locked) return askUnlock(() => void reveal());
    const ask = ++asked;
    try {
      const { value } = await api.totp.reveal(entryId);
      if (ask !== asked || appLock.locked) return;
      revealed = value;
      hide.start();
    } catch (e) {
      if (ask !== asked) return;
      if (hasErrorCode(e, 'vault_locked')) askUnlock(() => void reveal());
      else failure = formatError(e);
    }
  }

  async function copy() {
    if (appLock.locked) return askUnlock(() => void copy());
    failure = '';
    try {
      await api.totp.copy(entryId);
      secretCopied($t('toast_copied'));
    } catch (e) {
      if (hasErrorCode(e, 'vault_locked')) return askUnlock(() => void copy());
      // No clipboard of the backend (the phone): the WebView's.
      try {
        await copySecret((await api.totp.reveal(entryId)).value, { toast: $t('toast_copied') });
      } catch (err) {
        failure = formatError(err);
      }
    }
  }

  function change() {
    if (appLock.locked) return askUnlock(change);
    forget();
    onchange();
  }
</script>

<div class="section-label key-label">{$t('totp_field_secret')}</div>
{#if failure}<div class="failure"><Notice tone="danger">{failure}</Notice></div>{/if}
<div class="m-list">
  {#if revealed}
    <div class="list-row dense still shown">
      <span class="value mono">{grouped}</span>
      {#if hide.left}<span class="autohide">{$t('pw_autohide_in', { n: String(hide.left) })}</span>{/if}
    </div>
  {/if}
  <button type="button" class="list-row dense" onclick={reveal}>
    <Icon name={revealed ? 'eye-off' : 'eye'} size={20} /><span class="list-row-label">{$t(revealed ? 'pw_btn_hide' : 'pw_btn_reveal')}</span>
  </button>
  <button type="button" class="list-row dense" onclick={copy}>
    <Icon name="copy" size={20} /><span class="list-row-label">{$t('common_copy')}</span>
  </button>
  <button type="button" class="list-row dense" onclick={change}>
    <Icon name="key" size={20} /><span class="list-row-label">{$t('totp_key_change')}</span>
  </button>
</div>

<VaultUnlock bind:open={unlockOpen} onunlocked={onUnlocked} />

<style>
  .key-label { margin-top: var(--sp-4); }
  .failure { margin-bottom: var(--sp-2); }
  .shown { flex-direction: column; align-items: flex-start; justify-content: center; gap: 2px; }
  .value { overflow-wrap: anywhere; font-weight: var(--fw-semibold); user-select: text; -webkit-user-select: text; }
  .autohide { font-size: var(--fs-xs); color: var(--text-2); }
</style>

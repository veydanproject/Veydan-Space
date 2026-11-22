<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { AutoHide } from '$lib/pass/autohide.svelte';
  import { copied as copiedToast, toast } from '$lib/core/ui/toast.svelte';
  import { goto } from '$app/navigation';
  import { leaveGuard } from '$lib/pass/mobile/leaveGuard';
  import VaultLockNotice, { vaultWritable } from '$lib/pass/mobile/VaultLockNotice.svelte';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import LabelField from '$lib/pass/mobile/LabelField.svelte';
  import PasswordForm, { fieldError } from '$lib/pass/mobile/PasswordForm.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';
  import { api } from '$lib/pass/api';
  import { copySecret } from '$lib/pass/clipboard';
  import { formatError } from '$lib/core/utils';
  import { t } from '$lib/core/mobile/i18n';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { hasErrorCode } from '$lib/core/utils';
  import { AVATAR_COLORS } from '$lib/core/mobile/nav-colors';
  import type { PasswordEntry, TotpEntry } from '$lib/pass/types';

  const id = $derived(page.params.id ?? '');

  let entry = $state<PasswordEntry | null>(null);
  let missing = $state(false);
  // The form is a step of the history, `?edit` (design 16): the system «Назад» closes it, not the card,
  // and the bottom bar is hidden by the address (entry/mobile.ts). The list's menu opens it at once.
  const editing = $derived(page.url.searchParams.has('edit'));
  /** The step `?edit` was made by the card's «Изменить»: leaving it goes back over it. */
  let stepped = false;
  let title = $state('');
  let username = $state('');
  let url = $state('');
  let password = $state('');
  let note = $state('');
  let noteTouched = $state(false);
  let totpIds = $state<string[]>([]);
  let addTotp = $state(false);
  let totpName = $state('');
  let totpIssuer = $state('');
  let totpSecret = $state('');
  let tags = $state<string[]>([]);
  let revealed = $state('');
  const hide = new AutoHide(() => (revealed = ''));
  $effect(() => () => hide.stop());
  // A locked app keeps no shown password, not even masked in memory.
  $effect(() => {
    if (appLock.locked) { revealed = ''; hide.stop(); }
  });
  let unlockOpen = $state(false);
  let afterUnlock = $state<'reveal' | 'copy' | 'edit' | 'primary' | null>(null);
  let pendingPrimary = $state<string | null>(null);
  let shownNote = $state('');
  let totp = $state<TotpEntry[]>([]);
  let error = $state('');
  let saving = $state(false);
  let form = $state<PasswordForm>();

  const unlocked = $derived(!appLock.locked);

  const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((v, i) => v === b[i]);
  /** Something typed in the form that a way out would lose. */
  function dirty(): boolean {
    if (!editing || !entry) return false;
    return (
      title !== entry.title ||
      username !== (entry.username ?? '') ||
      url !== (entry.url ?? '') ||
      password !== '' ||
      (noteTouched && note !== shownNote) ||
      !sameList(totpIds, entry.totp_ids) ||
      !sameList(tags, entry.tags) ||
      (addTotp && !!(totpSecret.trim() || totpName.trim() || totpIssuer.trim()))
    );
  }
  const guard = leaveGuard(dirty);

  // Out of the form by any way (a save, «Отмена», the system «Назад»): the fields are the card's again.
  $effect(() => {
    if (!editing) untrack(resetForm);
  });
  const linked = $derived(
    (entry?.totp_ids ?? []).map((tid) => totp.find((item) => item.id === tid)).filter((item): item is TotpEntry => !!item),
  );

  function fail(e: unknown) {
    const key = passwordErrorKey(e);
    error = key ? $t(key) : formatError(e);
  }

  async function load() {
    try {
      const list = await api.passwords.list();
      const found = list.find((item) => item.id === id) ?? null;
      entry = found;
      missing = !found;
      if (found) {
        title = found.title;
        username = found.username ?? '';
        url = found.url ?? '';
        totpIds = [...found.totp_ids];
        tags = [...found.tags];
      }
    } catch (e) {
      fail(e);
    }
  }

  $effect(() => {
    if (!unlocked || !entry?.has_note) {
      if (!unlocked) shownNote = '';
      return;
    }
    const noteId = entry.id;
    let cancelled = false;
    api.passwords.reveal(noteId, 'note').then((result) => {
      if (!cancelled) {
        shownNote = result.value;
        if (!noteTouched) note = result.value;
      }
    }).catch(() => {});
    return () => { cancelled = true; };
  });

  onMount(() => {
    // A locked app opens the form only after the unlock.
    void appLock.refresh().then(() => {
      if (editing && appLock.locked) {
        void goto(`/passwords/${id}`, { replaceState: true, noScroll: true, keepFocus: true });
        askUnlock('edit');
      }
    });
    void appLock.listen();
    void load();
    api.totp.list().then((list) => (totp = list)).catch(() => {});
  });

  function askUnlock(action: 'reveal' | 'copy' | 'edit' | 'primary') {
    afterUnlock = action;
    unlockOpen = true;
  }

  function onUnlocked() {
    const action = afterUnlock;
    afterUnlock = null;
    if (action === 'reveal') void reveal();
    else if (action === 'copy') void copyPassword();
    else if (action === 'edit') startEdit();
    else if (action === 'primary' && pendingPrimary) {
      const tid = pendingPrimary;
      pendingPrimary = null;
      void makePrimary(tid);
    }
  }

  async function reveal() {
    if (appLock.locked) return askUnlock('reveal');
    error = '';
    try {
      if (revealed) {
        revealed = '';
        hide.stop();
      } else {
        revealed = (await api.passwords.reveal(id, 'password')).value;
        hide.start();
      }
    } catch (e) {
      if (hasErrorCode(e, 'vault_locked') || hasErrorCode(e, 'vault_mismatch') || hasErrorCode(e, 'decrypt_failed')) {
        return askUnlock('reveal');
      }
      fail(e);
    }
  }

  async function copyPassword() {
    if (appLock.locked) return askUnlock('copy');
    error = '';
    try {
      const value = (await api.passwords.reveal(id, 'password')).value;
      revealed = value;
      hide.start();
      await copySecret(value, { toast: $t('toast_copied') });
    } catch (e) {
      if (hasErrorCode(e, 'vault_locked') || hasErrorCode(e, 'vault_mismatch') || hasErrorCode(e, 'decrypt_failed')) {
        return askUnlock('copy');
      }
      fail(e);
    }
  }

  async function copyField(value: string) {
    try {
      await writeClipboard(value);
      copiedToast();
    } catch (e) {
      fail(e);
    }
  }

  async function writeClipboard(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      const el = document.createElement('textarea');
      el.value = value;
      el.setAttribute('readonly', '');
      el.style.position = 'fixed';
      el.style.left = '-9999px';
      document.body.appendChild(el);
      el.select();
      const ok = document.execCommand('copy');
      el.remove();
      if (!ok) throw new Error('copy');
    }
  }

  async function save() {
    if (!title.trim()) {
      error = $t('pw_err_title');
      return form?.revealError();
    }
    saving = true;
    error = '';
    try {
      const next = [...totpIds];
      // Created before the password; removed again if the password save fails.
      let newTotpId: string | null = null;
      if (addTotp && totpSecret.trim()) {
        const code = await api.totp.add({
          name: (totpName || title).trim(),
          issuer: totpIssuer.trim() || null,
          secret: totpSecret.trim(),
          tags,
        });
        newTotpId = code.id;
        next.push(code.id);
      }
      try {
        entry = await api.passwords.update(id, {
          title: title.trim(),
          username,
          url,
          password: password || null,
          note: noteTouched || note ? note : null,
          clear_note: noteTouched && !note.trim(),
          totp_ids: next,
          tags,
        });
      } catch (e) {
        if (newTotpId) await api.totp.delete(newTotpId).catch(() => {});
        throw e;
      } finally {
        if (newTotpId) totp = await api.totp.list();
      }
      await passwordStore.refresh();
      await passwordStore.syncNoteBindings(id, tags);
      totpIds = next;
      addTotp = false;
      totpSecret = '';
      shownNote = note;
      password = '';
      guard.pass();
      leaveEdit();
      toast.success($t('common_saved'));
    } catch (e) {
      fail(e);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!entry || !(await ask({ title: $t('pw_delete_confirm', { name: entry.title }) }))) return;
    try {
      await api.passwords.delete(id);
      goto('/passwords', { replaceState: true });
    } catch (e) {
      fail(e);
    }
  }

  /** «Изменить»: the form as a step of the history, over the card. */
  function startEdit() {
    if (appLock.locked) return askUnlock('edit');
    stepped = true;
    void goto(`/passwords/${id}?edit`, { noScroll: true, keepFocus: true });
  }

  /** Back to the card: over the step «Изменить» made, or in place of the list menu's `?edit`. */
  function leaveEdit() {
    if (!page.url.searchParams.has('edit')) return;
    if (stepped) history.back();
    else void goto(`/passwords/${id}`, { replaceState: true, noScroll: true, keepFocus: true });
    stepped = false;
  }

  /** The arrow and «Отмена» of the form: changes are kept unless the question is answered «Отменить». */
  async function closeEdit() {
    if (await guard.mayLeave()) leaveEdit();
  }

  function resetForm() {
    stepped = false;
    addTotp = false;
    error = '';
    password = '';
    if (!entry) return;
    title = entry.title;
    username = entry.username ?? '';
    url = entry.url ?? '';
    totpIds = [...entry.totp_ids];
    tags = [...entry.tags];
    noteTouched = false;
    note = shownNote;
  }

  function avatarColor(s: string): string {
    let h = 0;
    for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return AVATAR_COLORS[h % AVATAR_COLORS.length];
  }

  /** Move a code to the front; the first one shows in the password list. */
  async function makePrimary(totpId: string) {
    if (!entry) return;
    if (appLock.locked) {
      pendingPrimary = totpId;
      askUnlock('primary');
      return;
    }
    const next = [totpId, ...entry.totp_ids.filter((tid) => tid !== totpId)];
    try {
      entry = await api.passwords.update(id, { totp_ids: next });
      totpIds = next;
      await passwordStore.refresh();
    } catch (e) {
      fail(e);
    }
  }

</script>

<div class="m-page">
  <div class="m-header">
    {#if editing}
      <button type="button" class="icon-btn" onclick={closeEdit} aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></button>
    {:else}
      <a class="icon-btn" href="/passwords" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    {/if}
    <!-- The head is the way back and the name only: «Удалить | Изменить» are the card's foot (design 16). -->
    <h1 class="m-title">{editing ? $t('pw_edit_title') : (entry?.title ?? $t('pw_title'))}</h1>
  </div>
  <div class="m-body">
    {#if missing}
      <div class="empty-state"><span class="empty-icon" aria-hidden="true"><Icon name="search" size={22} /></span><p>{$t('totp_not_found')}</p></div>
    {:else if entry && editing}
      <PasswordForm
        bind:this={form}
        editing
        bind:title
        bind:username
        bind:url
        bind:password
        bind:note
        bind:noteTouched
        bind:totpIds
        bind:addTotp
        bind:totpName
        bind:totpIssuer
        bind:totpSecret
        bind:tags
        bind:error
        {totp}
      />
    {:else if entry}
      <VaultLockNotice />
      {#if error}<div class="err"><Notice tone="danger">{error}</Notice></div>{/if}
      <!-- The fields as one grouped card, like every list of the phone; an empty field is left out. -->
      <div class="m-list fields">
        {#if entry.username}
          <div class="list-row dense still field">
            <span class="field-text">
              <span class="form-label">{$t('pw_field_username')}</span>
              <span class="val">{entry.username}</span>
            </span>
            <button type="button" class="icon-btn" onclick={() => copyField(entry?.username ?? '')} aria-label={$t('common_copy')}>
              <Icon name="id" size={22} />
            </button>
          </div>
        {/if}
        {#if entry.url}
          <div class="list-row dense still field">
            <span class="field-text">
              <span class="form-label">{$t('pw_field_url')}</span>
              <span class="val">{entry.url}</span>
            </span>
            <button type="button" class="icon-btn" onclick={() => copyField(entry?.url ?? '')} aria-label={$t('common_copy')}>
              <Icon name="copy" size={22} />
            </button>
          </div>
        {/if}
        <div class="list-row dense still field">
          <span class="field-text">
            <span class="form-label">{$t('pw_field_password')}</span>
            <span class="val mono">{unlocked && revealed ? revealed : '••••••••'}</span>
            {#if unlocked && revealed && hide.left}<span class="autohide">{$t('pw_autohide_in', { n: String(hide.left) })}</span>{/if}
          </span>
          <button type="button" class="icon-btn" onclick={reveal} aria-label={$t(revealed ? 'pw_btn_hide' : 'pw_btn_reveal')}>
            <Icon name={revealed ? 'eye-off' : 'eye'} size={22} />
          </button>
          <button type="button" class="icon-btn" onclick={copyPassword} aria-label={$t('common_copy')}>
            <Icon name="lock-password" size={22} />
          </button>
        </div>
        {#if shownNote}
          <div class="list-row dense still field note">
            <span class="field-text">
              <span class="form-label">{$t('pw_field_note')}</span>
              <span class="note-text">{shownNote}</span>
            </span>
          </div>
        {/if}
      </div>
      {#each linked as item, i (item.id)}
        {@const totpLabel = item.issuer || item.name}
        <div class="m-card totp-card">
          <span class="m-avatar" style:--c={avatarColor(totpLabel)}>{totpLabel.charAt(0).toUpperCase()}</span>
          <span class="totp-meta">
            <span class="totp-name">{totpLabel}</span>
            {#if item.issuer}<span class="totp-account">{item.name}</span>{/if}
          </span>
          {#if linked.length > 1}
            {#if i === 0}
              <span class="primary" aria-label={$t('pw_totp_primary')}><Icon name="pin" size={16} /></span>
            {:else}
              <button type="button" class="icon-btn" onclick={() => makePrimary(item.id)} aria-label={$t('pw_totp_make_primary')}>
                <Icon name="arrow-up" size={22} />
              </button>
            {/if}
          {/if}
          <TotpLiveCode entryId={item.id} period={item.period} copiedLabel={$t('totp_copy')} compact />
        </div>
      {/each}
      {#if tags.length}
        <!-- The card only shows its labels; one is taken off in the edit form. -->
        <LabelField readonly {tags} onchange={() => {}} />
      {/if}
    {/if}
  </div>
  {#if entry && editing}
    <!-- «Отмена | Сохранить» in the page's foot, over the keyboard; the error that is not a field's over them. -->
    <div class="page-foot">
      {#if error && !fieldError(error, $t)}<Notice tone="danger">{error}</Notice>{/if}
      <button type="button" class="btn btn-ghost" onclick={closeEdit} disabled={saving}>{$t('common_cancel')}</button>
      <button type="button" class="btn btn-primary" onclick={save} disabled={saving}>
        <Icon name={saving ? 'loader' : 'check'} size={18} class={saving ? 'spin' : ''} />
        {saving ? $t('notes_saving') : $t('common_save')}
      </button>
    </div>
  {:else if entry && appLock.ready && vaultWritable()}
    <!-- The card's foot: the red «Удалить» (it asks), the main «Изменить» last; a vault that takes no change has none. -->
    <div class="page-foot">
      <button type="button" class="btn btn-danger-soft" onclick={remove}><Icon name="trash-2" size={18} />{$t('common_delete')}</button>
      <button type="button" class="btn btn-primary" onclick={startEdit}><Icon name="pencil" size={18} />{$t('common_edit')}</button>
    </div>
  {/if}
</div>
<VaultUnlock bind:open={unlockOpen} onunlocked={onUnlocked} />

<style>
  .err { margin-bottom: var(--sp-3); }
  .fields { margin-bottom: var(--sp-3); }
  .field { gap: var(--sp-1); padding-right: var(--sp-2); }
  .field-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding: var(--sp-1) 0; }
  .val { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--fw-semibold); user-select: text; -webkit-user-select: text; }
  .note-text { white-space: pre-wrap; user-select: text; -webkit-user-select: text; }
  .totp-card {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 64px;
    margin-bottom: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
  }
  .totp-meta { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .totp-name { font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .totp-account { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .primary { display: inline-flex; color: var(--accent-text); }
  .autohide { font-size: var(--fs-xs); color: var(--text-2); }
</style>

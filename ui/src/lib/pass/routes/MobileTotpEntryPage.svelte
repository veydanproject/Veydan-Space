<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A 2FA entry on the phone, opened by a tap on its row (design 16): its form
  with «Отмена | Сохранить» in the foot and no bottom bar; a way out with changes
  asks first. «⋮» of the head: copy the code, delete, and the key — show, copy,
  change (`?key`, the list's «Сменить ключ», opens the new key at once).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { leave } from '$lib/core/ui/back';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import LabelField from '$lib/pass/mobile/LabelField.svelte';
  import TotpKeyChange from '$lib/pass/mobile/TotpKeyChange.svelte';
  import TotpKeyRows from '$lib/pass/mobile/TotpKeyRows.svelte';
  import TotpNoteLinks from '$lib/pass/components/TotpNoteLinks.svelte';
  import type { KeyChange } from '$lib/pass/components/TotpSecretField.svelte';
  import { leaveGuard } from '$lib/pass/mobile/leaveGuard';
  import { api } from '$lib/pass/api';
  import { copySecret } from '$lib/pass/clipboard';
  import { formatError } from '$lib/core/utils';
  import { directory } from '$lib/core/directory';
  import { t } from '$lib/core/mobile/i18n';
  import { notesLinkedToTotp, syncTotpNotes } from '$lib/pass/totp-notes';

  const id = $derived(page.params.id ?? '');

  let name = $state('');
  let issuer = $state('');
  let tags = $state<string[]>([]);
  let notes = $state<{ id: string; title: string }[]>([]);
  let noteIds = $state<string[]>([]);
  let noteSeed = $state<string[]>([]);
  /** The entry as it was read: what the form is compared with. */
  let saved = $state({ name: '', issuer: '', tags: [] as string[] });
  let change = $state<KeyChange | null>(null);
  let keyError = $state('');
  let menu = $state(false);
  let ready = $state(false);
  let missing = $state(false);
  let error = $state('');
  let saving = $state(false);
  // Before the form is there the error is the page's (a plate); in the form, a line under the name or above the buttons.
  const nameErr = $derived(!!error && error === $t('totp_error_name'));

  const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((v, i) => v === b[i]);
  const sameSet = (a: string[], b: string[]) => a.length === b.length && a.every((v) => b.includes(v));
  const guard = leaveGuard(
    () =>
      ready &&
      (name !== saved.name ||
        issuer !== saved.issuer ||
        !sameList(tags, saved.tags) ||
        !sameSet(noteIds, noteSeed) ||
        (!!change && !!(change.secret.trim() || change.uri))),
  );

  onMount(async () => {
    try {
      const entries = await api.totp.list();
      const entry = entries.find((item) => item.id === id);
      if (!entry) {
        missing = true;
        return;
      }
      name = entry.name;
      issuer = entry.issuer ?? '';
      tags = [...entry.tags];
      saved = { name, issuer, tags: [...entry.tags] };
      ready = true;
      if (page.url.searchParams.has('key')) startChange();
      try {
        // Notes to link come through the catalog; a product without them offers none.
        await directory.get('note')?.ensureLoaded();
        notes = directory.list('note').map((n) => ({ id: n.id, title: n.name }));
        noteIds = notesLinkedToTotp(id);
        noteSeed = [...noteIds];
      } catch {
        /* note picker stays empty; the account can still be saved */
      }
    } catch (err) {
      error = formatError(err);
    }
  });

  function startChange() {
    menu = false;
    keyError = '';
    change = { tab: 'manual', secret: '', uri: '' };
  }

  /** The key a save sends: none unless «Сменить ключ» is open; an empty one is an error under its field. */
  function newKey(): { secret?: string; uri?: string } | null {
    if (!change) return {};
    if (change.tab === 'qr') return change.uri ? { uri: change.uri } : null;
    return change.secret.trim() ? { secret: change.secret.trim() } : null;
  }

  /** The backend's word for a key that makes no codes (`normalize_secret`, `build_totp`, a QR's link). */
  function badKey(err: unknown): boolean {
    const message = err && typeof err === 'object' && 'message' in err ? String((err as { message: unknown }).message) : '';
    return /^(Invalid secret|Could not decode|Invalid URI)/.test(message);
  }

  async function save() {
    error = '';
    keyError = '';
    if (!name.trim()) {
      error = $t('totp_error_name');
      return;
    }
    const key = newKey();
    if (!key) {
      keyError = $t('totp_error_secret');
      return;
    }
    saving = true;
    try {
      await api.totp.update(id, {
        name: name.trim(),
        issuer: issuer.trim() || null,
        tags,
        ...key,
      });
      await syncTotpNotes(id, noteIds, noteSeed);
      guard.pass();
      void leave('/totp');
    } catch (err) {
      if (change && badKey(err)) keyError = $t('totp_error_invalid');
      else error = formatError(err);
    } finally {
      saving = false;
    }
  }

  /** The arrow and «Отмена»: back to the list once the question allows it. */
  async function cancel() {
    if (await guard.mayLeave()) void leave('/totp');
  }

  async function copyCode() {
    menu = false;
    try {
      await copySecret((await api.totp.generateCode(id)).code);
    } catch (err) {
      error = formatError(err);
    }
  }

  async function remove() {
    menu = false;
    if (!(await ask({ title: $t('totp_delete_confirm', { name: saved.name }) }))) return;
    try {
      await api.totp.delete(id);
      guard.pass();
      void leave('/totp');
    } catch (err) {
      error = formatError(err);
    }
  }
</script>

<div class="m-page">
  <div class="m-header">
    <button type="button" class="icon-btn" onclick={cancel} aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </button>
    <h1 class="m-title">{$t('totp_edit_title')}</h1>
    {#if ready && !missing}
      <button type="button" class="icon-btn" onclick={() => (menu = true)} aria-label={$t('nav_more')}><Icon name="more-vertical" size={22} /></button>
    {/if}
  </div>

  <div class="m-body">
    {#if error && !ready}
      <div class="err"><Notice tone="danger">{error}</Notice></div>
    {/if}

    {#if missing}
      <div class="empty-state">
        <span class="empty-icon" aria-hidden="true"><Icon name="search" size={22} /></span>
        <p>{$t('totp_not_found')}</p>
      </div>
    {:else if ready}
      <div class="m-field">
        <label for="name">{$t('totp_field_name')}</label>
        <input id="name" bind:value={name} placeholder={$t('totp_field_name_placeholder')} autocomplete="off" aria-invalid={nameErr} />
        {#if nameErr}<div class="m-error">{error}</div>{/if}
      </div>
      <div class="m-field">
        <label for="issuer">{$t('totp_field_issuer')}</label>
        <input id="issuer" bind:value={issuer} placeholder={$t('totp_field_issuer_placeholder')} autocomplete="off" />
      </div>

      {#if change}
        <TotpKeyChange bind:change error={keyError} onkeep={() => { change = null; keyError = ''; }} />
      {/if}

      <LabelField {tags} onchange={(next) => (tags = next)} />
      {#if directory.owned('note')}
        <TotpNoteLinks mobile {notes} selected={noteIds} onchange={(ids) => (noteIds = ids)} />
      {/if}
    {:else if !error}
      <p class="wait">{$t('loading')}</p>
    {/if}
  </div>
  {#if ready && !missing}
    <!-- «Отмена | Сохранить» in the page's foot, over the keyboard; the error that is not a field's over them. -->
    <div class="page-foot">
      {#if error && !nameErr}<Notice tone="danger">{error}</Notice>{/if}
      <button type="button" class="btn btn-ghost" onclick={cancel} disabled={saving}>{$t('common_cancel')}</button>
      <button type="button" class="btn btn-primary" onclick={save} disabled={saving}>
        <Icon name="check" size={18} />
        {$t('common_save')}
      </button>
    </div>
  {/if}
</div>

<BottomSheet open={menu} title={saved.issuer || saved.name} onclose={() => (menu = false)}>
  <div class="m-list">
    <button type="button" class="list-row dense" onclick={copyCode}>
      <Icon name="copy" size={20} /><span class="list-row-label">{$t('common_copy')}</span>
    </button>
    <button type="button" class="list-row dense" onclick={remove}>
      <Icon name="trash-2" size={20} /><span class="list-row-label danger">{$t('common_delete')}</span>
    </button>
  </div>
  {#if menu}<TotpKeyRows entryId={id} onchange={startChange} />{/if}
</BottomSheet>

<style>
  .err { margin-bottom: var(--sp-3); }
  .wait { color: var(--text-2); font-size: var(--fs-sm); }
  .danger { color: var(--danger-text); }
</style>

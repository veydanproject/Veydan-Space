<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { goto } from '$app/navigation';
  import { leave } from '$lib/core/ui/back';
  import { leaveGuard } from '$lib/pass/mobile/leaveGuard';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import PasswordForm, { fieldError } from '$lib/pass/mobile/PasswordForm.svelte';
  import { api } from '$lib/pass/api';
  import { formatError } from '$lib/core/utils';
  import { t } from '$lib/core/mobile/i18n';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import type { TotpEntry } from '$lib/pass/types';
  import { toast } from '$lib/core/ui/toast.svelte';

  let title = $state('');
  let username = $state('');
  let url = $state('');
  let password = $state('');
  let note = $state('');
  let totpIds = $state<string[]>([]);
  let addTotp = $state(false);
  let totpName = $state('');
  let totpIssuer = $state('');
  let totpSecret = $state('');
  let tags = $state<string[]>([]);
  let totp = $state<TotpEntry[]>([]);
  let error = $state('');
  let saving = $state(false);
  let form = $state<PasswordForm>();

  /** Anything typed: the way out asks before it is lost (design 16). */
  const guard = leaveGuard(
    () =>
      !!(title.trim() || username.trim() || url.trim() || password || note.trim()) ||
      totpIds.length > 0 ||
      tags.length > 0 ||
      (addTotp && !!(totpSecret.trim() || totpName.trim() || totpIssuer.trim())),
  );

  /** The arrow and «Отмена»: back to the list once the question allows it. */
  async function cancel() {
    if (await guard.mayLeave()) void leave('/passwords');
  }

  $effect(() => {
    api.totp.list().then((list) => (totp = list)).catch(() => {});
  });

  async function save() {
    error = '';
    if (!title.trim()) error = $t('pw_err_title');
    else if (!password) error = $t('pw_err_password');
    if (error) return form?.revealError();
    saving = true;
    try {
      const linked = [...totpIds];
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
        linked.push(code.id);
      }
      const created = await api.passwords.create({
        title: title.trim(),
        username: username || null,
        url: url || null,
        password,
        note: note || null,
        totp_ids: linked,
        tags,
      }).catch(async (e) => {
        if (newTotpId) await api.totp.delete(newTotpId).catch(() => {});
        throw e;
      });
      await passwordStore.refresh();
      await passwordStore.syncNoteBindings(created.id, tags);
      guard.pass();
      goto(`/passwords/${created.id}`, { replaceState: true });
      toast.success($t('common_saved'));
    } catch (e) {
      const key = passwordErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="m-page">
  <div class="m-header">
    <button type="button" class="icon-btn" onclick={cancel} aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></button>
    <h1 class="m-title">{$t('cmd_passwords_create')}</h1>
  </div>
  <div class="m-body">
    <PasswordForm
      bind:this={form}
      bind:title
      bind:username
      bind:url
      bind:password
      bind:note
      bind:totpIds
      bind:addTotp
      bind:totpName
      bind:totpIssuer
      bind:totpSecret
      bind:tags
      bind:error
      {totp}
    />
  </div>
  <!-- «Отмена | Сохранить» in the page's foot, over the keyboard; the error that is not a field's over them. -->
  <div class="page-foot">
    {#if error && !fieldError(error, $t)}<Notice tone="danger">{error}</Notice>{/if}
    <button type="button" class="btn btn-ghost" onclick={cancel} disabled={saving}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-primary" onclick={save} disabled={saving}>
      <Icon name={saving ? 'loader' : 'check'} size={18} class={saving ? 'spin' : ''} />
      {saving ? $t('notes_saving') : $t('common_save')}
    </button>
  </div>
</div>

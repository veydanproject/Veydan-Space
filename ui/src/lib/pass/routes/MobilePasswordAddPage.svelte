<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import PasswordForm from '$lib/pass/mobile/PasswordForm.svelte';
  import { api } from '$lib/pass/api';
  import { formatError } from '$lib/core/utils';
  import { t } from '$lib/core/mobile/i18n';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import type { TotpEntry } from '$lib/pass/types';

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

  $effect(() => {
    api.totp.list().then((list) => (totp = list)).catch(() => {});
  });

  async function save() {
    error = '';
    if (!title.trim()) return (error = $t('pw_err_title'));
    if (!password) return (error = $t('pw_err_password'));
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
      goto(`/passwords/${created.id}`, { replaceState: true });
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
    <a class="m-ibtn" href="/passwords" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    <h1 class="m-title">{$t('cmd_passwords_create')}</h1>
  </div>
  <div class="m-body">
    <PasswordForm
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
      {saving}
      onsave={save}
    />
  </div>
</div>

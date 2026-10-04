<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's form of a password, one body for New and Edit: the fields, the
  password with reveal and generate inside it, the linked TOTP codes, the
  labels, and one full-width Save, as on the TOTP form.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { generatePassword, pwSettings } from '$lib/pass/password-gen';
  import LabelField from '$lib/pass/mobile/LabelField.svelte';
  import PasswordTotpField from '$lib/pass/mobile/PasswordTotpField.svelte';
  import type { TotpEntry } from '$lib/pass/types';

  interface Props {
    /** Editing a saved password: an empty password field keeps the stored one. */
    editing?: boolean;
    title: string;
    username: string;
    url: string;
    password: string;
    note: string;
    /** Set once the note was typed in (an emptied note is then cleared). */
    noteTouched?: boolean;
    totpIds: string[];
    addTotp: boolean;
    totpName: string;
    totpIssuer: string;
    totpSecret: string;
    tags: string[];
    /** Every TOTP entry, to pick from. */
    totp: TotpEntry[];
    error?: string;
    saving?: boolean;
    onsave: () => void;
  }

  let {
    editing = false,
    title = $bindable(),
    username = $bindable(),
    url = $bindable(),
    password = $bindable(),
    note = $bindable(),
    noteTouched = $bindable(false),
    totpIds = $bindable(),
    addTotp = $bindable(),
    totpName = $bindable(),
    totpIssuer = $bindable(),
    totpSecret = $bindable(),
    tags = $bindable(),
    totp,
    error = $bindable(''),
    saving = false,
    onsave,
  }: Props = $props();

  let show = $state(false);

  function generate() {
    try {
      password = generatePassword(get(pwSettings));
      show = true;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      error = msg === 'pwgen_error_no_charset' ? $t('pwgen_error_no_charset') : msg;
    }
  }
</script>

<!-- Not a <form>: the sheets of the TOTP and label fields render inside it, and Enter there must not save. -->
<div class="form">
  {#if error}<div class="m-error">{error}</div>{/if}
  <div class="m-field"><label for="pw-title">{$t('pw_field_title')}</label><input id="pw-title" bind:value={title} autocomplete="off" /></div>
  <div class="m-field"><label for="pw-user">{$t('pw_field_username')}</label><input id="pw-user" bind:value={username} autocomplete="off" /></div>
  <div class="m-field"><label for="pw-url">{$t('pw_field_url')}</label><input id="pw-url" type="url" bind:value={url} autocomplete="off" /></div>
  <div class="m-field">
    <label for="pw-secret">{$t('pw_field_password')}</label>
    <div class="secret">
      <input
        id="pw-secret"
        class="mono"
        type={show ? 'text' : 'password'}
        bind:value={password}
        autocomplete="new-password"
        placeholder={editing ? $t('pw_unchanged') : ''}
      />
      <button type="button" class="m-ibtn" onclick={() => (show = !show)} aria-label={$t(show ? 'pw_btn_hide' : 'pw_btn_reveal')}>
        <Icon name={show ? 'eye-off' : 'eye'} size={20} />
      </button>
      <button type="button" class="m-ibtn" onclick={generate} aria-label={$t('pw_btn_generate')}>
        <Icon name="dices" size={20} />
      </button>
    </div>
  </div>
  <div class="m-field">
    <label for="pw-note">{$t('pw_field_note')}</label>
    <textarea id="pw-note" rows="3" bind:value={note} oninput={() => (noteTouched = true)}></textarea>
  </div>
  <PasswordTotpField
    entries={totp}
    bind:totpIds
    bind:creating={addTotp}
    bind:totpName
    bind:totpIssuer
    bind:totpSecret
    suggestName={title}
  />
  <LabelField linkNotes {tags} onchange={(next) => (tags = next)} />
  <button type="button" class="m-btn-grad save" onclick={onsave} disabled={saving}>
    <Icon name="check" size={18} />
    {$t('pw_btn_save')}
  </button>
</div>

<style>
  .form { display: flex; flex-direction: column; padding-bottom: var(--sp-6); }
  .secret { position: relative; display: flex; align-items: center; }
  /* Room for the two buttons inside the field. */
  .secret input { padding-right: calc(2 * 40px + var(--sp-2)); }
  .secret .m-ibtn { position: absolute; color: var(--text-2); }
  .secret .m-ibtn:last-child { right: var(--sp-1); }
  .secret .m-ibtn:nth-last-child(2) { right: calc(var(--sp-1) + 40px); }
  .secret input::placeholder { font-family: var(--font-ui); }
  .secret input { text-overflow: ellipsis; }
  textarea { width: 100%; resize: none; }
  .save { margin-top: var(--sp-3); }
</style>

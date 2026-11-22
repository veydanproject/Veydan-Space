<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's form of a password, one body for New and Edit: the fields, the
  password with reveal and generate inside it, the linked TOTP codes, the
  labels. Save is the page's: it stands in the page's foot (.page-foot), with
  the error that is not a field's over it (`fieldError`).
-->
<script lang="ts" module>
  /** The errors shown under their field; any other stands over the page's Save. */
  export function fieldError(error: string, t: (key: 'pw_err_title' | 'pw_err_password' | 'pwgen_error_no_charset') => string): boolean {
    return error === t('pw_err_title') || error === t('pw_err_password') || error === t('pwgen_error_no_charset');
  }
</script>

<script lang="ts">
  import { tick } from 'svelte';
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
  }: Props = $props();

  let show = $state(false);
  let titleEl = $state<HTMLInputElement>();
  let secretEl = $state<HTMLInputElement>();
  // One error line for the form: under the field it is about, else over the page's Save.
  const titleErr = $derived(!!error && error === $t('pw_err_title'));
  const passErr = $derived(!!error && error === $t('pw_err_password'));
  const genErr = $derived(!!error && error === $t('pwgen_error_no_charset'));

  /** Save found a field wrong: its field comes into sight (the body scrolls to it) and takes the focus, the error under it. */
  export async function revealError() {
    await tick();
    const el = titleErr ? titleEl : passErr ? secretEl : undefined;
    const field = el?.closest<HTMLElement>('.m-field');
    const body = el?.closest<HTMLElement>('.m-body');
    if (!el || !field) return;
    if (body) {
      const at = field.getBoundingClientRect();
      const view = body.getBoundingClientRect();
      if (at.top < view.top || at.bottom > view.bottom) body.scrollTop += at.top - view.top - 16;
    }
    el.focus({ preventScroll: true });
  }

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
  <div class="m-field">
    <label for="pw-title">{$t('pw_field_title')}</label><input id="pw-title" bind:this={titleEl} bind:value={title} autocomplete="off" aria-invalid={titleErr} aria-describedby={titleErr ? 'pw-title-err' : undefined} />
    {#if titleErr}<div class="m-error" id="pw-title-err" role="alert">{error}</div>{/if}
  </div>
  <div class="m-field"><label for="pw-user">{$t('pw_field_username')}</label><input id="pw-user" bind:value={username} autocomplete="off" autocapitalize="off" spellcheck="false" enterkeyhint="next" /></div>
  <div class="m-field"><label for="pw-url">{$t('pw_field_url')}</label><input id="pw-url" type="url" bind:value={url} autocomplete="off" autocapitalize="off" spellcheck="false" enterkeyhint="next" /></div>
  <div class="m-field">
    <label for="pw-secret">{$t('pw_field_password')}</label>
    <div class="secret">
      <input
        id="pw-secret"
        bind:this={secretEl}
        class="mono"
        type={show ? 'text' : 'password'}
        bind:value={password}
        autocomplete="new-password"
        autocapitalize="off"
        spellcheck="false"
        enterkeyhint="next"
        placeholder={editing ? $t('pw_unchanged') : ''}
        aria-invalid={passErr}
        aria-describedby={passErr || genErr ? 'pw-secret-err' : undefined}
      />
      <button type="button" class="icon-btn" onclick={() => (show = !show)} aria-label={$t(show ? 'pw_btn_hide' : 'pw_btn_reveal')}>
        <Icon name={show ? 'eye-off' : 'eye'} size={22} />
      </button>
      <button type="button" class="icon-btn" onclick={generate} aria-label={$t('pw_btn_generate')}>
        <Icon name="dices" size={22} />
      </button>
    </div>
    {#if passErr || genErr}<div class="m-error" id="pw-secret-err" role="alert">{error}</div>{/if}
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
</div>

<style>
  .form { display: flex; flex-direction: column; }
  .secret { position: relative; display: flex; align-items: center; }
  /* Two icon buttons of 40 inside the field, --sp-1 between them (their zones of 44 do not cross); room for them. */
  .secret input { padding-right: calc(2 * 40px + 3 * var(--sp-1)); }
  .secret .icon-btn { position: absolute; }
  .secret .icon-btn:last-child { right: var(--sp-1); }
  .secret .icon-btn:nth-last-child(2) { right: calc(2 * var(--sp-1) + 40px); }
  .secret input::placeholder { font-family: var(--font-ui); }
  .secret input { text-overflow: ellipsis; }
  textarea { width: 100%; resize: none; }
</style>

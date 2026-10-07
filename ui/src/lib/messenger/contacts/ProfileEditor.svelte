<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My profile as a form. The avatar is set at once (it has commands of its
  own); everything else waits for Publish: the public part goes out as my
  kind 0, the phone only to my own devices. A refusal is shown at the
  field it is about.
-->
<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { BIO_MAX_CHARS, profileInputOf, type MessengerProfileInput } from '../api';
  import { messengerStore } from '../store.svelte';
  import AvatarPicker from './AvatarPicker.svelte';
  import BioEditor from './BioEditor.svelte';
  import PhoneField from './PhoneField.svelte';
  import SocialEditor from './SocialEditor.svelte';
  import { codePoints, sameInput, websiteOf } from './profileEdit';
  import { profileDraft } from './profileDraft';
  import { fieldOf, profileCodeOf, profileErrorText, type ProfileField } from './profileErrors';

  interface Props {
    /** The form is done: `saved` when something was published or kept. */
    onclose: (saved: boolean) => void;
    onopenmedia?: () => void;
  }
  let { onclose, onopenmedia }: Props = $props();

  // The form starts from the profile as it is now, or from the form left
  // unpublished a moment ago (profileDraft); the avatar changing under it
  // (its own commands) leaves the typing alone.
  const owner = messengerStore.identity?.pubkey ?? null;
  const draft = profileDraft.take(owner);
  const start = draft?.start ?? profileInputOf(messengerStore.ownProfile);
  let form = $state<MessengerProfileInput>(structuredClone(draft?.form ?? start));
  let about = $state(draft?.about ?? start.about ?? '');
  let phone = $state(draft?.phone ?? '');
  let share = $state(draft?.share ?? true);
  /** The private part as loaded; `null` until it is. */
  let privStart = $state<{ phone: string; share: boolean } | null>(draft?.privStart ?? null);
  /** Closed by Cancel or Publish: nothing is kept. */
  let closing = false;

  let busy = $state(false);
  let formEl = $state<HTMLFormElement | null>(null);
  let errors = $state<Partial<Record<ProfileField, string>>>({});

  const canAct = $derived(!!messengerStore.status?.runtime?.session_active);

  // The phone comes from the runtime; the field fills once, unless typed in already.
  $effect(() => {
    const own = messengerStore.ownPrivate;
    if (!own) { messengerStore.loadOwnPrivate().catch(() => {}); return; }
    if (privStart) return;
    privStart = { phone: own.phone ?? '', share: own.share_phone };
    if (!phone) phone = own.phone ?? '';
    share = own.share_phone;
  });

  const input = $derived<MessengerProfileInput>({ ...form, about });
  const profileDirty = $derived(!sameInput(input, start));
  const privDirty = $derived(!!privStart && (phone.trim() !== privStart.phone || share !== privStart.share));
  const dirty = $derived(profileDirty || privDirty);
  const bioOver = $derived(codePoints(about) > BIO_MAX_CHARS);

  // Left any other way than Cancel or Publish (another chat, the media
  // settings, the back arrow): the typing waits for the next Edit.
  onDestroy(() => {
    if (closing || !owner) return;
    const f = $state.snapshot(form);
    const p = privStart ? { ...privStart } : null;
    const changed = !sameInput({ ...f, about }, start) || (!!p && (phone.trim() !== p.phone || share !== p.share));
    if (changed) profileDraft.keep({ owner, start, form: f, about, phone, share, privStart: p });
  });

  function done(saved: boolean) {
    closing = true;
    profileDraft.clear();
    onclose(saved);
  }

  // A refusal goes once its field is changed.
  function clearOn(field: ProfileField, read: () => unknown) {
    let first = true;
    $effect(() => {
      read();
      if (first) { first = false; return; }
      untrack(() => { if (errors[field]) errors = { ...errors, [field]: undefined }; });
    });
  }
  clearOn('bio', () => about);
  clearOn('website', () => form.website);
  clearOn('socials', () => form.socials.map((s) => s.p + s.h).join());
  clearOn('phone', () => phone);

  function fail(e: unknown) {
    const code = profileCodeOf(e);
    errors = { ...errors, [fieldOf(code)]: profileErrorText(e) };
    // The refusal may be far down a long form (a phone's screen).
    void tick().then(() => formEl?.querySelector('[role="alert"]')?.scrollIntoView({ block: 'center', behavior: 'smooth' }));
  }

  async function publish() {
    if (busy || !dirty || bioOver) return;
    errors = {};
    let website: string | null;
    // A website left as it came (another client may have set one this form
    // would not) goes back unchanged; the runtime has the last word on it.
    if ((form.website ?? '').trim() === (start.website ?? '').trim()) website = start.website?.trim() || null;
    else try { website = websiteOf(form.website); } catch (e) { fail(e); return; }
    busy = true;
    try {
      if (privDirty) {
        try {
          const saved = await messengerStore.saveOwnPrivate(phone.trim() || null, share);
          privStart = { phone: saved.phone ?? '', share: saved.share_phone };
          phone = saved.phone ?? '';
          share = saved.share_phone;
        } catch (e) { fail(e); return; }
      }
      if (profileDirty) {
        try { await messengerStore.saveOwnProfile({ ...input, website }); }
        catch (e) { fail(e); return; }
      }
      done(true);
    } finally { busy = false; }
  }

  async function cancel() {
    if (busy) return;
    if (dirty) {
      const yes = await ask({ title: $t('msg_profile_discard_confirm'), confirmLabel: $t('msg_profile_discard'), cancelLabel: $t('msg_back') });
      if (!yes) return;
    }
    done(false);
  }
</script>

<form class="editor" novalidate bind:this={formEl} onsubmit={(e) => { e.preventDefault(); void publish(); }}>
  <section class="block">
    <AvatarPicker disabled={!canAct} {onopenmedia} />
  </section>

  <section class="block fields">
    <div class="field">
      <label for="own-display-name">{$t('msg_own_display_name')}</label>
      <input id="own-display-name" type="text" bind:value={form.display_name} disabled={busy} maxlength="100" autocomplete="nickname" />
      <p class="hint">{$t('msg_profile_display_name_hint')}</p>
    </div>
    <div class="field">
      <label for="own-name">{$t('msg_own_name')}</label>
      <input id="own-name" type="text" bind:value={form.name} disabled={busy} maxlength="100" spellcheck="false" autocapitalize="off" autocomplete="username" />
      <p class="hint">{$t('msg_profile_name_hint')}</p>
    </div>
  </section>

  <section class="block">
    <div class="field">
      <label for="own-bio">{$t('msg_profile_bio')}</label>
      <BioEditor id="own-bio" bind:value={about} disabled={busy} error={errors.bio} />
    </div>
  </section>

  <section class="block">
    <div class="field">
      <label for="own-website">{$t('msg_profile_website')}</label>
      <input id="own-website" type="url" inputmode="url" bind:value={form.website} disabled={busy} placeholder={$t('msg_profile_website_placeholder')}
        spellcheck="false" autocapitalize="off" aria-invalid={!!errors.website} class:bad={!!errors.website} />
      {#if errors.website}<div class="field-error" role="alert">{errors.website}</div>{/if}
    </div>
    <div class="field">
      <span class="label">{$t('msg_profile_socials')}</span>
      <SocialEditor bind:socials={form.socials} disabled={busy} error={errors.socials} />
    </div>
  </section>

  <section class="block">
    <PhoneField bind:phone bind:share disabled={busy || !privStart} error={errors.phone} />
  </section>

  <section class="block fields">
    <div class="field">
      <label for="own-nip05">NIP-05</label>
      <input id="own-nip05" type="text" bind:value={form.nip05} disabled={busy} placeholder="user@domain" spellcheck="false" autocapitalize="off" />
    </div>
    <div class="field">
      <label for="own-lud16">{$t('msg_profile_lud16')}</label>
      <input id="own-lud16" type="text" bind:value={form.lud16} disabled={busy} placeholder="name@wallet.example" spellcheck="false" autocapitalize="off" />
    </div>
  </section>

  {#if errors.form}<div class="error-msg" role="alert">{errors.form}</div>{/if}
  {#if errors.avatar}<div class="error-msg" role="alert">{errors.avatar}</div>{/if}

  <div class="footer">
    <p class="hint public"><Icon name="globe" size={12} />{$t('msg_profile_public_hint')}</p>
    <div class="actions">
      {#if dirty}<span class="unsaved">{$t('msg_profile_unsaved')}</span>{/if}
      <button type="button" class="btn btn-ghost" disabled={busy} onclick={cancel}>{$t('common_cancel')}</button>
      <button type="submit" class="btn btn-primary" disabled={busy || !dirty || bioOver || !canAct}>
        {#if busy}<span class="spinner"></span>{/if}{$t('msg_own_publish')}
      </button>
    </div>
  </div>
</form>

<style>
  .editor { display: flex; flex-direction: column; gap: var(--sp-4); min-width: 0; }
  .block { display: flex; flex-direction: column; gap: var(--sp-3); min-width: 0; }
  .block + .block { padding-top: var(--sp-4); border-top: 1px solid var(--border); }
  .fields { display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-3); }
  @media (max-width: 560px) { .fields { grid-template-columns: 1fr; } }
  .field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .field > label, .label { font-size: var(--fs-xs); color: var(--text-3); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  .bad { border-color: var(--danger-border); }
  .field-error { font-size: var(--fs-xs); color: var(--danger-text); }
  .footer {
    display: flex; flex-direction: column; gap: var(--sp-3); padding-top: var(--sp-3); border-top: 1px solid var(--border);
  }
  .public { display: flex; gap: 6px; align-items: flex-start; }
  .public :global(svg) { margin-top: 2px; flex-shrink: 0; }
  .actions { display: flex; align-items: center; justify-content: flex-end; gap: var(--sp-2); flex-wrap: wrap; }
  .unsaved { margin-right: auto; font-size: var(--fs-xs); color: var(--warn-text); }
  .actions .spinner { margin-right: 6px; border-color: color-mix(in srgb, #fff 35%, transparent); border-top-color: #fff; }
  @media (pointer: coarse) {
    .actions .btn { flex: 1; }
    .unsaved { flex-basis: 100%; }
  }
</style>

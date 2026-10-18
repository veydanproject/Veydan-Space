<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The language of the first start: the one guessed from the system, in sight
  on every step and a tap away from the list (platform-spec 11.8). No step
  of its own: most people never need it, and who does finds it at once. The
  choice is the same as the picker's of the settings: shown at once, kept
  by this device and, on a computer, saved for the other computers.
-->
<script lang="ts">
  import { api } from '$lib/core/api';
  import Icon from '$lib/core/Icon.svelte';
  import { LANGUAGES, isLocale, locale, t } from '$lib/core/i18n';
  import PickerSheet from '$lib/core/mobile/PickerSheet.svelte';
  import { isMobile } from '$lib/core/platform';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let open = $state(false);
  const current = $derived(LANGUAGES.find((l) => l.code === $locale)?.native ?? $locale);

  function choose(code: string | null | undefined) {
    if (!isLocale(code) || code === $locale) return;
    locale.set(code);
    if (isTauri && !isMobile) api.settings.setLocale(code).catch((e) => console.error('app_locale_set failed', e));
  }
</script>

{#if isMobile}
  <button type="button" class="chip" onclick={() => (open = true)} aria-label={$t('settings_section_language')}>
    <Icon name="language" size={15} />
    <span>{current}</span>
    <Icon name="chevron-down" size={13} />
  </button>
  <PickerSheet
    {open}
    title={$t('settings_section_language')}
    options={LANGUAGES.map((l) => ({ id: l.code, label: l.native }))}
    value={$locale}
    onclose={() => (open = false)}
    onpick={choose}
  />
{:else}
  <label class="desk">
    <span class="label"><Icon name="language" size={14} />{$t('settings_section_language')}</span>
    <CustomSelect options={LANGUAGES.map((l) => ({ value: l.code, label: l.native }))} value={$locale} onchange={choose} />
  </label>
{/if}

<style>
  .chip {
    display: inline-flex; align-items: center; gap: 5px; height: 32px; padding: 0 10px; border-radius: var(--radius-pill);
    border: 1px solid var(--m-card-border, var(--border)); background: var(--m-card, var(--surface-2)); color: var(--text);
    font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-semibold);
  }
  .desk { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { display: inline-flex; align-items: center; gap: 6px; color: var(--text-3); font-size: var(--fs-xs); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My phone: never in the public profile. It stays on my devices and goes
  only into my own contact card, when I choose; the switch is whether a
  card carries it unless I untick it.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    phone: string;
    share: boolean;
    disabled?: boolean;
    /** A refusal of the last save about the phone. */
    error?: string;
  }
  let { phone = $bindable(), share = $bindable(), disabled = false, error = '' }: Props = $props();
</script>

<div class="phone-field" class:invalid={!!error}>
  <div class="head">
    <label for="own-phone">{$t('msg_profile_phone')}</label>
    <span class="private" title={$t('msg_profile_phone_hint')}><Icon name="lock" size={11} />{$t('msg_profile_phone_private')}</span>
  </div>
  <div class="input-row">
    <input id="own-phone" type="tel" inputmode="tel" autocomplete="tel" bind:value={phone} {disabled}
      placeholder={$t('msg_profile_phone_placeholder')} spellcheck="false" aria-invalid={!!error} aria-describedby="own-phone-hint" />
    {#if phone}
      <button type="button" class="icon-btn" {disabled} onclick={() => (phone = '')} title={$t('msg_profile_phone_remove')} aria-label={$t('msg_profile_phone_remove')}>
        <Icon name="x" size={14} />
      </button>
    {/if}
  </div>
  {#if error}<div class="field-error" role="alert">{error}</div>{/if}
  <p class="hint" id="own-phone-hint">{$t('msg_profile_phone_hint')}</p>
  <div class="share">
    <span id="own-phone-share">{$t('msg_profile_phone_share_default')}</span>
    <button type="button" class="toggle" class:on={share} role="switch" aria-checked={share} aria-labelledby="own-phone-share"
      disabled={disabled || !phone.trim()} onclick={() => (share = !share)}></button>
  </div>
</div>

<style>
  .phone-field { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .head { display: flex; align-items: center; gap: var(--sp-2); }
  label { font-size: var(--fs-xs); color: var(--text-3); }
  .private {
    display: inline-flex; align-items: center; gap: 4px; padding: 1px 8px; border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold);
    background: var(--success-bg); color: var(--success-text); border: 1px solid var(--success-border);
  }
  .input-row { display: flex; gap: var(--sp-2); align-items: center; }
  .input-row input { flex: 1; min-width: 0; font-variant-numeric: tabular-nums; }
  .invalid input { border-color: var(--danger-border); }
  .icon-btn { flex-shrink: 0; }
  .field-error { font-size: var(--fs-xs); color: var(--danger-text); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  .share { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); color: var(--text-body); }
  .toggle:disabled { opacity: 0.45; cursor: default; }
  @media (pointer: coarse) { .icon-btn { width: 40px; height: 40px; } }
</style>

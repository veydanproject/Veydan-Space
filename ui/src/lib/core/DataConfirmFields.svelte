<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The confirmation of «Load demo data» and «Clear app data» (platform-spec 5.9,
  8): the secret of the lock when one is set, and, for the clear of a device
  joined to a vault, whether it is this device alone (it leaves the vault first)
  or every device of the vault. Shown inside the dialog of each settings page.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/core/api';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { lockInputAttrs } from '$lib/core/lock/secret';

  interface Props {
    /** The clear of the data; the demo data asks the secret alone. */
    clear: boolean;
    secret: string;
    scope: 'device' | 'all';
    /** The last secret given was not the lock's. */
    wrong?: boolean;
    disabled?: boolean;
  }

  let {
    clear,
    secret = $bindable(''),
    scope = $bindable('device'),
    wrong = false,
    disabled = false,
  }: Props = $props();

  let joined = $state(false);
  const kind = $derived(appLock.status.kind);
  const attrs = $derived(lockInputAttrs(kind));

  onMount(() => {
    void appLock.refresh();
    if (clear) void api.sync.status().then((s) => (joined = s.joined)).catch(() => {});
  });
</script>

{#if clear && joined}
  <div class="scope" role="radiogroup">
    <label class="choice">
      <input type="radio" name="clear-scope" value="device" bind:group={scope} {disabled} />
      <span class="text">
        <span class="title">{$t('settings_clear_scope_device')}</span>
        <span class="sub">{$t('settings_clear_scope_device_hint')}</span>
      </span>
    </label>
    <label class="choice">
      <input type="radio" name="clear-scope" value="all" bind:group={scope} {disabled} />
      <span class="text">
        <span class="title">{$t('settings_clear_scope_all')}</span>
        <span class="sub">{$t('settings_clear_scope_all_hint')}</span>
      </span>
    </label>
  </div>
{/if}

{#if appLock.status.enabled}
  <input
    class="secret"
    type={kind === 'pin' ? 'text' : 'password'}
    inputmode={attrs.inputmode}
    pattern={attrs.pattern}
    bind:value={secret}
    placeholder={$t('settings_lock_current')}
    autocomplete="off"
    autocapitalize="off"
    spellcheck="false"
    {disabled}
  />
  {#if wrong}<span class="err">{$t('lock_wrong')}</span>{/if}
{/if}

<style>
  .scope { display: flex; flex-direction: column; gap: var(--sp-2); margin: var(--sp-2) 0; }
  .choice { display: flex; gap: var(--sp-2); align-items: flex-start; cursor: pointer; }
  .choice input { margin-top: 3px; flex-shrink: 0; }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .title { font-size: var(--fs-sm); color: var(--text); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; }
  .secret {
    width: 100%;
    box-sizing: border-box;
    height: var(--control-h-lg);
    margin-top: var(--sp-2);
    padding: 0 var(--sp-3);
    font: inherit;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    background-color: var(--surface-3);
    color: var(--text-body);
  }
  .err { display: block; margin-top: var(--sp-1); font-size: var(--fs-xs); color: var(--danger-text); }
</style>

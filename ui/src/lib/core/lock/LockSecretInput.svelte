<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The field of the lock's secret a form asks for its guard (the Cloud's link,
  recovery, deletion): its label over a field of 44 with the type of 16, and
  the keyboard of the lock's kind — digits for a PIN. One field for every form,
  as the fields of the lock screen and its settings (lockInputAttrs).
-->
<script lang="ts">
  import { appLock } from '$lib/core/lock/store.svelte';
  import { lockInputAttrs } from '$lib/core/lock/secret';
  import type { LockKind } from '$lib/core/types';

  interface Props {
    value: string;
    label: string;
    id: string;
    /** The kind of the secret asked; the lock's of this device by default. */
    kind?: LockKind;
    disabled?: boolean;
    autofocus?: boolean;
    /** The last secret given was not the lock's. */
    wrong?: boolean;
  }

  let { value = $bindable(''), label, id, kind, disabled = false, autofocus = false, wrong = false }: Props = $props();

  const secretKind = $derived(kind ?? appLock.status.kind);
  const attrs = $derived(lockInputAttrs(secretKind));
</script>

<div class="form-group">
  <label for={id}>{label}</label>
  <!-- svelte-ignore a11y_autofocus -->
  <input
    {id}
    class="lock-secret"
    class:pin={secretKind === 'pin'}
    type={secretKind === 'pin' ? 'text' : 'password'}
    inputmode={attrs.inputmode}
    pattern={attrs.pattern}
    autocomplete={attrs.autocomplete}
    autocapitalize="off"
    spellcheck="false"
    bind:value
    {disabled}
    {autofocus}
    aria-invalid={wrong || undefined}
  />
</div>

<style>
  .lock-secret {
    height: var(--control-h-lg);
    min-height: var(--control-h-lg);
    padding: 0 var(--sp-3);
    font-size: var(--fs-md);
    border-radius: var(--radius-field);
  }
  /* A PIN is a text field for its digit keyboard; its digits are hidden as a password's. */
  .lock-secret.pin { -webkit-text-security: disc; }
</style>

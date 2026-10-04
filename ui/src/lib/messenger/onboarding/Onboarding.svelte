<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import IdentityOnboarding from '../identity/IdentityOnboarding.svelte';
  import ServersStep from './ServersStep.svelte';

  // The key first (with its one-time backup), then whose servers.
  const keyStep = $derived(!messengerStore.identity || !!messengerStore.pendingBackup);
</script>

<div class="onboarding">
  <ol class="steps">
    <li class:active={keyStep} class:done={!keyStep}><span class="num">1</span>{$t('msg_onb_step_key')}</li>
    <li class="sep" aria-hidden="true"></li>
    <li class:active={!keyStep}><span class="num">2</span>{$t('msg_onb_step_servers')}</li>
  </ol>
  {#if keyStep}
    <IdentityOnboarding />
  {:else}
    <div class="card servers">
      <ServersStep />
    </div>
  {/if}
</div>

<style>
  .onboarding { max-width: 640px; display: flex; flex-direction: column; gap: var(--sp-3); }
  .servers { display: flex; flex-direction: column; gap: var(--sp-3); }
  .steps { list-style: none; margin: 0; padding: 0; display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-3); }
  .steps li { display: flex; align-items: center; gap: 6px; }
  .steps li.active { color: var(--text); font-weight: 600; }
  .steps li.done { color: var(--text-2); }
  .num {
    display: inline-flex; align-items: center; justify-content: center;
    width: 20px; height: 20px; border-radius: 50%; font-size: var(--fs-xs);
    border: 1px solid var(--border); background: var(--surface-2);
  }
  .active .num { background: var(--accent-bg); border-color: var(--accent-border); color: var(--accent-text); }
  .sep { flex: 0 0 24px; height: 1px; background: var(--border); }
</style>

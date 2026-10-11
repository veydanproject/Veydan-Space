<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The first start over a new data file (platform-spec 12): the frame of the
  wizard, the same steps on a phone (the whole screen, the progress as
  dashes) and on a computer (a window with the list of steps). The core's
  steps are named here; the modules add theirs through the registry
  (`ModuleDef.onboarding`). The shell mounts it once and tells it where the
  person is, so a step can send them to a screen of the app and have the
  wizard back when they leave it.
-->
<script lang="ts">
  import './onboarding.css';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import type { OnboardingStep } from '$lib/core/module';
  import { isMobile } from '$lib/core/platform';
  import { portal } from '$lib/core/portal';
  import { product } from '$lib/core/product';
  import { registry } from '$lib/core/registry';
  import LangChip from './LangChip.svelte';
  import ModulesStep from './ModulesStep.svelte';
  import SyncStep from './SyncStep.svelte';
  import WelcomeStep from './WelcomeStep.svelte';
  import { MODULES, WELCOME, onboarding } from './store.svelte';

  const core: OnboardingStep[] = [
    { id: WELCOME, title: 'ob_step_welcome', order: 0, component: WelcomeStep },
    {
      id: MODULES,
      title: 'modules_section',
      order: 10,
      component: ModulesStep,
      // A product of one module has none to choose between.
      visible: () => registry.only === undefined && registry.switchable().length > 1,
    },
    { id: 'sync', title: 'settings_sync_section', order: 30, component: SyncStep },
  ];
  onboarding.setCore(core);
  onboarding.begin();
  // A plain browser (`vite dev`) has no shell to say this is the first start: `?onboarding` opens the wizard there, to look at its screens.
  if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window) && page.url.searchParams.has('onboarding')) onboarding.open();

  $effect(() => {
    onboarding.at(page.url.pathname);
  });

  const step = $derived(onboarding.step);
  const steps = $derived(onboarding.steps);
  const counted = $derived(onboarding.counted);
  const position = $derived(onboarding.position);
  const at = $derived(steps.findIndex((s) => s.id === step?.id));
  /** Opened again at this step (the card «Finish setting up»): it is closed here, not gone back from. */
  const atFloor = $derived(step !== undefined && step.id === onboarding.floor);
  /** What a module's step is given: it moves the wizard without importing it. */
  const given = $derived({
    next: () => void onboarding.next(),
    detour: (href: string) => onboarding.detour(href),
    intent: onboarding.intent,
    // The product is the module of this step alone: the step cannot be passed by.
    only: (registry.only?.onboarding ?? []).some((s) => s.id === step?.id),
    onback: (handler: (() => boolean) | null) => onboarding.takeBack(handler),
  });
</script>

{#if onboarding.active && !onboarding.hidden && step}
  {#if isMobile}
    <div class="ob m" role="dialog" aria-modal="true" aria-label={product.name}>
      <div class="ob-top">
        {#if atFloor}
          <button type="button" class="ob-back" onclick={() => onboarding.close()} aria-label={$t('common_close')}>
            <Icon name="x" size={22} />
          </button>
        {:else if at > 0}
          <button type="button" class="ob-back" onclick={() => onboarding.back()} aria-label={$t('ob_back')}>
            <Icon name="chevron-left" size={24} />
          </button>
        {:else}
          <span></span>
        {/if}
        <div class="ob-dots" aria-hidden="true">
          {#if position > 0}
            {#each counted as s, i (s.id)}<i class:on={i < position}></i>{/each}
          {/if}
        </div>
        <LangChip />
      </div>
      <div class="ob-step">
        {#key step.id}<step.component {...given} />{/key}
      </div>
    </div>
  {:else}
    <div class="ob" use:portal role="presentation">
      <div class="ob-win" role="dialog" aria-modal="true" aria-label={product.name}>
        <aside class="ob-side">
          <div class="ob-brand"><img src="/logo.png" alt="" />{product.name}</div>
          <ol class="ob-steps">
            {#each steps as s, i (s.id)}
              <li class:on={i === at} class:done={i < at} aria-current={i === at ? 'step' : undefined}>
                <span class="num">{#if i < at}<Icon name="check" size={12} />{:else}{i + 1}{/if}</span>
                {$t(s.title as TranslationKey)}
              </li>
            {/each}
          </ol>
          <LangChip />
        </aside>
        <section class="ob-step">
          <div class="ob-backrow">
            {#if atFloor}
              <button type="button" class="ob-back text" onclick={() => onboarding.close()}>
                <Icon name="x" size={14} />{$t('common_close')}
              </button>
            {:else if at > 0}
              <button type="button" class="ob-back text" onclick={() => onboarding.back()}>
                <Icon name="chevron-left" size={14} />{$t('ob_back')}
              </button>
            {/if}
          </div>
          {#key step.id}<step.component {...given} />{/key}
        </section>
      </div>
    </div>
  {/if}
{/if}

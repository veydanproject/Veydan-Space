// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The first start of the app over a new data file (platform-spec 12): one
// wizard of a few steps, each of which may be passed by — the welcome with
// the language, the modules to use where the product has a choice, the
// steps the modules add (the account of the messenger), sync, the
// protection. The shell begins it while the flag of the first start is up;
// from then on it lives in memory, so answering the first screens (which
// takes the flag away) does not end it. A step may send the person to a
// screen of the app (the link of a device, the settings of sync): the
// wizard steps aside while they are there and comes back when they leave.

import { goto } from '$app/navigation';
import type { OnboardingStep } from '$lib/core/module';
import { registry } from '$lib/core/registry';
import { modulesStore } from '$lib/core/store/modules.svelte';

/** The step every product begins with. */
export const WELCOME = 'welcome';
/** The step that asks which modules to use. */
export const MODULES = 'modules';
/** Home shows the card «Finish setting up» while this is kept (`open`). */
export const SETUP_CARD_KEY = 'setup_card';

/** The path of an address, without its query and fragment. */
export function pathOf(href: string): string {
  return href.split(/[?#]/)[0] || '/';
}

/** Whether `pathname` is the screen `aside` or one under it. */
export function isUnder(pathname: string, aside: string): boolean {
  return pathname === aside || pathname.startsWith(`${aside}/`);
}

/** The steps to show: the core's and the modules', those visible, in their order. */
export function orderSteps(steps: OnboardingStep[]): OnboardingStep[] {
  return steps.filter((s) => s.visible?.() ?? true).sort((a, b) => a.order - b.order);
}

class Onboarding {
  /** The wizard is on, since the shell found the first start. */
  active = $state(false);
  /** The id of the step on the screen. */
  current = $state(WELCOME);
  /** What a step was asked to open with (the account: `have`). */
  intent = $state<string | null>(null);
  /** The screen the person was sent to; the wizard is out of the way while they are there. */
  aside = $state<string | null>(null);
  /** The person arrived there: until then the wizard stays, so the page under it does not show on the way. */
  private entered = $state(false);
  /**
   * The wizard was opened again at this step (the card «Finish setting up»):
   * it goes on from here to its end, Back stops here, and it can be closed.
   */
  floor = $state<string | null>(null);
  /** The step on the screen has inner screens and takes Back while on one of them. */
  private inner: (() => boolean) | null = null;
  private core = $state<OnboardingStep[]>([]);

  /** The core's own steps, given by the wizard's component: this file imports no component. */
  setCore(steps: OnboardingStep[]) {
    this.core = steps;
  }

  /** The steps of this product now, in their order. */
  get steps(): OnboardingStep[] {
    return orderSteps([...this.core, ...registry.onboarding()]);
  }

  get step(): OnboardingStep | undefined {
    const steps = this.steps;
    return steps.find((s) => s.id === this.current) ?? steps[0];
  }

  /** The steps after the welcome, which the progress counts. */
  get counted(): OnboardingStep[] {
    return this.steps.filter((s) => s.id !== WELCOME);
  }

  /** The place of the step among the counted ones, from 1; 0 on the welcome. */
  get position(): number {
    return this.counted.findIndex((s) => s.id === this.step?.id) + 1;
  }

  has(id: string): boolean {
    return this.steps.some((s) => s.id === id);
  }

  /** Out of the way: the person is on a screen a step sent them to. */
  get hidden(): boolean {
    return this.aside !== null && this.entered;
  }

  /** The shell found the app started; the wizard opens over a new data file. */
  begin() {
    if (this.active || !modulesStore.firstRun) return;
    this.open();
  }

  /** Opens the wizard at its first step. */
  open() {
    this.active = true;
    this.floor = null;
    this.show(WELCOME);
    this.aside = null;
    try {
      localStorage.setItem(SETUP_CARD_KEY, 'open');
    } catch {
      // No storage: the card of Home is not shown, the wizard is.
    }
  }

  /**
   * Opens the wizard again at a step, to set up what was passed by: the
   * steps after it follow as at the first start. False where the product
   * has no such step now (its module is off).
   */
  resume(id: string): boolean {
    if (!this.has(id)) return false;
    this.active = true;
    this.floor = id;
    this.aside = null;
    this.show(id);
    return true;
  }

  /** Closes the wizard where it stands; the page under it stays. */
  close() {
    this.active = false;
    this.floor = null;
    this.aside = null;
    this.intent = null;
    this.inner = null;
  }

  /**
   * The first screens are answered: the flag of the first start goes, and a
   * start of the app no longer opens the wizard. The step of the modules
   * does it with its answer (`modules_choose`); every other way past the
   * welcome does it here.
   */
  private async answered() {
    try {
      await modulesStore.endFirstRun();
    } catch (e) {
      console.error('onboarding: the first start was not ended', e);
    }
  }

  /** A step of more than one screen takes Back (null gives it back); the next step starts without one. */
  takeBack(handler: (() => boolean) | null) {
    this.inner = handler;
  }

  private show(id: string, intent: string | null = null) {
    this.inner = null;
    this.intent = intent;
    this.current = id;
  }

  /** To the step after this one; past the last the wizard is over. */
  async next() {
    const steps = this.steps;
    const at = steps.findIndex((s) => s.id === this.step?.id);
    const following = steps[at + 1];
    if (!following) return this.finish();
    if (following.id !== MODULES) await this.answered();
    this.show(following.id);
  }

  /** One screen back: inside the step when it has inner screens, else to the step before. */
  back() {
    if (this.inner?.()) return;
    // Opened again at this step: there is nothing before it to go back to.
    if (this.step?.id === this.floor) return;
    const steps = this.steps;
    const at = steps.findIndex((s) => s.id === this.step?.id);
    if (at > 0) this.show(steps[at - 1].id);
  }

  /** Straight to a step, past those between («I already have an account» of the welcome). */
  async go(id: string, intent: string | null = null) {
    if (!this.has(id)) return;
    await this.answered();
    this.show(id, intent);
  }

  /** Sends the person to a screen of the app; the wizard comes back when they leave it. */
  detour(href: string) {
    this.aside = pathOf(href);
    this.entered = false;
    void goto(href);
  }

  /** The shell says where the person is. */
  at(pathname: string) {
    if (this.aside === null) return;
    if (isUnder(pathname, this.aside)) this.entered = true;
    else if (this.entered) {
      this.aside = null;
      this.entered = false;
    }
  }

  /** The wizard is over: the app opens at its start. */
  async finish() {
    await this.answered();
    // Opened again from a page of the app: that page stays; the first start ends at the start of the app.
    const resumed = this.floor !== null;
    this.active = false;
    this.floor = null;
    this.aside = null;
    this.intent = null;
    if (!resumed) await goto('/', { replaceState: true }).catch(() => {});
  }
}

export const onboarding = new Onboarding();

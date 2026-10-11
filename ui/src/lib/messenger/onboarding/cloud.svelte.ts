// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Veydan Cloud from the side of the messenger's screens
// (internal/vcloud-plan.md 4.5): after «Veydan servers» the free plan is activated in
// the background while the next screen is read; before a switch to the
// user's own servers, an account is offered for deletion. The module cloud
// answers these commands; a product without it answers none, and the
// screens go on as if there were no cloud.

/** The commands of the module cloud this file calls. */
const STATUS = 'cloud_status';
const ACTIVATE = 'cloud_activate_free';
const ACTIVATION_EVENT = 'cloud://activation';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Where the activation of the free plan is, as the module tells it. */
export type FreePhase = 'idle' | 'asking' | 'solving' | 'activating' | 'done' | 'failed';

/** What the messenger reads of `cloud_status`. */
export interface CloudSeen {
  account: { plan: string } | null;
  kept?: { plan: string } | null;
  servers?: string;
}

/** A command of the module; in a plain browser the answers of `mock`. */
type Invoke = <T>(cmd: string) => Promise<T>;

let mock: Record<string, () => unknown> = {
  [STATUS]: () => ({ account: null, kept: null, servers: 'none' }),
  [ACTIVATE]: () => ({ phase: 'done' }),
};

/** The answers of a plain browser (and of the tests). */
export function mockCloud(answers: Record<string, () => unknown>): void {
  mock = { ...mock, ...answers };
}

const invoke: Invoke = async <T>(cmd: string): Promise<T> => {
  if (isTauri) {
    const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
    return tauriInvoke<T>(cmd);
  }
  const answer = mock[cmd];
  if (!answer) throw { code: 'unknown_command', message: cmd };
  return answer() as T;
};

/** Whether the identity has an account of Veydan Cloud this device knows of. */
export function hasAccount(seen: CloudSeen | null): boolean {
  return !!seen && (!!seen.account || !!seen.kept);
}

class CloudOnboarding {
  /** The module cloud answered: the screen «Your free Veydan Cloud» follows. */
  available = $state(false);
  phase = $state<FreePhase>('idle');
  private unlisten: (() => void) | null = null;

  /**
   * Starts the free plan after «Veydan servers»: the work of its proof goes
   * in the background, without a word on the screen (plan 4.5, cloud-api.md
   * 3.3). A product without the module: nothing happens, nothing is shown.
   */
  async start(): Promise<void> {
    this.phase = 'idle';
    try {
      await this.follow();
      const view = await invoke<{ phase: FreePhase }>(ACTIVATE);
      this.available = true;
      // An event that came first and ended it says more than the answer.
      if (!this.settled()) this.phase = view.phase;
    } catch {
      this.available = false;
      this.stop();
    }
  }

  /** The activation ended, one way or the other. */
  private settled(): boolean {
    return this.phase === 'done' || this.phase === 'failed';
  }

  private async follow(): Promise<void> {
    if (!isTauri || this.unlisten) return;
    const { listen } = await import('@tauri-apps/api/event');
    this.unlisten = await listen<{ phase: FreePhase }>(ACTIVATION_EVENT, (e) => {
      this.phase = e.payload.phase;
      if (this.phase === 'done' || this.phase === 'failed') this.stop();
    });
  }

  /** Stops following the activation (the onboarding is over). */
  stop(): void {
    this.unlisten?.();
    this.unlisten = null;
  }

  /** What the module says of the account; null without the module or when it cannot say. */
  async seen(): Promise<CloudSeen | null> {
    try {
      return await invoke<CloudSeen>(STATUS);
    } catch {
      return null;
    }
  }
}

export const cloudOnboarding = new CloudOnboarding();

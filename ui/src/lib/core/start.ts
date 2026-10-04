// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The first commands of the page, as the root layout sends them: a start
// that is never answered must not leave a blank page. The shell builds the
// window only once its setup is done (platform-spec 19, № 54), so they are
// answered at once; this is what the page does when they still are not.

/** How long the start of the page may take before it gives up on it. */
export const START_TIMEOUT_MS = 20_000;

/** The backend did not answer the start of the page in time. */
export class NoAnswer extends Error {
  constructor(ms: number) {
    super(`the app did not answer within ${ms} ms`);
    this.name = 'NoAnswer';
  }
}

/** `promise`, or a rejection with {@link NoAnswer} once `ms` pass. */
export function answerWithin<T>(promise: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new NoAnswer(ms)), ms);
  });
  return Promise.race([promise, late]).finally(() => clearTimeout(timer));
}

/**
 * Whether this UI was built for the other kind of device than the app runs
 * on: the phone UI in a desktop window, or the desktop UI on a phone. The
 * platform of the UI is a constant of its build (core/platform.ts), so a
 * product's two builds must never trade places; the pages of each go to a
 * folder of their own (platform-spec 11.4), and this is the second barrier:
 * the root layout shows the start-error screen instead of the wrong shell.
 * `hostOs` is `host_info().os`; an empty one (no answer) accuses nothing.
 */
export function wrongPlatform(uiIsMobile: boolean, hostOs: string | undefined): boolean {
  if (!hostOs) return false;
  return uiIsMobile !== isMobileOs(hostOs);
}

export function isMobileOs(os: string): boolean {
  return os === 'android' || os === 'ios';
}

/** Set while the page reloads after a start that failed; cleared by a start that worked. */
const RETRIED_KEY = 'veydan_start_retried';

type Flags = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function session(): Flags | null {
  try {
    return sessionStorage;
  } catch {
    return null;
  }
}

/**
 * After a failed start: reload the page once (`true`, the caller waits for
 * the reload), or `false` when this session already did and the page shows
 * why it cannot start. Without storage the page does not reload: it could
 * not tell the second failure from the first.
 */
export function retryOnce(reload: () => void = () => location.reload(), flags: Flags | null = session()): boolean {
  try {
    if (!flags || flags.getItem(RETRIED_KEY)) return false;
    flags.setItem(RETRIED_KEY, '1');
  } catch {
    return false;
  }
  reload();
  return true;
}

/** The start worked: a later failure in this session may reload once again. */
export function started(flags: Flags | null = session()): void {
  try {
    flags?.removeItem(RETRIED_KEY);
  } catch {
    // Nothing to forget.
  }
}

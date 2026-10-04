// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Why a message was not sent, in the UI's language. The backend keeps its own
 * English text (`transport error: no relay connected`); the Retry button
 * shows the reader a sentence of the dictionary, and the raw text goes to the
 * log (`logSendFailure`).
 */
export function sendFailureKey(reason: string | null | undefined): 'msg_send_failed_offline' | 'msg_send_failed' {
  if (reason && /^transport error\b|\bno relay\b|^offline$/i.test(reason)) return 'msg_send_failed_offline';
  return 'msg_send_failed';
}

const logged = new Set<string>();

/** The backend's reason of a failed message, once per message and reason. */
export function logSendFailure(id: string, reason: string | null | undefined) {
  if (!reason || logged.has(`${id}\n${reason}`)) return;
  logged.add(`${id}\n${reason}`);
  console.warn(`message ${id} was not sent: ${reason}`);
}

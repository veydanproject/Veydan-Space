// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a transfer shows under its file, in words: the first line says what
// it is doing, the second how far it is. And what can be done with it now.

import { countKey, localeTag, type Locale } from '$lib/core/i18n';
import type { MessengerTransferProgress } from '../api';
import type { messengerTranslations } from '../i18n';
import { bytes, duration, eta, percent, rate } from '../shared/format';
import { mediaErrorText, type MediaErrorKey } from './errors';

export type TransferKey = Extract<keyof typeof messengerTranslations.en, `msg_xfer_${string}`> | MediaErrorKey;
export type Translate = (key: TransferKey, vars?: Record<string, string>) => string;

export interface Words {
  t: Translate;
  locale: Locale;
  /** Now, unix ms: the countdown to the next attempt. */
  now: number;
}

export interface TransferLines {
  line1: string;
  line2: string;
}

/** The fields of a transfer its words are made of. */
export type TransferState = Pick<
  MessengerTransferProgress,
  | 'direction' | 'status' | 'stage' | 'done_bytes' | 'total_bytes' | 'chunks_done' | 'chunks_total'
  | 'rate_bps' | 'eta_secs' | 'retry_at_ms' | 'failure_reason'
>;

/** Whole seconds until the next attempt; 0 when it is due or unknown. */
export function secondsLeft(retryAtMs: number | null, now: number): number {
  return retryAtMs ? Math.max(0, Math.ceil((retryAtMs - now) / 1000)) : 0;
}

/** The reasons of a wait that mean the connection is lost. */
const OFFLINE = new Set(['err.network', 'err.timeout']);

/**
 * The two lines of a transfer. "Part i of n" while a part moves names the
 * one in progress; elsewhere the count is of the parts done.
 */
export function transferLines(p: TransferState, w: Words): TransferLines {
  const { t } = w;
  const lang = localeTag(w.locale);
  const i = String(p.chunks_done);
  const n = String(p.chunks_total);
  /** The part in progress, counted from 1. */
  const at = String(Math.min(p.chunks_done + 1, p.chunks_total));
  const parts = p.chunks_total > 0;
  const size = bytes(p.total_bytes, lang);
  const amount = t('msg_xfer_bytes', { done: bytes(p.done_bytes, lang), total: size });
  const moving = [amount, rate(p.rate_bps, lang), eta(p.eta_secs, lang)].filter(Boolean).join(' · ');
  const counted = (key: 'msg_xfer_paused' | 'msg_xfer_uploaded' | 'msg_xfer_downloaded') =>
    t(countKey(key, p.chunks_total, w.locale) as TransferKey, { i, n });
  /** How far it got: in parts when they are known. */
  const sofar = parts ? counted(p.direction === 'up' ? 'msg_xfer_uploaded' : 'msg_xfer_downloaded') : amount;

  switch (p.status) {
    // A photo being made smaller is still queued: its stage tells it.
    case 'queued':
      if (p.stage !== 'preparing') return { line1: t('msg_xfer_queued'), line2: size };
      break;
    case 'paused':
      return { line1: parts ? counted('msg_xfer_paused') : t('msg_xfer_paused_short'), line2: amount };
    case 'waiting_retry': {
      const left = secondsLeft(p.retry_at_ms, w.now);
      const time = duration(left, 'second', lang);
      // Only a lost connection is told as one; a busy or failing server is not.
      const offline = !p.failure_reason || OFFLINE.has(p.failure_reason);
      const line1 = offline
        ? (left > 0 ? t('msg_xfer_waiting', { time }) : t('msg_xfer_waiting_now'))
        : (left > 0 ? t('msg_xfer_retry_in', { time }) : t('msg_xfer_retrying'));
      return { line1, line2: sofar };
    }
    case 'failed':
      return { line1: mediaErrorText(p.failure_reason || 'err.unknown', t), line2: sofar };
    case 'cancelled':
      return { line1: t('msg_xfer_cancelled'), line2: size };
    case 'done':
      return { line1: '', line2: size };
  }

  switch (p.stage) {
    case 'queued':
      return { line1: t('msg_xfer_queued'), line2: size };
    case 'preparing':
      return { line1: t('msg_xfer_preparing'), line2: size };
    case 'checking':
      return {
        line1: t(p.direction === 'up' ? 'msg_xfer_checking' : 'msg_xfer_checking_down'),
        line2: parts ? t('msg_xfer_of', { i, n }) : '',
      };
    case 'uploading':
      return { line1: parts ? t('msg_xfer_uploading', { i: at, n }) : t('msg_xfer_uploading_short'), line2: moving };
    case 'downloading':
      return { line1: parts ? t('msg_xfer_downloading', { i: at, n }) : t('msg_xfer_downloading_short'), line2: moving };
    case 'publishing':
      return { line1: t('msg_xfer_publishing'), line2: size };
    case 'assembling':
      return { line1: t('msg_xfer_assembling', { i, n }), line2: size };
    case 'verifying':
      return { line1: t('msg_xfer_verifying'), line2: size };
  }
  return { line1: '', line2: size };
}

/** A file of a message that is not on this device: its size, and its parts when the message lists them. */
export function remoteLines(size: number, chunks: number | undefined, w: Words): TransferLines {
  const s = bytes(size, localeTag(w.locale));
  if (!chunks) return { line1: s, line2: '' };
  return { line1: w.t(countKey('msg_xfer_parts', chunks, w.locale) as TransferKey, { size: s, n: String(chunks) }), line2: '' };
}

/** The few characters under the ring of a picture: how far, or how long until the next attempt. */
export function shortLine(p: TransferState | null, size: number, w: Words): string {
  const lang = localeTag(w.locale);
  if (!p) return bytes(size, lang);
  if (p.status === 'failed' || p.status === 'done' || p.status === 'cancelled') return '';
  if (p.status === 'waiting_retry') {
    const left = secondsLeft(p.retry_at_ms, w.now);
    return left > 0 ? duration(left, 'second', lang) : '…';
  }
  if (p.status === 'queued' || p.stage === 'queued' || p.stage === 'preparing') return bytes(p.total_bytes || size, lang);
  return `${percent(p.done_bytes, p.total_bytes)}%`;
}

export type TransferAction = 'pause' | 'resume' | 'retry_now' | 'retry' | 'retry_download' | 'download' | 'cancel';

/**
 * What can be done with a transfer, the main action first. `null`: the file
 * is elsewhere and nothing fetches it. Publishing is a moment: nothing then.
 */
export function transferActions(p: Pick<TransferState, 'status' | 'stage' | 'direction'> | null): TransferAction[] {
  if (!p) return ['download'];
  switch (p.status) {
    case 'queued':
    case 'running':
      return p.stage === 'publishing' ? [] : ['pause', 'cancel'];
    case 'paused':
      return ['resume', 'cancel'];
    case 'waiting_retry':
      return ['retry_now', 'cancel'];
    case 'failed':
      // A download that keeps failing can be let go: cancelled, it waits for a Download.
      return p.direction === 'up' ? ['retry', 'cancel'] : ['retry_download', 'cancel'];
    default:
      return [];
  }
}

/** Whether "retry all" takes the transfer up: failed, waiting for its next attempt, or paused by the closing of the app; never a pause the user made. */
export function retriable(p: Pick<TransferState, 'status' | 'failure_reason'>): boolean {
  return p.status === 'failed' || p.status === 'waiting_retry' || (p.status === 'paused' && p.failure_reason === 'err.interrupted');
}

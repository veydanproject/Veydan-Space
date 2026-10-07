// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { messengerTranslations } from '../i18n';
import * as label from './transferLabel';
import { transferActions, type TransferLines, type TransferState, type Words } from './transferLabel';

// The space between a number and its unit does not break; written here as a plain one.
const plain = (s: string) => s.replace(/\u00a0/g, ' ');
const both = (l: TransferLines): TransferLines => ({ line1: plain(l.line1), line2: plain(l.line2) });
const transferLines = (...a: Parameters<typeof label.transferLines>) => both(label.transferLines(...a));
const remoteLines = (...a: Parameters<typeof label.remoteLines>) => both(label.remoteLines(...a));
const shortLine = (...a: Parameters<typeof label.shortLine>) => plain(label.shortLine(...a));

const MB = 1024 * 1024;
const NOW = 1_800_000_000_000;

function words(lang: 'en' | 'ru'): Words {
  const dict = messengerTranslations[lang] as Record<string, string>;
  return {
    locale: lang,
    now: NOW,
    t: (key, vars) => Object.entries(vars ?? {}).reduce((s, [k, v]) => s.split(`{${k}}`).join(v), dict[key] ?? key),
  };
}
const ru = words('ru');

function state(over: Partial<TransferState>): TransferState {
  return {
    direction: 'up', status: 'running', stage: 'uploading', done_bytes: 330 * MB, total_bytes: 812 * MB,
    chunks_done: 82, chunks_total: 203, rate_bps: 5.2 * MB, eta_secs: 92, retry_at_ms: null, failure_reason: null, ...over,
  };
}

describe('the lines of a transfer', () => {
  it('tells an upload part by part, with speed and time left', () => {
    expect(transferLines(state({}), ru)).toEqual({
      line1: 'Шифрование и выгрузка · часть 83 из 203',
      line2: '330 МБ из 812 МБ · 5,2 МБ/с · ~2 мин',
    });
    expect(transferLines(state({}), words('en')).line2).toBe('330 MB of 812 MB · 5.2 MB/s · ~2 min');
  });

  it('leaves out the speed and the time while nothing moves', () => {
    expect(transferLines(state({ rate_bps: 0, eta_secs: null }), ru).line2).toBe('330 МБ из 812 МБ');
  });

  it('names every stage', () => {
    const line1 = (over: Partial<TransferState>) => transferLines(state(over), ru).line1;
    expect(line1({ status: 'queued', stage: 'queued' })).toBe('В очереди');
    expect(line1({ stage: 'preparing' })).toBe('Подготовка: сжатие фото');
    // As the runtime tells it: still queued while the photo is made smaller.
    expect(line1({ status: 'queued', stage: 'preparing' })).toBe('Подготовка: сжатие фото');
    expect(transferLines(state({ stage: 'checking', chunks_done: 7, chunks_total: 24 }), ru))
      .toEqual({ line1: 'Проверка выгруженных частей', line2: '7 из 24' });
    expect(line1({ stage: 'publishing' })).toBe('Отправка сообщения…');
    expect(line1({ direction: 'down', stage: 'downloading', chunks_done: 10, chunks_total: 200 })).toBe('Скачивание · часть 11 из 200');
    expect(line1({ direction: 'down', stage: 'assembling', chunks_done: 34, chunks_total: 75 })).toBe('Расшифровка и сборка · 34 из 75');
    expect(line1({ direction: 'down', stage: 'verifying' })).toBe('Проверка целостности…');
    expect(line1({ stage: 'uploading', chunks_total: 0 })).toBe('Шифрование и выгрузка');
  });

  it('names the part in progress while parts move, counted from 1', () => {
    const line1 = (over: Partial<TransferState>) => transferLines(state(over), ru).line1;
    expect(line1({ chunks_done: 0, chunks_total: 1 })).toBe('Шифрование и выгрузка · часть 1 из 1');
    expect(line1({ chunks_done: 0, chunks_total: 203 })).toBe('Шифрование и выгрузка · часть 1 из 203');
    // The last part done: it is the last one, never one past it.
    expect(line1({ chunks_done: 203, chunks_total: 203 })).toBe('Шифрование и выгрузка · часть 203 из 203');
    expect(line1({ direction: 'down', stage: 'downloading', chunks_done: 0, chunks_total: 1 })).toBe('Скачивание · часть 1 из 1');
    expect(transferLines(state({ chunks_done: 0, chunks_total: 203 }), words('en')).line1).toBe('Encrypting and uploading · part 1 of 203');
    // Counted where they are counted: the parts done.
    expect(transferLines(state({ status: 'paused', chunks_done: 0, chunks_total: 203 }), ru).line1).toBe('Пауза · 0 из 203 частей');
  });

  it('counts down to the next attempt', () => {
    const waiting = state({ status: 'waiting_retry', chunks_done: 30, chunks_total: 63, retry_at_ms: NOW + 11_200 });
    expect(transferLines(waiting, ru)).toEqual({ line1: 'Нет связи, повтор через 12 с', line2: 'выгружено 30 из 63 частей' });
    expect(transferLines(waiting, { ...ru, now: NOW + 20_000 }).line1).toBe('Нет связи, повторяем…');
    expect(transferLines(state({ status: 'waiting_retry', failure_reason: 'err.timeout', retry_at_ms: NOW + 3000 }), ru).line1)
      .toBe('Нет связи, повтор через 3 с');
  });

  it('never tells a busy or failing server as a lost connection', () => {
    const waiting = (failure_reason: string, retry_at_ms: number) =>
      transferLines(state({ status: 'waiting_retry', failure_reason, retry_at_ms }), ru).line1;
    expect(waiting('err.server', NOW + 5000)).toBe('Не получилось, повтор через 5 с');
    expect(waiting('err.rate_limited', NOW)).toBe('Не получилось, повторяем…');
    expect(transferLines(state({ status: 'waiting_retry', failure_reason: 'err.server', retry_at_ms: NOW + 5000 }), words('en')).line1)
      .toBe('Failed, retrying in 5 sec');
  });

  it('says the parts in the right plural form', () => {
    const paused = (chunks_total: number) => transferLines(state({ status: 'paused', chunks_done: 1, chunks_total }), ru).line1;
    expect(paused(1)).toBe('Пауза · 1 из 1 части');
    expect(paused(21)).toBe('Пауза · 1 из 21 части');
    expect(paused(3)).toBe('Пауза · 1 из 3 частей');
    expect(paused(100)).toBe('Пауза · 1 из 100 частей');
    expect(remoteLines(820 * MB, 205, ru).line1).toBe('820 МБ · 205 частей');
    expect(remoteLines(8 * MB, 2, ru).line1).toBe('8,0 МБ · 2 части');
    expect(remoteLines(3 * MB, 1, ru).line1).toBe('3,0 МБ · 1 часть');
    expect(remoteLines(3 * MB, undefined, ru).line1).toBe('3,0 МБ');
    expect(remoteLines(3 * MB, 1, words('en')).line1).toBe('3.0 MB · 1 part');
  });

  it('explains a failure and tells how far it got', () => {
    expect(transferLines(state({ status: 'failed', failure_reason: 'err.network', chunks_done: 4, chunks_total: 12 }), ru))
      .toEqual({ line1: 'Нет связи с хранилищем.', line2: 'выгружено 4 из 12 частей' });
    expect(transferLines(state({ status: 'failed', direction: 'down', failure_reason: null, chunks_done: 5, chunks_total: 15 }), ru))
      .toEqual({ line1: 'Передача не удалась.', line2: 'скачано 5 из 15 частей' });
  });

  it('puts a percent or a countdown under the ring', () => {
    expect(shortLine(state({}), 0, ru)).toBe('41%');
    expect(shortLine(state({ status: 'waiting_retry', retry_at_ms: NOW + 5000 }), 0, ru)).toBe('5 с');
    expect(shortLine(state({ status: 'failed' }), 0, ru)).toBe('');
    expect(shortLine(null, 3 * MB, ru)).toBe('3,0 МБ');
  });
});

describe('the actions of a transfer', () => {
  it('offers what the state allows, the main one first', () => {
    const of = (over: Partial<TransferState>) => transferActions(state(over));
    expect(of({})).toEqual(['pause', 'cancel']);
    expect(of({ status: 'queued', stage: 'queued' })).toEqual(['pause', 'cancel']);
    expect(of({ stage: 'publishing' })).toEqual([]);
    expect(of({ status: 'paused' })).toEqual(['resume', 'cancel']);
    expect(of({ status: 'waiting_retry' })).toEqual(['retry_now', 'cancel']);
    expect(of({ status: 'failed' })).toEqual(['retry', 'cancel']);
    expect(of({ status: 'failed', direction: 'down' })).toEqual(['retry_download', 'cancel']);
    expect(of({ status: 'done' })).toEqual([]);
    expect(transferActions(null)).toEqual(['download']);
  });
});

describe('retry all', () => {
  it('takes up what failed, what waits and what the closing of the app paused, never the user’s pause', () => {
    const of = (over: Partial<TransferState>) => label.retriable(state(over));
    expect(of({ status: 'failed' })).toBe(true);
    expect(of({ status: 'waiting_retry' })).toBe(true);
    expect(of({ status: 'paused', failure_reason: 'err.interrupted' })).toBe(true);
    expect(of({ status: 'paused', failure_reason: null })).toBe(false);
    expect(of({ status: 'running' })).toBe(false);
  });
});

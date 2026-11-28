// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The buttons under a bot's message: how rows are laid out, what of a
// label is shown, what is no button at all, and how long a press waits.

import { describe, expect, it } from 'vitest';
import type { KbButton, MessageKeyboard } from '../api';
import { messengerTranslations } from '../i18n';
import { hasKeys, keyboardOf, labelOf, laterPress, waitingPress, LABEL_MAX, PRESS_WAIT_S } from './keyboard';
import keyboardSource from './Keyboard.svelte?raw';
import bubbleSource from '../dm/MessageBubble.svelte?raw';

/** The code of a component without its comments: what it does, not what it says it does not. */
const code = (src: string) => src.replace(/<!--[\s\S]*?-->/g, '').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

const b = (t: string, d = t, style?: KbButton['style']): KbButton => ({ t, d, ...(style ? { style } : {}) });
const kb = (...rows: KbButton[][]): MessageKeyboard => ({ rows });
const ch = (code: number) => String.fromCharCode(code);

describe('the layout of a keyboard', () => {
  it('gives a row of one button the width of the card, a row of several equal widths', () => {
    const blocks = keyboardOf(kb([b('Разрешить', 'a', 'primary')], [b('Разрешить такие запросы всегда', 's')], [b('Отклонить', 'd', 'danger')]));
    expect(blocks.map((x) => [x.type, x.buttons.length])).toEqual([['row', 1], ['row', 1], ['row', 1]]);
    expect(blocks.map((x) => x.buttons[0].style)).toEqual(['primary', 'quiet', 'danger']);
    const two = keyboardOf(kb([b('Да'), b('Нет')]));
    expect(two).toEqual([{ type: 'row', buttons: [{ label: 'Да', style: 'quiet', d: 'Да' }, { label: 'Нет', style: 'quiet', d: 'Нет' }] }]);
  });

  it('lets two rows of short labels or more flow together as chips', () => {
    const blocks = keyboardOf(kb([b('English'), b('Русский')], [b('Deutsch'), b('Español')], [b('Français'), b('中文')], [b('Отмена')]));
    expect(blocks.map((x) => [x.type, x.buttons.length])).toEqual([['chips', 6], ['row', 1]]);
  });

  it('keeps rows when a label is longer than 12, a row has one button, or one such row stands alone', () => {
    // 12 characters is short, 13 is not.
    expect(keyboardOf(kb([b('abcdefghijkl'), b('x')], [b('y'), b('z')]))[0].type).toBe('chips');
    expect(keyboardOf(kb([b('abcdefghijklm'), b('x')], [b('y'), b('z')])).map((x) => x.type)).toEqual(['row', 'row']);
    expect(keyboardOf(kb([b('Да')], [b('Нет')])).map((x) => x.type)).toEqual(['row', 'row']);
    expect(keyboardOf(kb([b('Да'), b('Нет')], [b('Подробнее о правилах')], [b('A'), b('B')])).map((x) => x.type)).toEqual(['row', 'row', 'row']);
  });

  it('draws at most 8 rows of 4', () => {
    const row = [b('1'), b('2'), b('3'), b('4'), b('5')];
    const blocks = keyboardOf(kb(...Array.from({ length: 10 }, () => [b('a long label here')])));
    expect(blocks).toHaveLength(8);
    expect(keyboardOf(kb(row))[0].buttons.map((x) => x.d)).toEqual(['1', '2', '3', '4']);
  });

  it('is nothing without rows', () => {
    expect(keyboardOf(null)).toEqual([]);
    expect(keyboardOf(undefined)).toEqual([]);
    expect(keyboardOf({ rows: [] })).toEqual([]);
    expect(keyboardOf({ rows: 'x' } as unknown as MessageKeyboard)).toEqual([]);
  });
});

describe('the label of a button', () => {
  it('is cut at 64 characters, the last one «…»', () => {
    expect(labelOf('a'.repeat(LABEL_MAX))).toBe('a'.repeat(LABEL_MAX));
    const cut = labelOf('я'.repeat(LABEL_MAX + 10));
    expect([...cut]).toHaveLength(LABEL_MAX);
    expect(cut.endsWith('…')).toBe(true);
  });

  it('is one line of plain words: no marks that turn the letters, no control characters', () => {
    const rtlo = ch(0x202e);
    expect(labelOf(`${rtlo}Отклонить`)).toBe('Отклонить');
    expect(labelOf(`Раз${ch(0x2066)}решить${ch(0x2069)}`)).toBe('Разрешить');
    expect(labelOf(`Да\nнет\t${ch(0)}ок`)).toBe('Да нет ок');
    expect(labelOf('  <b>Да</b>  ')).toBe('<b>Да</b>');
    expect(labelOf(42)).toBe('');
  });
});

describe('what is no button', () => {
  it('leaves out a button without a label, without `d`, with a `d` over 128 bytes, or with a link', () => {
    const blocks = keyboardOf(kb(
      [b('', 'x')],
      [{ t: 'Нет данных' } as KbButton],
      [{ t: 'Пусто', d: '' }],
      [b('Длинные', 'я'.repeat(65))],
      [{ t: 'Сайт', url: 'https://example.com' } as unknown as KbButton],
      [{ t: 'Скрипт', d: 'x', url: 'javascript:alert(1)' } as unknown as KbButton],
      [b('Годится', 'ok:1')],
    ));
    expect(blocks.flatMap((x) => x.buttons.map((y) => y.label))).toEqual(['Годится']);
    expect(keyboardOf(kb([b('Ровно', 'я'.repeat(64))]))[0].buttons).toHaveLength(1);
  });

  it('drops a row left empty and an unknown style becomes quiet', () => {
    const blocks = keyboardOf(kb([b('', 'x')], [{ t: 'Да', d: 'y', style: 'loud' } as unknown as KbButton]));
    expect(blocks).toHaveLength(1);
    expect(blocks[0].buttons[0].style).toBe('quiet');
  });

  it('hands `d` back exactly as the bot wrote it', () => {
    const d = ' {"a":1} <x> ';
    expect(keyboardOf(kb([b('Да', d)]))[0].buttons[0].d).toBe(d);
  });

  it('is never drawn under my own message nor under one taken back', () => {
    const k = kb([b('Да')]);
    expect(hasKeys({ direction: 'in', deleted: false, kb: k })).toBe(true);
    expect(hasKeys({ direction: 'out', deleted: false, kb: k })).toBe(false);
    expect(hasKeys({ direction: 'in', deleted: true, kb: k })).toBe(false);
    expect(hasKeys({ direction: 'in', deleted: false, kb: kb([b('', 'x')]) })).toBe(false);
    expect(hasKeys({ direction: 'in', deleted: false })).toBe(false);
  });
});

describe('a press waiting for the bot', () => {
  const blocks = keyboardOf(kb([b('Да', 'yes'), b('Нет', 'no')]));
  const at = 1_000_000;

  it('waits for 30 s, then the buttons are live again', () => {
    expect(waitingPress({ d: 'yes', at }, blocks, null, at)).toEqual({ d: 'yes', at });
    expect(waitingPress({ d: 'yes', at }, blocks, null, at + PRESS_WAIT_S - 0.1)).not.toBeNull();
    expect(waitingPress({ d: 'yes', at }, blocks, null, at + PRESS_WAIT_S)).toBeNull();
    expect(waitingPress(null, blocks, null, at)).toBeNull();
  });

  it('ends with the bot\'s edit, and with a keyboard that has no such button', () => {
    expect(waitingPress({ d: 'yes', at }, blocks, at + 1, at + 2)).toBeNull();
    expect(waitingPress({ d: 'yes', at }, blocks, at, at + 2)).toBeNull();
    // An edit before the press (the bot put the buttons on later) is no answer.
    expect(waitingPress({ d: 'yes', at }, blocks, at - 5, at + 2)).not.toBeNull();
    expect(waitingPress({ d: 'maybe', at }, blocks, null, at)).toBeNull();
  });

  it('does not wait for a press from a clock far ahead', () => {
    expect(waitingPress({ d: 'yes', at: at + 3600 }, blocks, null, at)).toBeNull();
  });

  it('takes the later of mine and the runtime\'s', () => {
    expect(laterPress(null, null)).toBeNull();
    expect(laterPress({ d: 'a', at: 1 }, null)).toEqual({ d: 'a', at: 1 });
    expect(laterPress(null, { d: 'b', at: 2 })).toEqual({ d: 'b', at: 2 });
    expect(laterPress({ d: 'a', at: 1 }, { d: 'b', at: 2 })).toEqual({ d: 'b', at: 2 });
    expect(laterPress({ d: 'a', at: 3 }, { d: 'b', at: 2 })).toEqual({ d: 'a', at: 3 });
  });
});

describe('the component', () => {
  it('shows labels as text and does nothing on a press but the command', () => {
    const src = code(keyboardSource);
    expect(src).not.toMatch(/\{@html/);
    expect(src).not.toMatch(/clipboard|window\.open|openUrl|confirm\(|alert\(|goto\(|location|invoke\(/);
    expect(src.match(/messengerApi\.\w+/g)).toEqual(['messengerApi.press']);
    expect(keyboardSource).toMatch(/aria-disabled=/);
    expect(keyboardSource).toMatch(/<button type="button"/);
  });

  it('stands in the bubble after the words, the time under it', () => {
    expect(bubbleSource).toMatch(/\{#if keys\}<Keyboard message=\{m\} \/>\{\/if\}/);
    expect(bubbleSource).toMatch(/!m\.bot_card && !keys/);
  });

  it('says a failed press in both languages written here', () => {
    expect(messengerTranslations.en.msg_kb_press_failed).toBe('The press did not get through');
    expect(messengerTranslations.ru.msg_kb_press_failed).toBe('Нажатие не дошло');
  });
});

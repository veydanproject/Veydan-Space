// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { commandsFor, stepRow, typedCommand } from './bot-commands';

const list = [
  { name: 'help', about: 'The commands of the bot' },
  { name: 'new', about: 'Start a new session' },
  { name: 'news', about: '' },
  { name: 'status', about: 'How the bot is doing' },
];
const names = (text: string, all = false) => commandsFor(list, text, all).map((c) => c.name);

describe('the «/» menu', () => {
  it('reads a command being typed: a slash and a name, no space yet', () => {
    expect(typedCommand('/')).toBe('');
    expect(typedCommand('/NeW')).toBe('new');
    expect(typedCommand('/new ')).toBeNull();
    expect(typedCommand('/new words')).toBeNull();
    expect(typedCommand(' /new')).toBeNull();
    expect(typedCommand('new')).toBeNull();
    expect(typedCommand('')).toBeNull();
  });

  it('shows the commands whose name begins with what is typed', () => {
    expect(names('/')).toEqual(['help', 'new', 'news', 'status']);
    expect(names('/ne')).toEqual(['new', 'news']);
    expect(names('/NEWS')).toEqual(['news']);
    expect(names('/x')).toEqual([]);
    expect(names('/new please'), 'a space ends the name: the words are a message').toEqual([]);
  });

  it('shows all of them when the button asked, whatever the field holds, unless a name is being typed', () => {
    expect(names('', true)).toEqual(['help', 'new', 'news', 'status']);
    expect(names('half a sentence', true)).toEqual(['help', 'new', 'news', 'status']);
    expect(names('/st', true)).toEqual(['status']);
    expect(names('half a sentence')).toEqual([]);
    expect(commandsFor([], '/', true)).toEqual([]);
  });

  it('moves the highlight round the list', () => {
    expect(stepRow(0, 1, 3)).toBe(1);
    expect(stepRow(2, 1, 3)).toBe(0);
    expect(stepRow(0, -1, 3)).toBe(2);
    expect(stepRow(0, 1, 0)).toBe(0);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** App shortcuts matched by physical key (`code`), so the layout does not matter. */

import { get, writable } from 'svelte/store';

export interface Chord {
  code: string;
  /** Ctrl on Linux/Windows, Cmd on macOS */
  mod: boolean;
  shift: boolean;
  alt: boolean;
}

export type CommandId =
  | 'edit.undo'
  | 'edit.redo'
  | 'editor.bold'
  | 'editor.italic'
  | 'editor.link'
  | 'editor.find'
  | 'editor.save'
  | 'app.palette'
  | 'app.inspector';

export type Overrides = Partial<Record<CommandId, Chord[]>>;

const chord = (code: string, mod: boolean, shift = false, alt = false): Chord => ({ code, mod, shift, alt });

export const DEFAULT_CHORDS: Record<CommandId, Chord[]> = {
  'edit.undo': [chord('KeyZ', true)],
  'edit.redo': [chord('KeyZ', true, true), chord('KeyY', true)],
  'editor.bold': [chord('KeyB', true)],
  'editor.italic': [chord('KeyI', true)],
  'editor.link': [chord('KeyK', true)],
  'editor.find': [chord('KeyF', true)],
  'editor.save': [chord('KeyS', true)],
  'app.palette': [chord('KeyP', true), chord('KeyP', true, true)],
  'app.inspector': [chord('KeyD', true, true)],
};

export const COMMAND_IDS = Object.keys(DEFAULT_CHORDS) as CommandId[];

/**
 * The groups of Settings → Hotkeys. A group marked `module` holds the
 * shortcuts of a module's screen (the note editor): it is listed where a
 * module that is switched on claims it (`ModuleDef.hotkeys`), so a product
 * without that module does not offer shortcuts that do nothing.
 */
export const HOTKEY_GROUPS: { id: 'edit' | 'editor' | 'app'; commands: CommandId[]; module?: boolean }[] = [
  { id: 'edit', commands: ['edit.undo', 'edit.redo'] },
  { id: 'editor', module: true, commands: ['editor.bold', 'editor.italic', 'editor.link', 'editor.find', 'editor.save'] },
  { id: 'app', commands: ['app.palette', 'app.inspector'] },
];

const STORAGE_KEY = 'veydan_keybindings';

const MODIFIER_CODES = new Set([
  'ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight',
  'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight', 'OSLeft', 'OSRight',
]);

const CODE_LABELS: Record<string, string> = {
  Space: 'Space',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  Backquote: '`',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  Escape: 'Esc',
  Backspace: 'Backspace',
  Delete: 'Del',
  Enter: 'Enter',
  Tab: 'Tab',
};

function parseChord(raw: unknown): Chord | null {
  if (!raw || typeof raw !== 'object') return null;
  const src = raw as Record<string, unknown>;
  if (typeof src.code !== 'string' || !src.code || MODIFIER_CODES.has(src.code)) return null;
  return {
    code: src.code,
    mod: src.mod === true,
    shift: src.shift === true,
    alt: src.alt === true,
  };
}

function loadOverrides(): Overrides {
  if (typeof localStorage === 'undefined') return {};
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as Record<string, unknown>;
    if (!raw || typeof raw !== 'object') return {};
    const out: Overrides = {};
    for (const id of COMMAND_IDS) {
      if (!Object.prototype.hasOwnProperty.call(raw, id)) continue;
      const list = raw[id];
      if (!Array.isArray(list)) continue;
      out[id] = list.map(parseChord).filter((c): c is Chord => c !== null);
    }
    return out;
  } catch {
    return {};
  }
}

function persist(val: Overrides) {
  if (typeof localStorage === 'undefined') return;
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(val)); } catch { /* quota */ }
}

export const keybindingOverrides = writable<Overrides>(loadOverrides());

/** True while Settings is listening for the next shortcut. Commands stay idle. */
export const shortcutCapture = writable(false);

function commit(next: Overrides) {
  persist(next);
  return next;
}

/** Active chords: a stored array (possibly empty) replaces the default. */
export function effective(command: CommandId, map: Overrides = get(keybindingOverrides)): Chord[] {
  if (Object.prototype.hasOwnProperty.call(map, command)) return map[command] ?? [];
  return DEFAULT_CHORDS[command];
}

export function isOverridden(command: CommandId, map: Overrides = get(keybindingOverrides)): boolean {
  return Object.prototype.hasOwnProperty.call(map, command);
}

function sameChord(a: Chord, b: Chord): boolean {
  return a.code === b.code && a.mod === b.mod && a.shift === b.shift && a.alt === b.alt;
}

/** Other command that already uses this chord, if any. */
export function findConflict(command: CommandId, chord: Chord, map: Overrides = get(keybindingOverrides)): CommandId | null {
  for (const id of COMMAND_IDS) {
    if (id === command) continue;
    if (effective(id, map).some((c) => sameChord(c, chord))) return id;
  }
  return null;
}

export function matches(e: KeyboardEvent, command: CommandId): boolean {
  if (get(shortcutCapture) || e.isComposing || !e.code || MODIFIER_CODES.has(e.code)) return false;
  const pressed: Chord = { code: e.code, mod: e.ctrlKey || e.metaKey, shift: e.shiftKey, alt: e.altKey };
  return effective(command).some((c) => sameChord(c, pressed));
}

/** Ctrl/Cmd+Z or Ctrl/Cmd+Y, including Shift. TipTap also binds these by letter. */
export function stockHistoryChord(e: KeyboardEvent): boolean {
  if (e.altKey || !(e.ctrlKey || e.metaKey)) return false;
  return e.code === 'KeyZ' || e.code === 'KeyY';
}

/** Ctrl/Cmd+B or Ctrl/Cmd+I without Shift. TipTap binds these by letter. */
export function stockMarkChord(e: KeyboardEvent): boolean {
  if (e.altKey || e.shiftKey || !(e.ctrlKey || e.metaKey)) return false;
  return e.code === 'KeyB' || e.code === 'KeyI';
}

/** Calls `run` and returns true when the event is undo or redo. */
export function takeHistory(e: KeyboardEvent, run: (command: 'edit.undo' | 'edit.redo') => void): boolean {
  if (matches(e, 'edit.undo')) { run('edit.undo'); return true; }
  if (matches(e, 'edit.redo')) { run('edit.redo'); return true; }
  return false;
}

/** Replace the command's chords. Returns the conflicting command and does not save. */
export function assignChord(command: CommandId, next: Chord): CommandId | null {
  const other = findConflict(command, next);
  if (other) return other;
  keybindingOverrides.update((o) => commit({ ...o, [command]: [next] }));
  return null;
}

/** Remove the shortcut until the user resets it. */
export function clearCommand(command: CommandId) {
  keybindingOverrides.update((o) => commit({ ...o, [command]: [] }));
}

export function resetCommand(command: CommandId) {
  keybindingOverrides.update((o) => {
    const next = { ...o };
    delete next[command];
    return commit(next);
  });
}

function modLabel(): string {
  if (typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)) return 'Cmd';
  return 'Ctrl';
}

function codeLabel(code: string): string {
  if (code.startsWith('Key') && code.length === 4) return code.slice(3);
  if (code.startsWith('Digit')) return code.slice(5);
  if (code.startsWith('Numpad')) return `Num ${code.slice(6)}`;
  return CODE_LABELS[code] ?? code;
}

/** Latin label by key position, e.g. Ctrl+Shift+Z. */
export function formatChord(c: Chord): string {
  const parts: string[] = [];
  if (c.mod) parts.push(modLabel());
  if (c.shift) parts.push('Shift');
  if (c.alt) parts.push('Alt');
  parts.push(codeLabel(c.code));
  return parts.join('+');
}

export function formatCommand(command: CommandId, map: Overrides = get(keybindingOverrides)): string {
  return effective(command, map).map(formatChord).join(', ');
}

export type Capture =
  | { kind: 'ignore' }
  | { kind: 'cancel' }
  | { kind: 'clear' }
  | { kind: 'reject' }
  | { kind: 'ok'; chord: Chord };

/** Interpret one keydown while the settings row is listening. */
export function captureEvent(e: KeyboardEvent): Capture {
  if (e.isComposing || e.repeat || !e.code || e.code === 'Unidentified' || MODIFIER_CODES.has(e.code)) {
    return { kind: 'ignore' };
  }
  const next: Chord = { code: e.code, mod: e.ctrlKey || e.metaKey, shift: e.shiftKey, alt: e.altKey };
  if (e.code === 'Escape' && !next.mod && !next.alt) return { kind: 'cancel' };
  if ((e.code === 'Backspace' || e.code === 'Delete') && !next.mod && !next.alt && !next.shift) return { kind: 'clear' };
  if (!next.mod && !next.alt) return { kind: 'reject' };
  return { kind: 'ok', chord: next };
}

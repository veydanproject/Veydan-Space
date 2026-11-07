// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A command the UI calls is one the Rust side registers. Merging button
// words under `common_*` once renamed `cloud_recover_finish` and `totp_delete`
// along with their mocks, so the tests passed while both commands failed in
// the app; this checks every `call` and `mockCommand` name against the golden
// lists of the products.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const ROOT = join(__dirname, '..', '..');
const REPO = join(ROOT, '..', '..');

const GOLDEN = [
  'apps/space/src/modules/commands.golden.txt',
  'apps/chat/src/commands.golden.txt',
  'apps/notes/src/commands.golden.txt',
  'apps/pass/src/commands.golden.txt',
];

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.(svelte|ts)$/.test(name) && !name.endsWith('.test.ts') ? [path] : [];
  });
}

const known = new Set(
  GOLDEN.flatMap((path) => readFileSync(join(REPO, path), 'utf8').split('\n').filter(Boolean).map((line) => line.split(' ')[0])),
);

describe('the commands of the UI', () => {
  it('are all registered by Rust', () => {
    const unknown: string[] = [];
    for (const path of files(join(ROOT, 'lib'))) {
      const text = readFileSync(path, 'utf8');
      // A file with its own `call` (the dialer's) calls no command.
      if (!/\bcall\b[^;]*from '\$lib\/core\/api'/.test(text) && !text.includes('mockCommand')) continue;
      for (const m of text.matchAll(/\b(?:call(?:<[^>()]*(?:<[^>]*>)?[^>()]*>)?|mockCommand)\(\s*'([a-z0-9_]+)'/g)) {
        if (!known.has(m[1])) unknown.push(`${relative(ROOT, path)}: ${m[1]}`);
      }
    }
    expect(unknown).toEqual([]);
  });

  it('keep the names the button words once took', () => {
    expect(known.has('cloud_recover_finish')).toBe(true);
    expect(known.has('totp_delete')).toBe(true);
  });
});

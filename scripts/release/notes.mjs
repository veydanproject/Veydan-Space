#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
//
// The release note of a product version (docs/ci-cd.md, "Release notes";
// internal/platform-spec.md 14.1): apps/<product>/release-notes/<X.Y.Z>.md,
// written from TEMPLATE.md of the same folder before `make push <product> rc`.
// One note per version: every rc of it and the release show the same text.
//
//   node scripts/release/notes.mjs check <product> [X.Y.Z]
//       the note of the version (VERSION by default) is there and follows
//       the rules; scripts/push.sh runs it before an rc and a release
//   node scripts/release/notes.mjs body <file> [--kind rc|release]
//       the text of the release on GitHub (and of latest.json)
//   node scripts/release/notes.mjs draft <product>
//       for the one who writes the note: the template and the commits since
//       the last release that touch the product
//
// The rules: English; sections "## New", "## Improved", "## Fixed" in this
// order, only those that have something; each line "- " and one short
// sentence, said for the people who use the app — what changed for them,
// never how: no code, file names, commit ids, links, versions of libraries
// or words of the build.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

export const SECTIONS = ['New', 'Improved', 'Fixed'];
export const MAX_LINES = 15;
export const MAX_LINE = 140;

/** Words of the build and the code: a note says what changed for a person, not how. */
const TECHNICAL = [
  'crate', 'crates', 'rust', 'tauri', 'svelte', 'sveltekit', 'typescript', 'javascript', 'cargo', 'pnpm', 'npm',
  'clippy', 'refactor', 'refactored', 'refactoring', 'commit', 'commits', 'merge', 'merged', 'pull request',
  'ci', 'workflow', 'gitea', 'github', 'regex', 'stack trace', 'stacktrace', 'panic', 'panics', 'mutex', 'async',
  'struct', 'enum', 'null', 'undefined', 'sql', 'sqlite', 'migration', 'migrations', 'webrtc', 'libwebrtc',
  'nostr', 'blossom', 'vpush', 'vlink', 'vcall', 'vhub', 'sfu', 'endpoint', 'json', 'api', 'backend', 'frontend',
  'dependency', 'dependencies', 'lint', 'linter', 'unit test', 'tests', 'debug', 'deserialize', 'serialize',
];
const TECHNICAL_RE = new RegExp(`\\b(${TECHNICAL.map((w) => w.replace(/ /g, '\\s+')).join('|')})\\b`, 'i');

/** The text without the comments of the template, trimmed. */
export function strip(text) {
  return text.replace(/<!--[\s\S]*?-->/g, '').replace(/\r\n/g, '\n').trim();
}

/** What is wrong with a note: an empty list is a note that may be published. */
export function problems(text) {
  const out = [];
  const body = strip(text);
  if (!body) return ['the note is empty'];
  let section = -1;
  let inSection = 0;
  let lines = 0;
  const close = () => { if (section >= 0 && inSection === 0) out.push(`the section "## ${SECTIONS[section]}" has no line: drop it`); };
  for (const [i, raw] of body.split('\n').entries()) {
    const line = raw.trimEnd();
    const at = `line ${i + 1}`;
    if (!line.trim()) continue;
    const heading = /^## (.+)$/.exec(line);
    if (heading) {
      close();
      const k = SECTIONS.indexOf(heading[1].trim());
      if (k < 0) out.push(`${at}: "## ${heading[1]}" — the sections are ${SECTIONS.map((s) => `"## ${s}"`).join(', ')}`);
      else if (k <= section) out.push(`${at}: "## ${heading[1]}" — each section once, in the order ${SECTIONS.join(', ')}`);
      else section = k;
      inSection = 0;
      continue;
    }
    const item = /^-(?: (.*))?$/.exec(line);
    if (!item) { out.push(`${at}: "${line}" — a line is "## <section>" or "- <one sentence>"`); continue; }
    if (section < 0) out.push(`${at}: a line before any section`);
    const s = (item[1] ?? '').trim();
    inSection++;
    lines++;
    if (!s) { out.push(`${at}: an empty line of the template`); continue; }
    if (s.length > MAX_LINE) out.push(`${at}: ${s.length} characters, at most ${MAX_LINE}: say it shorter`);
    if (/[Ѐ-ӿ]/.test(s)) out.push(`${at}: the note is in English`);
    if (/`/.test(s)) out.push(`${at}: no code`);
    if (/https?:\/\/|www\./i.test(s)) out.push(`${at}: no links`);
    if (/\b[\w-]+\.(rs|ts|js|mjs|svelte|toml|json|md|ya?ml|sh|kt|gradle|lock)\b/i.test(s) || /\b[\w-]+\/[\w-]+\/[\w./-]+/.test(s)) out.push(`${at}: no file names`);
    if (/\b(?=[0-9a-f]*\d)(?=[0-9a-f]*[a-f])[0-9a-f]{7,40}\b/.test(s)) out.push(`${at}: no commit ids`);
    if (/\bv?\d+\.\d+\.\d+\b/.test(s)) out.push(`${at}: no version numbers`);
    const word = TECHNICAL_RE.exec(s);
    if (word) out.push(`${at}: "${word[1]}" is a word of the build — say what changed for the person who uses the app`);
  }
  close();
  if (lines === 0) out.push('the note has no line');
  if (lines > MAX_LINES) out.push(`${lines} lines, at most ${MAX_LINES}: keep what matters to people`);
  return out;
}

/** The text of the release: the note, an rc says what it is, and where the files are. */
export function releaseBody(text, { kind = 'release' } = {}) {
  const head = kind === 'rc'
    ? 'A release candidate, for testing before the release. Installed apps do not update to it.\n\n'
    : '';
  return `${head}${strip(text)}\n\nDownload the file for your system below.\n`;
}

const manifest = () => JSON.parse(fs.readFileSync(path.join(ROOT, 'products.json'), 'utf8'));

/** apps/<product>/release-notes/<version>.md of the monorepo (the version of VERSION by default). */
export function noteFile(product, version = null) {
  const target = manifest().targets[product];
  if (!target || target.kind !== 'product') throw new Error(`'${product}' is not a product of products.json`);
  const v = version ?? fs.readFileSync(path.join(ROOT, target.app, 'VERSION'), 'utf8').trim();
  return { app: target.app, version: v, file: path.join(target.app, 'release-notes', `${v}.md`) };
}

function git(args) {
  const r = spawnSync('git', ['-C', ROOT, ...args], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')}: ${r.stderr}`);
  return r.stdout;
}

/** The commits since the last release of the product that touch its own files (not the build's). */
function draft(product) {
  const { app, version, file } = noteFile(product);
  // affected.mjs compares each product with its last release, not a dev build or an rc.
  const r = spawnSync(process.execPath, [path.join(ROOT, 'scripts/affected.mjs'), '--json'], { cwd: ROOT, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`scripts/affected.mjs: ${r.stderr}`);
  const affected = JSON.parse(r.stdout);
  const base = affected.bases?.[product] ?? null;
  const files = (affected.files?.[product] ?? []).filter((f) => !/^(scripts|docs|internal|\.github|\.gitea|ops)\//.test(f)
    && !/(^|\/)(VERSION|Cargo\.lock|Cargo\.toml|tauri(\.\w+)?\.conf\.json|release-notes\/.*)$/.test(f));
  const out = [];
  out.push(`# The note of ${product} ${version}: ${file}`);
  out.push(`# Copy ${path.join(app, 'release-notes/TEMPLATE.md')}, keep the sections that have something.`);
  out.push(`# The changes since ${base ?? 'the beginning'} in the files of ${product} (${files.length} files):`);
  if (files.length) {
    const log = git(['log', '--no-merges', '--format=%h %s', ...(base ? [`${base}..HEAD`] : ['HEAD']), '--', ...files]).trim();
    out.push(log || '(no commit)');
  } else {
    out.push('(nothing of the product changed: a note like "- Maintenance update: the same app, built again." under "## Improved")');
  }
  return out.join('\n');
}

function main(argv) {
  const [command, ...rest] = argv;
  if (command === 'check') {
    const [product, version] = rest;
    const { file } = noteFile(product, version ?? null);
    const full = path.join(ROOT, file);
    if (!fs.existsSync(full)) {
      console.error(`notes: ${file} is missing — write the release note from ${path.join(path.dirname(file), 'TEMPLATE.md')} (node scripts/release/notes.mjs draft ${product}) and merge it before an rc or a release`);
      return 1;
    }
    const found = problems(fs.readFileSync(full, 'utf8'));
    if (found.length) {
      console.error(`notes: ${file}:`);
      for (const p of found) console.error(`  ${p}`);
      return 1;
    }
    console.log(`notes: ${file} ok`);
    return 0;
  }
  if (command === 'body') {
    const file = rest[0];
    const i = rest.indexOf('--kind');
    const kind = i < 0 ? 'release' : rest[i + 1];
    const text = fs.readFileSync(file, 'utf8');
    const found = problems(text);
    if (found.length) {
      console.error(`notes: ${file}:\n  ${found.join('\n  ')}`);
      return 1;
    }
    process.stdout.write(releaseBody(text, { kind }));
    return 0;
  }
  if (command === 'draft') {
    console.log(draft(rest[0]));
    return 0;
  }
  console.error('usage: notes.mjs check <product> [X.Y.Z] | body <file> [--kind rc|release] | draft <product>');
  return 2;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exit(main(process.argv.slice(2)));
  } catch (error) {
    console.error(`notes: ${error.message}`);
    process.exit(2);
  }
}

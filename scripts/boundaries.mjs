#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
//
// The scans of scripts/boundaries.sh that read source text. Each prints one
// line per finding and exits 0; any other exit status means the scan itself
// failed, and the caller takes that for a failure of the check.
//
//   commands <golden file> <src dir of the product> [<src dir>[=<crate>]]...
//       the platform of every command, read from the `module!` lists; a
//       crate of a module is named after the `=`, and the product lists it
//       as `<crate>::module()`
//   rules <lib|module|plugin|product> <package> <dir> <build script | -> <yes|no>
//       the formal rules of the module contract; the last word says whether
//       the crate depends on Tauri
//   tables <lib|module> <src dir> [[+]<src dir of another crate>]...
//       the tables the SQL of a crate names that its schema does not create;
//       a crate given with `+` lends its tables, the others' are foreign
//   tables product <src dir> <table>...
//       where the SQL of a product names one of these tables
//   imports <dir>
//       what the UI files of a directory import: <file>:<line> TAB <path>
//   ui <products.json>
//       the matrix of the frontend (platform-spec 6.3): where a UI file
//       imports what its directory may not
//   ui-selftest
//       the ui scan against a scratch tree of planted imports: what it
//       missed or reported wrongly
//   foreign <products.json of the monorepo> <product> <root of a snapshot>
//       what a published snapshot of the product holds of the modules and
//       products it is not: their files, and imports that point at them

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const read = (file) => fs.readFileSync(file, 'utf8');
const lineOf = (text, at) => text.slice(0, at).split('\n').length;

/** Every file under `dir` whose name matches, build output and vendored code aside. */
function walk(dir, pattern) {
  const found = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!['target', 'node_modules', 'gen'].includes(entry.name) && !entry.name.startsWith('.')) found.push(...walk(full, pattern));
    } else if (pattern.test(entry.name)) {
      found.push(full);
    }
  }
  return found;
}

// ── Rust source ─────────────────────────────────────────────────────────────

/** Rust source of the same length and lines with comments blanked and
 *  literals defused: nothing inside a string or a character reads as a
 *  bracket, a separator, a quote or the start of an attribute or a macro. */
function clean(src) {
  const out = src.split('');
  const blank = (from, to) => { for (let k = from; k < to; k++) if (out[k] !== '\n') out[k] = ' '; };
  const defuse = (from, to) => { for (let k = from; k < to; k++) if ('{}()[];,#"\'!\\'.includes(out[k])) out[k] = '_'; };
  const n = src.length;
  let i = 0;
  while (i < n) {
    const c = src[i];
    const d = src[i + 1];
    if (c === '/' && d === '/') {
      let j = src.indexOf('\n', i);
      if (j < 0) j = n;
      blank(i, j);
      i = j;
    } else if (c === '/' && d === '*') {
      let depth = 1;
      let j = i + 2;
      while (j < n && depth > 0) {
        if (src.startsWith('/*', j)) { depth++; j += 2; }
        else if (src.startsWith('*/', j)) { depth--; j += 2; }
        else j++;
      }
      blank(i, j);
      i = j;
    } else if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') j += src[j] === '\\' ? 2 : 1;
      defuse(i + 1, Math.min(j, n));
      i = j + 1;
    } else if (c === 'r' && /^r#*"/.test(src.slice(i, i + 40)) && !/\w/.test((src[i - 1] ?? ' ') === 'b' ? (src[i - 2] ?? ' ') : (src[i - 1] ?? ' '))) {
      const hashes = /^r(#*)"/.exec(src.slice(i, i + 40))[1];
      const start = i + 2 + hashes.length;
      let j = src.indexOf('"' + hashes, start);
      if (j < 0) j = n;
      defuse(start, j);
      i = j + 1 + hashes.length;
    } else if (c === "'") {
      if (d === '\\') {
        let j = src.indexOf("'", i + 3);
        if (j < 0) j = n;
        defuse(i + 1, j);
        i = j + 1;
      } else if (src[i + 2] === "'") {
        defuse(i + 1, i + 2);
        i += 3;
      } else if (src[i + 3] === "'" && /[\uD800-\uDBFF]/.test(d ?? '')) {
        i += 4;
      } else {
        // A lifetime.
        i++;
      }
    } else {
      i++;
    }
  }
  return out.join('');
}

/** The text of every string literal of Rust source, comments aside. */
function literals(src) {
  const found = [];
  const n = src.length;
  let i = 0;
  while (i < n) {
    const c = src[i];
    const d = src[i + 1];
    if (c === '/' && d === '/') {
      const j = src.indexOf('\n', i);
      i = j < 0 ? n : j;
    } else if (c === '/' && d === '*') {
      let depth = 1;
      let j = i + 2;
      while (j < n && depth > 0) {
        if (src.startsWith('/*', j)) { depth++; j += 2; }
        else if (src.startsWith('*/', j)) { depth--; j += 2; }
        else j++;
      }
      i = j;
    } else if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') j += src[j] === '\\' ? 2 : 1;
      found.push({ at: i, text: src.slice(i + 1, j) });
      i = j + 1;
    } else if (c === 'r' && /^r#*"/.test(src.slice(i, i + 40)) && !/\w/.test(src[i - 1] ?? ' ')) {
      const hashes = /^r(#*)"/.exec(src.slice(i, i + 40))[1];
      const start = i + 2 + hashes.length;
      let j = src.indexOf('"' + hashes, start);
      if (j < 0) j = n;
      found.push({ at: i, text: src.slice(start, j) });
      i = j + 1 + hashes.length;
    } else if (c === "'" && d === '\\') {
      const j = src.indexOf("'", i + 3);
      i = j < 0 ? n : j + 1;
    } else if (c === "'" && src[i + 2] === "'") {
      i += 3;
    } else {
      i++;
    }
  }
  return found;
}

/** The same with the text of every string blanked too. */
const withoutStrings = (code) => code.replace(/"[^"]*"/g, (s) => '"' + s.slice(1, -1).replace(/[^\n]/g, ' ') + '"');

const OPEN = '([{';
const CLOSE = ')]}';

/** The index of the bracket that closes the one at `open`. */
function matching(code, open) {
  let depth = 0;
  for (let i = open; i < code.length; i++) {
    if (OPEN.includes(code[i])) depth++;
    else if (CLOSE.includes(code[i]) && --depth === 0) return i;
  }
  throw new Error(`a bracket opened at offset ${open} is never closed`);
}

/** The attributes in `text`, outer and inner, each without its whitespace. */
function attributes(text) {
  const found = [];
  const start = /#\s*!?\s*\[/g;
  let m;
  while ((m = start.exec(text))) {
    const open = m.index + m[0].length - 1;
    let close;
    try { close = matching(text, open); } catch { break; }
    found.push({ at: m.index, text: text.slice(open + 1, close).replace(/\s+/g, '') });
    start.lastIndex = close;
  }
  return found;
}

/** The `cfg` conditions that govern offset `at`: those on the item, the
 *  statement or the list entry it is in, on everything around that one, and
 *  the inner attributes of the blocks around it. */
function cfgsAt(code, at) {
  const levels = [{ start: 0, until: 0, inner: [] }];
  for (let i = 0; i < at; i++) {
    const c = code[i];
    const top = levels[levels.length - 1];
    if (OPEN.includes(c)) {
      top.until = i;
      levels.push({ start: i + 1, until: 0, inner: [] });
    } else if (CLOSE.includes(c)) {
      if (levels.length === 1) throw new Error(`a bracket at offset ${i} closes nothing`);
      levels.pop();
      const parent = levels[levels.length - 1];
      if (c === ']' && /#\s*!\s*$/.test(code.slice(parent.start, parent.until))) {
        parent.inner.push(code.slice(parent.until + 1, i).replace(/\s+/g, ''));
        parent.start = i + 1;
      } else if (c === '}') {
        parent.start = i + 1;
      }
    } else if (c === ';' || c === ',') {
      top.start = i + 1;
    }
  }
  const found = [];
  levels.forEach((level, k) => {
    const end = k === levels.length - 1 ? at : level.until;
    found.push(...level.inner, ...attributes(code.slice(level.start, end)).map((a) => a.text));
  });
  return found.filter((text) => text.startsWith('cfg('));
}

/** Whether a set of conditions leaves the code to the tests alone. */
const testOnly = (cfgs) => cfgs.some((cfg) => cfg === 'cfg(test)' || /^cfg\(all\((.*,)?test(,.*)?\)\)$/.test(cfg));

/** What a set of conditions leaves of the platforms: `all`, `desktop`,
 *  `mobile`, or `none` for code no build of the app holds. A feature does
 *  not choose a platform. `unknown` is a condition this script cannot read. */
function platform(cfgs) {
  if (testOnly(cfgs)) return { where: 'none' };
  let where = 'all';
  for (const cfg of cfgs) {
    let narrowed;
    if (cfg === 'cfg(desktop)') narrowed = 'desktop';
    else if (cfg === 'cfg(mobile)') narrowed = 'mobile';
    else if (/^cfg\(feature="[\w-]+"\)$/.test(cfg)) continue;
    else return { unknown: cfg };
    if (where !== 'all' && where !== narrowed) return { where: 'none' };
    where = narrowed;
  }
  return { where };
}

/** `text` cut at the commas that are inside no bracket. */
function splitTop(text) {
  const parts = [];
  let depth = 0;
  let from = 0;
  for (let i = 0; i < text.length; i++) {
    if (OPEN.includes(text[i])) depth++;
    else if (CLOSE.includes(text[i])) depth--;
    else if (text[i] === ',' && depth === 0) { parts.push(text.slice(from, i)); from = i + 1; }
  }
  parts.push(text.slice(from));
  return parts;
}

/** The files of one crate's `src` with what the scans below ask of them. */
function crate(root) {
  const files = new Map(walk(root, /\.rs$/).map((file) => [file, clean(read(file))]));
  const known = new Map();
  /** The conditions on the `mod` declarations that lead to `file`, or null
   *  when no declaration of it is found. */
  const cfgsOf = (file) => {
    if (known.has(file)) return known.get(file);
    known.set(file, null);
    const rel = path.relative(root, file);
    let result = null;
    if (rel === 'lib.rs' || rel === 'main.rs') {
      result = [];
    } else {
      const isMod = path.basename(file) === 'mod.rs';
      const dir = isMod ? path.dirname(path.dirname(file)) : path.dirname(file);
      const name = isMod ? path.basename(path.dirname(file)) : path.basename(file, '.rs');
      const owners = dir === root ? [path.join(root, 'lib.rs'), path.join(root, 'main.rs')] : [path.join(dir, 'mod.rs'), dir + '.rs'];
      for (const owner of owners) {
        const code = files.get(owner);
        const declared = code && new RegExp(`\\bmod\\s+${name}\\s*;`).exec(code);
        if (!declared) continue;
        const above = cfgsOf(owner);
        if (above) result = [...above, ...cfgsAt(code, declared.index)];
        break;
      }
    }
    known.set(file, result);
    return result;
  };
  return { files, cfgsOf };
}

/** Every command the `module!` lists under `roots` declare: its name, the id
 *  of its module and the platforms that have it. The first root is the
 *  product's; a root `<dir>=<crate>` is the crate of a module, whose
 *  `module()` in its `lib.rs` the product lists as `<crate>::module()`. */
function declared(roots, problems) {
  const found = [];
  const product = crate(roots[0]);
  for (const spec of roots) {
    const [root, ident] = spec.split('=');
    const { files, cfgsOf } = root === roots[0] ? product : crate(root);
    for (const [file, code] of files) {
      const call = /\bmodule\s*!\s*[({[]/g;
      let m;
      while ((m = call.exec(code))) {
        const open = m.index + m[0].length - 1;
        const body = code.slice(open + 1, matching(code, open));
        const here = `${file}:${lineOf(code, m.index)}`;
        const own = cfgsOf(file);
        const around = [...(own ?? []), ...cfgsAt(code, m.index)];
        if (platform(around).where === 'none') continue;
        if (!own) { problems.push(`${here}: no \`mod\` declaration leads to this file, so the platform of its commands cannot be told`); continue; }
        const id = /\bid\s*:\s*"(\w+)"/.exec(body)?.[1];
        const list = /\bcommands\s*:\s*\[/.exec(body);
        if (!id || !list) { problems.push(`${here}: a module! this script cannot read (id: "..", commands: [..])`); continue; }

        // A module the product lists on one platform has its commands there only.
        const isMod = path.basename(file) === 'mod.rs';
        const ofCrate = ident && path.relative(root, file) === 'lib.rs';
        const name = ofCrate ? ident : isMod ? path.basename(path.dirname(file)) : path.basename(file, '.rs');
        const listers = ofCrate ? product : { files, cfgsOf };
        const listings = [];
        for (const [other, text] of listers.files) {
          const use = new RegExp(`\\b${name}\\s*::\\s*module\\s*\\(\\s*\\)`, 'g');
          let u;
          while ((u = use.exec(text))) {
            const cfgs = [...(listers.cfgsOf(other) ?? []), ...cfgsAt(text, u.index)];
            if (platform(cfgs).where !== 'none') listings.push(cfgs);
          }
        }
        if (listings.length !== 1) {
          problems.push(`${here}: module \`${id}\`: \`${name}::module()\` is called ${listings.length} times; this script reads the platform of a module from the one place that lists it`);
          continue;
        }

        const listOpen = list.index + list[0].length - 1;
        for (const entry of splitTop(body.slice(listOpen + 1, matching(body, listOpen)))) {
          const attrs = attributes(entry);
          const last = attrs[attrs.length - 1];
          const item = entry.slice(last ? matching(entry, entry.indexOf('[', last.at)) + 1 : 0).replace(/\s+/g, '');
          if (!item) continue;
          if (!/^(\w+::)*\w+$/.test(item)) { problems.push(`${here}: module \`${id}\`: an entry this script cannot read: ${item}`); continue; }
          const conditional = attrs.filter((a) => a.text.startsWith('cfg'));
          const cfgs = [...around, ...listings[0], ...conditional.map((a) => a.text)];
          const p = platform(cfgs);
          const command = item.split('::').pop();
          if (p.unknown) problems.push(`${here}: command ${command}: this script cannot tell the platform from #[${p.unknown}]`);
          else if (p.where !== 'none') found.push({ command, module: id, where: p.where, at: here });
        }
      }
    }
  }
  return found;
}

function commands(golden, roots) {
  const problems = [];
  const found = declared(roots, problems);
  const byName = new Map();
  for (const one of found) {
    const first = byName.get(one.command);
    if (first) problems.push(`command ${one.command} is listed twice: by \`${first.module}\` (${first.at}) and by \`${one.module}\` (${one.at})`);
    else byName.set(one.command, one);
  }
  const wanted = new Set();
  for (const line of read(golden).split('\n').filter(Boolean)) {
    const [command, module, where] = line.split(' ');
    wanted.add(command);
    const one = byName.get(command);
    if (!one) problems.push(`command ${command}: in ${golden}, in no module! list`);
    else if (one.module !== module) problems.push(`command ${command}: ${golden} gives it to \`${module}\`, the module! list at ${one.at} to \`${one.module}\``);
    else if (one.where !== where) problems.push(`command ${command}: ${golden} says \`${where}\`, the module! list at ${one.at} makes it \`${one.where}\``);
  }
  for (const one of byName.values()) {
    if (!wanted.has(one.command)) problems.push(`command ${one.command} (${one.at}): not in ${golden}`);
  }
  return problems;
}

function rules(kind, pkg, dir, buildScript, tauri) {
  const problems = [];
  const script = buildScript !== '-' ? clean(read(buildScript)) : '';
  const helper = /\bveydan_build_cfg\s*::\s*platform_cfg\s*\(/.test(script);
  // The test executable of a library that links Tauri does not start on
  // Windows without the application manifest; a product gets its manifest
  // from tauri-build, and a second one would break the link of its binary.
  const testManifest = /\bveydan_build_cfg\s*::\s*windows_test_manifest\s*\(/.test(script);
  if (kind === 'product') {
    if (testManifest) problems.push(`${buildScript}: ${pkg} is a product and calls veydan_build_cfg::windows_test_manifest(); its binary has the manifest of tauri-build`);
  } else if (tauri === 'yes' && !testManifest) {
    problems.push(`${buildScript === '-' ? dir : buildScript}: ${pkg} depends on Tauri and has no build script calling veydan_build_cfg::windows_test_manifest(); its tests would not start on Windows`);
  }
  let platformCfg = null;
  for (const file of walk(dir, /\.rs$/)) {
    const code = withoutStrings(clean(read(file)));
    const attrs = attributes(code);
    // The name of a command is the last segment of its path: the list of
    // names and the handler of a module come from one list of paths.
    for (const attr of attrs) {
      if (/\bcommand\((?:[^()]|\([^()]*\))*?\brename=/.test(attr.text)) {
        problems.push(`${file}:${lineOf(code, attr.at)}: #[command(rename = ..)] is not allowed`);
      }
    }
    const alias = /\buse\b[^;]*\bcommand\s+as\s+\w+/.exec(code);
    if (alias) problems.push(`${file}:${lineOf(code, alias.index)}: the attribute \`command\` is imported under another name; a rename behind it would not be seen`);

    if ((kind === 'lib' || kind === 'module') && !helper && !platformCfg) {
      const uses = attrs.filter((a) => /^cfg(_attr)?\(/.test(a.text) && /\b(desktop|mobile)\b/.test(a.text)).map((a) => a.at);
      const macro = /\bcfg\s*!\s*\(/g;
      let m;
      while ((m = macro.exec(code))) {
        const open = m.index + m[0].length - 1;
        if (/\b(desktop|mobile)\b/.test(code.slice(open, matching(code, open)))) uses.push(m.index);
      }
      if (uses.length) platformCfg = `${file}:${lineOf(code, Math.min(...uses))}`;
    }

    if (kind === 'product') {
      const manifest = /\bAppManifest\b|\bapp_manifest\s*\(/.exec(code);
      if (manifest) problems.push(`${file}:${lineOf(code, manifest.index)}: a product does not hand an app manifest to tauri-build; with one every command of a module needs a capability entry`);
    }
  }
  // In a plain library cfg(desktop) is false on every platform unless its
  // build script asks the helper.
  if (platformCfg) problems.push(`${platformCfg}: ${pkg} uses cfg(desktop) or cfg(mobile) and has no build script calling veydan_build_cfg::platform_cfg()`);
  return problems;
}

/** The string literals of a crate's `src` that a build of the app holds:
 *  those of tests, inline or in a file a `#[cfg(test)] mod` leads to, aside. */
function builtLiterals(root) {
  const { files, cfgsOf } = crate(root);
  const found = [];
  for (const [file, code] of files) {
    if (testOnly(cfgsOf(file) ?? [])) continue;
    const src = read(file);
    for (const lit of literals(src)) {
      if (!testOnly(cfgsAt(code, lit.at))) found.push({ file, src, ...lit });
    }
  }
  return found;
}

/** The tables the `CREATE TABLE` statements of these literals create. */
function created(lits) {
  const names = new Set();
  for (const lit of lits) {
    for (const m of lit.text.matchAll(/\bCREATE\s+(?:VIRTUAL\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(\w+)/gi)) names.add(m[1]);
  }
  return names;
}

/** The SQL of a crate names only the tables its own schema creates (6.2,
 *  item 8; 6.4): what another module keeps is reached through the entity
 *  directory and the deletion hooks. A library may also name the tables of
 *  the directories given with `+` (the core's). Read from the string
 *  literals a build of the app holds, tests aside: a name after FROM, JOIN,
 *  INTO, UPDATE or TABLE, in any case in a literal that begins with a
 *  statement and in capitals in any other — there only the tables another
 *  crate creates count, so prose (a demo note on SQL) passes. A name a
 *  WITH clause gives, column list or not, is the statement's own. A name
 *  `format!` puts in (`{table}`) is read from the literals of the same file:
 *  none of them may be exactly the name of a table another directory
 *  creates.
 *
 *  The internal modules of a product are not crates yet (5.10): there only
 *  the tables `others` names are foreign, and every other name passes. */
function tables(kind, dir, others) {
  const problems = [];
  const lits = builtLiterals(dir);
  const own = created(lits);
  const statement = /^\s*(?:SELECT|INSERT|UPDATE|DELETE|CREATE|WITH|REPLACE|ALTER|DROP)\b/i;
  // A module that keeps nothing in the data file (the messenger has a
  // database of its own) creates no table, and has no SQL to name one.
  if (kind === 'module' && own.size === 0 && lits.some((lit) => statement.test(lit.text))) {
    problems.push(`${dir}: SQL and no CREATE TABLE found; the crate of a module creates the tables it keeps`);
  }
  const allowed = new Set([...own, 'sqlite_master', 'sqlite_schema', 'sqlite_sequence']);
  const foreign = new Set();
  if (kind === 'product') {
    allowed.clear();
    for (const name of others) foreign.add(name);
  }
  for (const other of kind === 'product' ? [] : others) {
    const shared = other.startsWith('+');
    for (const name of created(builtLiterals(shared ? other.slice(1) : other))) {
      if (shared) allowed.add(name);
      else foreign.add(name);
    }
  }
  for (const name of allowed) foreign.delete(name);

  const keyword = /\b(FROM|JOIN|INTO|UPDATE|TABLE)\s+(?:IF\s+(?:NOT\s+)?EXISTS\s+)?(\{|\w+)(\s*\()?/gi;
  const interpolated = new Set();
  for (const lit of lits) {
    const sql = statement.test(lit.text);
    // The names a WITH clause gives its own tables.
    const named = new Set([...lit.text.matchAll(/(?:\bWITH\s+(?:RECURSIVE\s+)?|,\s*)(\w+)\s*(?:\([\w\s,]*\)\s*)?AS\s*\(/gi)].map((m) => m[1]));
    for (const m of lit.text.matchAll(keyword)) {
      const [, word, name, call] = m;
      if (!sql && word !== word.toUpperCase()) continue;
      if (name === '{') { interpolated.add(lit.file); continue; }
      // `DO UPDATE SET` of an upsert; a table-valued function such as json_each(..)
      if (/^set$/i.test(name) || (call && /^(FROM|JOIN)$/i.test(word))) continue;
      // Outside a statement only a table another crate creates is taken for
      // SQL: capitals in prose (a demo note on `ALTER TABLE accounts`) are not.
      if (!sql && !foreign.has(name)) continue;
      const stranger = kind === 'product' ? foreign.has(name) : !allowed.has(name);
      if (stranger && !named.has(name)) problems.push(`${lit.file}:${lineOf(lit.src, lit.at)}: SQL names the table ${name}, which this crate does not create`);
    }
  }
  for (const lit of lits) {
    if (interpolated.has(lit.file) && foreign.has(lit.text)) {
      problems.push(`${lit.file}:${lineOf(lit.src, lit.at)}: the table ${lit.text}, which this crate does not create, is named in a file that puts table names into its SQL`);
    }
  }
  return problems;
}

// ── UI ──────────────────────────────────────────────────────────────────────

function imports(dir) {
  const found = [];
  const keepLines = (s) => s.replace(/[^\n]/g, ' ');
  for (const file of walk(dir, /\.(ts|js|svelte)$/)) {
    const text = read(file)
      .replace(/\/\*[\s\S]*?\*\//g, keepLines)
      .replace(/<!--[\s\S]*?-->/g, keepLines)
      .replace(/^[ \t]*\/\/.*$/gm, keepLines);
    const take = (pattern, what) => {
      let m;
      while ((m = pattern.exec(text))) found.push(`${file}:${lineOf(text, m.index)}\t${what ?? m[1]}`);
    };
    // import .. from, import type .. from, export .. from, over any number of lines
    take(/\b(?:import|export)\b[^;'"`()=]*?\bfrom\s*['"]([^'"\n]+)['"]/g);
    // import of a file for what it does, @import of a style
    take(/\bimport\s*['"]([^'"\n]+)['"]/g);
    // import() of a literal; a template with `${…}`, or a literal that more is
    // joined to (`import('$lib/' + name)`), is a computed path
    let m;
    const dynamic = /(?<![.\w$])import\s*\(\s*(['"`])([^'"`\n]*)\1\s*(\)?)/g;
    while ((m = dynamic.exec(text))) {
      const literal = m[3] && !(m[1] === '`' && m[2].includes('${'));
      found.push(`${file}:${lineOf(text, m.index)}\t${literal ? m[2] : '(computed)'}`);
    }
    take(/(?<![.\w$])import\s*\(\s*(?!['"`\s])/g, '(computed)');
  }
  return found;
}

/**
 * The matrix of the frontend (platform-spec 6.3). The owner of a file is its
 * directory: `ui/src/lib/core`, `ui/src/lib/<module>` (a module of
 * products.json), a module's route directory (the manifest's `routes`), or a
 * shared route. The UI project is ui/ beside products.json.
 *
 *  - core imports itself, the virtual modules, Svelte, `$app/*`,
 *    `@tauri-apps/*` and packages — never a module, not even a type;
 *  - a module imports itself, `$lib/core/**` and packages; the messenger
 *    only `$lib/core/ui/*`, `$lib/core/i18n`, `$lib/core/Icon.svelte` and
 *    `$lib/core/module` of the core, and `svelte`, `$app/*`,
 *    `@tauri-apps/*`, `vitest` from outside;
 *  - a module's route files import that module and the core; the shared
 *    routes (the layouts, `/`, `/settings`) the core alone.
 *
 * Every form the `imports` scan reads counts, imports of types included;
 * an `import()` of a computed path cannot be checked and is an error.
 */
function ui(manifestFile) {
  const manifest = JSON.parse(read(manifestFile));
  const root = path.dirname(path.resolve(manifestFile));
  // The UI project, the root of Vite: `$lib`, `/src/…` and `src/…` resolve from it.
  const project = path.join(root, 'ui');
  const modules = Object.keys(manifest.modules);
  const routeOwner = [];
  for (const id of modules) for (const r of manifest.modules[id].routes) routeOwner.push([path.join(root, r) + path.sep, id]);
  const uiDir = (id) => path.join(root, manifest.modules[id].ui);

  /** The owner of a file: 'core', a module id, or a shared route ('shared'). */
  function ownerOf(file) {
    const abs = path.resolve(file);
    if (abs.startsWith(path.join(project, 'src/lib/core') + path.sep)) return 'core';
    for (const id of modules) if (abs.startsWith(uiDir(id) + path.sep)) return id;
    for (const [dir, id] of routeOwner) if (abs.startsWith(dir)) return `routes:${id}`;
    return 'shared';
  }
  /** The owner a specifier points at, or null for a package or a file outside ui/src/ (the build tooling). */
  function target(file, spec) {
    let abs;
    if (spec.startsWith('$lib/')) abs = path.join(project, 'src/lib', spec.slice(5));
    else if (spec.startsWith('.')) abs = path.resolve(path.dirname(file), spec);
    // Vite resolves `/src/…` from the project's root (and an absolute path as
    // it is); a bare `src/…` names the same files
    else if (spec.startsWith(root + path.sep)) abs = spec;
    else if (spec.startsWith('/') || spec.startsWith('src/')) abs = path.join(project, spec);
    else return null;
    if (!abs.startsWith(path.join(project, 'src') + path.sep)) return null;
    if (abs.startsWith(path.join(project, 'src/lib/core') + path.sep) || abs === path.join(project, 'src/lib/core')) return { owner: 'core', rel: path.relative(path.join(project, 'src/lib/core'), abs).split(path.sep).join('/') };
    for (const id of modules) if (abs.startsWith(uiDir(id) + path.sep) || abs === uiDir(id)) return { owner: id, rel: path.relative(uiDir(id), abs).split(path.sep).join('/') };
    for (const [dir, id] of routeOwner) if (abs.startsWith(dir)) return { owner: `routes:${id}`, rel: '' };
    return { owner: 'shared', rel: '' };
  }
  const external = (spec) => spec === 'svelte' || spec.startsWith('svelte/') || spec.startsWith('$app/') || spec.startsWith('@tauri-apps/') || spec.startsWith('virtual:veydan-modules');
  const messengerCore = (rel) => rel.startsWith('ui/') || ['i18n', 'i18n.ts', 'Icon.svelte', 'module', 'module.ts'].includes(rel);

  const problems = [];
  let seen = 0;
  for (const line of imports(path.relative(process.cwd(), path.join(project, 'src')) || '.')) {
    seen++;
    const [where, spec] = line.split('\t');
    const file = where.replace(/:\d+$/, '');
    if (spec === '(computed)') { problems.push(`${where}: import() of a path that is not a literal; the matrix cannot be checked`); continue; }
    const from = ownerOf(file);
    const to = target(file, spec);
    const say = (why) => problems.push(`${where}: imports '${spec}' — ${why}`);
    if (to === null) {
      // A package. The messenger's list of allowed packages is closed.
      if (from === 'messenger' && !external(spec) && spec !== 'vitest') say('the messenger UI may import only svelte, $app/*, @tauri-apps/* and vitest from outside');
      continue;
    }
    if (to.owner === from) continue;
    if (from === 'core') {
      say(`the core may not import a module (${to.owner}); modules reach the core through the registry and the catalog`);
    } else if (modules.includes(from)) {
      if (to.owner !== 'core') say(`module ${from} may import only itself and $lib/core; ${to.owner} is another module`);
      else if (from === 'messenger' && !messengerCore(to.rel)) say('the messenger UI may import from the core only $lib/core/ui/*, $lib/core/i18n, $lib/core/Icon.svelte and $lib/core/module');
    } else if (from.startsWith('routes:')) {
      const mod = from.slice('routes:'.length);
      if (to.owner !== 'core' && to.owner !== mod) say(`the routes of ${mod} may import only $lib/${mod} and $lib/core`);
    } else if (to.owner !== 'core') {
      say(`a shared route may import only $lib/core; ${to.owner} is a module`);
    }
  }
  if (seen === 0) problems.push('ui/src: no import found; the scan of the UI reads nothing');
  return problems;
}

/**
 * A published snapshot of one product (scripts/publish.sh, platform-spec
 * 14.4) holds nothing of a module the product lacks and nothing of another
 * target: no file under the UI folder, the routes or the crates of such a
 * module, under another product's crate or its extra paths, under a folder
 * of the infrastructure — and no import, of a type included, that points
 * into the UI folder or the routes of such a module, whether or not the
 * file is there. The folders come from the manifest of the monorepo, which
 * the snapshot does not have; the imports are read by the scan `imports`.
 */
function foreign(manifestFile, name, root) {
  const manifest = JSON.parse(read(manifestFile));
  const product = manifest.targets[name];
  if (!product || product.kind !== 'product') throw new Error(`${name}: not a product of ${manifestFile}`);
  root = path.resolve(root);
  const own = new Set(product.modules);
  /** [folder relative to the root, whose it is, whether an import may point into it] */
  const folders = [];
  for (const [id, m] of Object.entries(manifest.modules)) {
    if (own.has(id)) continue;
    for (const dir of [m.ui, ...m.routes]) folders.push([dir, `the module ${id}`, true]);
    for (const dir of m.crates) folders.push([dir, `the module ${id}`, false]);
  }
  for (const [other, t] of Object.entries(manifest.targets)) {
    if (other === name) continue;
    for (const dir of [t.app, ...(t.paths ?? []), ...(t.extra ?? [])]) if (dir) folders.push([dir, `the target ${other}`, false]);
  }
  // What the product itself is built from is never foreign: a crate two modules share, an extra path two products have.
  const mine = [product.app, ...(product.extra ?? []), ...product.modules.flatMap((id) => [manifest.modules[id].ui, ...manifest.modules[id].routes, ...manifest.modules[id].crates])];
  const inside = (rel, dir) => rel === dir || rel.startsWith(dir + '/');
  const ownerOf = (rel, importable) => {
    if (mine.some((dir) => inside(rel, dir))) return null;
    const hit = folders.find(([dir, , imp]) => (!importable || imp) && inside(rel, dir));
    return hit ? hit : null;
  };

  const problems = [];
  const counted = new Map();
  const visit = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      if (['.git', 'node_modules', 'target'].includes(entry.name)) continue;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) { visit(full); continue; }
      const rel = path.relative(root, full).split(path.sep).join('/');
      const hit = ownerOf(rel, false);
      if (!hit) continue;
      const seen = counted.get(hit[0]);
      if (seen) seen.more++;
      else counted.set(hit[0], { file: rel, whose: hit[1], more: 0 });
    }
  };
  visit(root);
  for (const [dir, { file, whose, more }] of counted) {
    problems.push(`${file}: a file of ${whose} (${dir}${more ? `, and ${more} more there` : ''}); ${name} does not have it`);
  }

  const project = path.join(root, 'ui');
  const src = path.join(project, 'src');
  if (!fs.existsSync(src)) return [...problems, 'ui/src: the snapshot has no UI; the scan of the imports reads nothing'];
  let seen = 0;
  for (const line of imports(src)) {
    seen++;
    const [where, spec] = line.split('\t');
    const file = where.replace(/:\d+$/, '');
    let abs;
    if (spec.startsWith('$lib/')) abs = path.join(project, 'src/lib', spec.slice(5));
    else if (spec.startsWith('.')) abs = path.resolve(path.dirname(file), spec);
    else if (spec.startsWith(root + path.sep)) abs = spec;
    else if (spec.startsWith('/') || spec.startsWith('src/')) abs = path.join(project, spec);
    else continue;
    const hit = ownerOf(path.relative(root, abs).split(path.sep).join('/'), true);
    if (hit) problems.push(`${path.relative(root, where)}: imports '${spec}' — a file of ${hit[1]}; ${name} does not have it`);
  }
  if (seen === 0) problems.push('ui/src: no import found; the scan of the imports reads nothing');
  return problems;
}

/**
 * The UI scan against planted imports (6.4, p. 7): a scratch tree in the
 * temporary directory, one import per file, every form the scan must
 * catch and the look-alikes it must let through. Prints what it got wrong.
 */
function uiSelftest() {
  const bad = {
    'ui/src/lib/core/static.ts': "import { p } from '$lib/pass/store';",
    'ui/src/lib/core/type-lines.ts': "import type {\n  A,\n  B,\n} from '$lib/notes/types';",
    'ui/src/lib/core/side-effect.ts': "import '$lib/notes/setup';",
    'ui/src/lib/core/reexport.ts': "export { p } from '$lib/pass/store';",
    'ui/src/lib/core/root-absolute.ts': "import { p } from '/src/lib/pass/store.svelte';",
    'ui/src/lib/core/bare-src.ts': "import { p } from 'src/lib/pass/store.svelte';",
    'ui/src/lib/core/dynamic.ts': "export const m = () => import('$lib/notes/entry/desktop');",
    'ui/src/lib/core/dynamic-joined.ts': "export const m = (n: string) => import('$lib/' + n + '/store/passwords.svelte.ts');",
    'ui/src/lib/core/dynamic-template.ts': 'export const m = (n: string) => import(`$lib/${n}/store`);',
    'ui/src/lib/core/dynamic-name.ts': 'export const m = (n: string) => import(n);',
    'ui/src/lib/notes/relative.ts': "import { p } from '../pass/store';",
    'ui/src/lib/notes/Other.svelte': "<script lang=\"ts\">\n  import { p } from '$lib/pass/store';\n</script>",
    'ui/src/lib/messenger/core-api.ts': "import { call } from '$lib/core/api';",
    'ui/src/lib/messenger/package.ts': "import DOMPurify from 'dompurify';",
    'ui/src/routes/notes/+page.svelte': "<script>\n  import P from '$lib/pass/P.svelte';\n</script>",
    'ui/src/routes/settings/+page.svelte': "<script>\n  import N from '$lib/notes/N.svelte';\n</script>",
  };
  const good = {
    'ui/src/lib/core/ok.ts': "import { writable } from 'svelte/store';\nimport { a } from './a';\nexport const l = () => import('$lib/core/lazy');\nimport type { ModuleDef } from '$lib/core/module';\nimport { modules } from 'virtual:veydan-modules/desktop';",
    'ui/src/lib/core/comments.ts': "// import { p } from '$lib/pass/store';\n/* import('$lib/pass/store') */\nexport const s = 1;",
    'ui/src/lib/notes/ok.ts': "import { call } from '$lib/core/api';\nexport const l = () => import('./lazy');\nimport { n } from '/src/lib/notes/n';",
    'ui/src/lib/messenger/ok.ts': "import { t } from '$lib/core/i18n';\nimport Icon from '$lib/core/Icon.svelte';\nimport { goto } from '$app/navigation';",
    'ui/src/routes/notes/[id]/+page.svelte': "<script>\n  import N from '$lib/notes/N.svelte';\n  import { t } from '$lib/core/i18n';\n</script>",
  };
  const computed = ['ui/src/lib/core/dynamic-joined.ts', 'ui/src/lib/core/dynamic-template.ts', 'ui/src/lib/core/dynamic-name.ts'];
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'boundaries-ui-'));
  try {
    const manifest = {
      modules: {
        notes: { ui: 'ui/src/lib/notes', routes: ['ui/src/routes/notes'], crates: [] },
        pass: { ui: 'ui/src/lib/pass', routes: ['ui/src/routes/passwords'], crates: [] },
        messenger: { ui: 'ui/src/lib/messenger', routes: [], crates: [] },
      },
      targets: {},
    };
    fs.writeFileSync(path.join(dir, 'products.json'), JSON.stringify(manifest));
    for (const [file, text] of Object.entries({ ...bad, ...good })) {
      fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
      fs.writeFileSync(path.join(dir, file), text + '\n');
    }
    const reported = new Map();
    for (const line of ui(path.join(dir, 'products.json'))) {
      const file = path.relative(dir, path.resolve(line.split(':')[0])).split(path.sep).join('/');
      reported.set(file, line);
    }
    const problems = [];
    for (const file of Object.keys(bad)) if (!reported.has(file)) problems.push(`ui self-test: the planted import in ${file} was not reported`);
    for (const file of computed) if (!reported.get(file)?.includes('not a literal')) problems.push(`ui self-test: the import() in ${file} was not taken for a computed path`);
    for (const [file, line] of reported) if (!(file in bad)) problems.push(`ui self-test: a permitted import was reported: ${line}`);
    return problems;
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const [scan, ...args] = process.argv.slice(2);
let lines;
if (scan === 'commands' && args.length >= 2) lines = commands(args[0], args.slice(1));
else if (scan === 'rules' && args.length === 5) lines = rules(...args);
else if (scan === 'imports' && args.length === 1) lines = imports(args[0]);
else if (scan === 'ui' && args.length === 1) lines = ui(args[0]);
else if (scan === 'ui-selftest' && args.length === 0) lines = uiSelftest();
else if (scan === 'foreign' && args.length === 3) lines = foreign(...args);
else if (scan === 'tables' && args.length >= 2 && ['lib', 'module', 'product'].includes(args[0])) lines = tables(args[0], args[1], args.slice(2));
else {
  console.error('usage: boundaries.mjs commands <golden> <src> [<src>[=<crate>]].. | rules <kind> <package> <dir> <build script|-> <yes|no> | tables <lib|module> <src> [[+]<src>].. | tables product <src> <table>.. | imports <dir> | ui <products.json> | ui-selftest | foreign <products.json> <product> <root>');
  process.exit(2);
}
for (const line of lines) console.log(line);

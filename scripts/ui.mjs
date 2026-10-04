#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The UI launcher (docs/platform-spec.md 11.6):
//
//   node scripts/ui.mjs <product> [vitest|svelte-kit|svelte-check] <args…>
//
// Sets VEYDAN_PRODUCT — the one variable that selects the product —, makes
// the UI project (ui/) the working directory, provides the tsconfig a clean
// clone lacks (11.4), writes the types of the virtual modules for the
// product, and runs the tool's JS entry with this very node: no shell, no
// `VAR=x cmd` (that does not work in cmd.exe / PowerShell). Without a tool
// name the arguments go to vite. It runs from any directory.
//
//   node scripts/ui.mjs space dev
//   node scripts/ui.mjs notes build
//   node scripts/ui.mjs notes vitest run
//   node scripts/ui.mjs notes svelte-check        (runs `svelte-kit sync` first)

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// The SvelteKit project: package.json, node_modules, the configs, src/.
const ui = path.join(root, 'ui');
const require = createRequire(path.join(ui, 'package.json'));

const TOOLS = { vite: 'vite', vitest: 'vitest', 'svelte-kit': '@sveltejs/kit', 'svelte-check': 'svelte-check' };

const [product, ...rest] = process.argv.slice(2);
const { productFromEnv, productNames, virtualTypes, PRODUCT_ENV, KIT_DIR } = await import(
  pathToFileURL(path.join(ui, 'vite-veydan-modules.js')).href
);
const names = productNames();
if (!product || !names.includes(product)) {
  console.error(`usage: node scripts/ui.mjs <${names.join('|')}> [${Object.keys(TOOLS).slice(1).join('|')}] <args…>`);
  process.exit(2);
}
if (!fs.existsSync(path.join(ui, 'node_modules'))) {
  console.error('ui/node_modules is missing: run `pnpm --dir ui install --frozen-lockfile` first');
  process.exit(2);
}
const env = { ...process.env, [PRODUCT_ENV]: product };
const info = productFromEnv(env);

/** Writes a file atomically: a temporary file in the same directory, then a rename. */
function writeAtomic(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, text);
  fs.renameSync(tmp, file);
}

// ui/tsconfig.json extends ./.svelte-kit/tsconfig.json, and Vite reads
// that chain for every file it transforms; SvelteKit writes its tsconfig into
// the product's own directory, so a clean clone has none at that path. This
// one holds only the compiler options SvelteKit writes for any product — no
// paths to generated types — and is created only when missing, never
// rewritten: concurrent builds, dev servers and the editor never compete
// for it. Type checking does not use it: svelte-check always gets
// --tsconfig tsconfig.<product>.json.
const kitDir = path.join(ui, KIT_DIR);
const fromKit = (/** @type {string} */ p) => path.relative(kitDir, p).split(path.sep).join('/');
const sharedTsconfig = path.join(kitDir, 'tsconfig.json');
if (!fs.existsSync(sharedTsconfig)) {
  writeAtomic(
    sharedTsconfig,
    JSON.stringify(
      {
        compilerOptions: {
          paths: { $lib: [fromKit(path.join(ui, 'src/lib'))], '$lib/*': [fromKit(path.join(ui, 'src/lib')) + '/*'] },
          rootDirs: [fromKit(ui)],
          verbatimModuleSyntax: true,
          isolatedModules: true,
          lib: ['esnext', 'DOM', 'DOM.Iterable'],
          moduleResolution: 'bundler',
          module: 'esnext',
          noEmit: true,
          target: 'esnext',
        },
      },
      null,
      '\t',
    ) + '\n',
  );
}

// Types of virtual:veydan-modules/* for this product (included by
// kit.typescript.config in svelte.config.js). The same for both platforms.
writeAtomic(path.join(ui, info.typesFile), virtualTypes(info));

const tool = rest[0] in TOOLS ? rest.shift() : 'vite';
function binOf(name) {
  const pkgPath = require.resolve(`${TOOLS[name]}/package.json`);
  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  const binRel = typeof pkg.bin === 'string' ? pkg.bin : pkg.bin[name];
  return path.resolve(path.dirname(pkgPath), binRel);
}

if (tool === 'svelte-check') {
  // The generated types of the product's routes are what svelte-check reads.
  const sync = spawnSync(process.execPath, [binOf('svelte-kit'), 'sync'], { cwd: ui, stdio: 'inherit', env });
  if (sync.status !== 0) process.exit(sync.status ?? 1);
  if (!rest.includes('--tsconfig')) rest.unshift('--tsconfig', `./tsconfig.${product}.json`);
}

const child = spawn(process.execPath, [binOf(tool), ...rest], { cwd: ui, stdio: 'inherit', env });
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => child.kill(sig));
child.on('exit', (code, signal) => process.exit(signal ? 1 : (code ?? 1)));

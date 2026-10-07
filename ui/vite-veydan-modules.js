// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// One SvelteKit app, several products (internal/platform-spec.md 11.2–11.4).
//
// products.json says which modules a product has; VEYDAN_PRODUCT (set by
// scripts/ui.mjs) says which product is built. This file gives
//  - productFromEnv(): the product, its modules, output directories and ports;
//  - veydanModules(): a Vite plugin that
//      * generates `virtual:veydan-modules/i18n`, `/desktop` and `/mobile`,
//        which import only the entries of the product's modules, and
//        `/platform` and `/shell`: the modules and the shell of the build's
//        own platform, so the other platform's never enter the build;
//      * replaces the route files of an absent module with a redirect stub,
//        keeping the file id (a swap in resolveId breaks the SvelteKit build);
//      * turns any other import of an absent module into a build error that
//        names the product, the module and the importing file;
//      * gives the UI the product's own logo and favicon (`brand`).
//   Imports of types alone are erased before the build and pass here;
//   scripts/boundaries.sh and svelte-check catch those.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// This file lives in the UI project (ui/). products.json and the paths it
// holds — the UI folders and routes of the modules, the crates of the
// products — belong to the repository root, one level up.
const UI = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.dirname(UI);
const MANIFEST = path.join(ROOT, 'products.json');

export const PRODUCT_ENV = 'VEYDAN_PRODUCT';
/** SvelteKit's working folders, from the UI project (11.4). */
export const KIT_DIR = '.svelte-kit';
const VIRTUAL = 'virtual:veydan-modules';
const SLOTS = ['i18n', 'desktop', 'mobile'];
/**
 * The shell of each platform: `virtual:veydan-modules/shell` is the build's.
 * The UI imports `/shell` and `/platform`, never `isMobile ? import(a) :
 * import(b)`: the minifier folds two such imports into one preload call that
 * carries the CSS of one branch only (the phone UI preloaded the desktop
 * shell's CSS and got its own only from index.html), and SvelteKit writes the
 * CSS of every branch into the fallback index.html (the desktop window loaded
 * the phone's global stylesheet). What a build never imports cannot leak.
 */
const SHELLS = /** @type {const} */ ({
  desktop: path.join(UI, 'src/lib/core/desktop/DesktopShell.svelte'),
  android: path.join(UI, 'src/lib/core/mobile/MobileShell.svelte'),
});
/** The languages written in the dictionaries themselves (src/lib/core/i18n.ts). */
const LOCALES = /** @type {const} */ (['en', 'ru']);
/** Every language of the UI (src/lib/core/languages.json). */
const languageCodes = () =>
  JSON.parse(fs.readFileSync(path.join(UI, 'src/lib/core/languages.json'), 'utf8')).languages.map((/** @type {{ code: string }} */ l) => l.code);
/**
 * The pictures of the brand the UI shows by a fixed path (13.5): `/logo.png`
 * (the title bar, the top bar, About, the phone's home, hub and notes menu:
 * 18 to 64 CSS px) and `/favicon.ico` (src/app.html), each from `icons/` of
 * the product's app folder, the set scripts/icons.sh makes from its master.
 * 256 px stays sharp at 64 CSS px on a phone's 3x screen.
 */
const BRAND = /** @type {const} */ ({ 'logo.png': '128x128@2x.png', 'favicon.ico': 'icon.ico' });
/** What the UI serves as it is (Vite's public folder). */
const STATIC = path.join(UI, 'static');

const posix = (/** @type {string} */ p) => p.split(path.sep).join('/');
/** A path as the repository names it (messages, products.json). */
const rel = (/** @type {string} */ p) => posix(path.relative(ROOT, p));
/** A path from the UI project, the root of Vite and vitest. */
const relUi = (/** @type {string} */ p) => posix(path.relative(UI, p));

/**
 * @typedef {{ id: string, ui: string, routes: string[], entries: Record<string, string>, platforms: string[] }} ModuleEntry
 * @typedef {{
 *   name: string, title: string, tagline: { en: string, ru: string } & Record<string, string>, repo: string, platform: 'desktop' | 'android',
 *   present: ModuleEntry[], absent: ModuleEntry[], brand: Record<string, string>,
 *   outDir: string, pages: string, cacheDir: string, port: number, hmrPort: number, typesFile: string,
 * }} Product
 */

/** Every product of the manifest, by name. */
export function readManifest() {
  return JSON.parse(fs.readFileSync(MANIFEST, 'utf8'));
}

export function productNames() {
  const manifest = readManifest();
  return Object.keys(manifest.targets).filter((t) => manifest.targets[t].kind === 'product');
}

/** @param {NodeJS.ProcessEnv} [env] @returns {Product} */
export function productFromEnv(env = process.env) {
  const manifest = readManifest();
  const name = env[PRODUCT_ENV];
  const names = productNames();
  if (!name || !names.includes(name)) {
    throw new Error(
      `${PRODUCT_ENV}=${name ?? '<unset>'}: not a product of ${rel(MANIFEST)} (have: ${names.join(', ')}). ` +
        `Run through \`node scripts/ui.mjs <product> …\`.`,
    );
  }
  const target = manifest.targets[name];
  /** @type {'desktop' | 'android'} */
  const platform = env.TAURI_ENV_PLATFORM === 'android' ? 'android' : 'desktop';
  /** @type {ModuleEntry[]} */
  const all = Object.entries(manifest.modules).map(([id, m]) => {
    const ui = path.resolve(ROOT, m.ui);
    /** @type {Record<string, string>} */
    const entries = {};
    for (const slot of SLOTS) {
      const file = path.join(ui, 'entry', `${slot}.ts`);
      if (fs.existsSync(file)) entries[slot] = file;
    }
    return {
      id,
      ui,
      routes: m.routes.map((/** @type {string} */ r) => path.resolve(ROOT, r)),
      entries,
      platforms: m.platforms ?? ['desktop', 'android'],
    };
  });
  for (const id of target.modules) {
    if (!manifest.modules[id]) throw new Error(`product ${name}: unknown module "${id}" in ${rel(MANIFEST)}`);
  }
  // A module is in a build when the product has it and it exists on the platform
  // (browser and ssh are desktop-only, platform-spec 1.2).
  const inBuild = (/** @type {ModuleEntry} */ m) => target.modules.includes(m.id) && m.platforms.includes(platform);
  const port = platform === 'android' ? target.androidDevPort : target.devPort;
  // The name the user reads: productName of the product's Tauri config (13.1),
  // the Android config's where it has one (Space is "Veydan" on a phone).
  const productName = (/** @type {string} */ file) => {
    const conf = path.resolve(ROOT, target.app, file);
    return fs.existsSync(conf) ? JSON.parse(fs.readFileSync(conf, 'utf8')).productName : undefined;
  };
  const title = (platform === 'android' && productName('tauri.android.conf.json')) || productName('tauri.conf.json');
  if (typeof title !== 'string' || !title) {
    throw new Error(`product ${name}: no productName in ${target.app}/tauri.conf.json; the UI names the product by it (13.5)`);
  }
  // The line under the name in About (13.5): English and Russian always, the
  // other languages of the UI where products.json has them (English stands in).
  const tagline = target.tagline ?? {};
  for (const lang of LOCALES) {
    if (typeof tagline[lang] !== 'string' || !tagline[lang]) {
      throw new Error(`product ${name}: no "tagline.${lang}" in ${rel(MANIFEST)} (13.5)`);
    }
  }
  const codes = languageCodes();
  for (const [lang, text] of Object.entries(tagline)) {
    if (!codes.includes(lang) || typeof text !== 'string' || !text) {
      throw new Error(`product ${name}: "tagline.${lang}" in ${rel(MANIFEST)} is not a language of the UI with a text (13.5)`);
    }
  }
  // The product's own repository (products.json `repo`): its releases, its
  // licence files, the page the About card and the updater banner open.
  if (typeof target.repo !== 'string' || !/^[\w.-]+\/[\w.-]+$/.test(target.repo)) {
    throw new Error(`product ${name}: no "repo" (owner/name) in ${rel(MANIFEST)} (1.3)`);
  }
  return {
    name,
    title,
    tagline,
    repo: `https://github.com/${target.repo}`,
    platform,
    present: all.filter(inBuild),
    absent: all.filter((m) => !inBuild(m)),
    brand: brandOf(name, path.resolve(ROOT, target.app, 'icons')),
    // From the UI project, the working directory of every tool
    // (scripts/ui.mjs). What the build makes goes to data/ of the
    // repository root: the pages of the desktop UI to data/build/<product>,
    // where frontendDist of the product's tauri.conf.json points, those of
    // the phone UI to data/build/<product>-android (frontendDist of its
    // tauri.android.conf.json) — a desktop binary never finds the phone's
    // pages in its folder —, and Vite's caches to
    // data/vite/<product>-<platform>. SvelteKit's own folder stays in the
    // project: its server output imports @sveltejs/kit, which Node finds only
    // in a node_modules/ above it (internal/platform-spec.md 11.4).
    outDir: `${KIT_DIR}/${name}-${platform}`,
    pages: platform === 'android' ? `../data/build/${name}-android` : `../data/build/${name}`,
    cacheDir: `../data/vite/${name}-${platform}`,
    port,
    hmrPort: port + 1,
    typesFile: `${KIT_DIR}/veydan-modules.${name}.d.ts`,
  };
}

/**
 * The product's logo and favicon: the path the UI asks for → the file of the
 * product that answers it. ui/static/ holds Space's (its master app-icon.png,
 * 1024 px, and its icons/icon.ico): the product whose icon.ico is the static
 * favicon keeps the static files as they are, and its map is empty. Every
 * other product serves its own; one without them does not build — it would
 * show another product's shield.
 * @param {string} name @param {string} icons the product's icons/ folder
 * @returns {Record<string, string>}
 */
function brandOf(name, icons) {
  /** @type {Record<string, string>} */
  const brand = {};
  for (const [to, from] of Object.entries(BRAND)) {
    const file = path.join(icons, from);
    if (!fs.existsSync(file)) {
      throw new Error(`product ${name}: no ${rel(file)}; the UI shows it as /${to} (13.5). scripts/icons.sh makes it.`);
    }
    brand[to] = file;
  }
  const favicon = path.join(STATIC, 'favicon.ico');
  if (fs.existsSync(favicon) && fs.readFileSync(favicon).equals(fs.readFileSync(brand['favicon.ico']))) return {};
  return brand;
}

/**
 * What the UI knows of its product (src/lib/core/product.ts, 13.5): its id,
 * the name the user reads, the tagline of About and its repository. The plugin defines them
 * for every build, dev server and test run, so they are never missing:
 * productFromEnv() refuses a product without them.
 * @param {Product} product
 */
export function productDefines(product) {
  return {
    __VEYDAN_PRODUCT__: JSON.stringify(product.name),
    __VEYDAN_PRODUCT_NAME__: JSON.stringify(product.title),
    __VEYDAN_PRODUCT_TAGLINE__: JSON.stringify(product.tagline),
    __VEYDAN_PRODUCT_REPO__: JSON.stringify(product.repo),
  };
}

/** @param {string} file @param {string} dir */
function inside(file, dir) {
  return file === dir || file.startsWith(dir + path.sep) || file.startsWith(posix(dir) + '/');
}

// Stands in for every +page.ts / +layout.ts of a module the product does not have.
const STUB_LOAD = [
  "import { redirect } from '@sveltejs/kit';",
  'export function load() {',
  "  redirect(307, '/');",
  '}',
  '',
].join('\n');

// Stands in for every +page.svelte / +layout.svelte of such a module. No <style>
// block: the real file's styles are read from disk past this plugin.
const STUB_COMPONENT = [
  '<script lang="ts">',
  "  import { onMount } from 'svelte';",
  "  import { goto } from '$app/navigation';",
  "  onMount(() => { goto('/', { replaceState: true }); });",
  '</script>',
  '',
].join('\n');

/** The code of a virtual module (`virtual:veydan-modules/<slot>`), or null for an unknown one. @param {Product} product @param {string} id */
export function virtualCode(product, id) {
  const slot = id.slice(VIRTUAL.length + 1);
  if (slot === 'shell') return `export { default } from ${JSON.stringify(posix(SHELLS[product.platform]))};\n`;
  if (slot === 'platform') return virtualCode(product, `${VIRTUAL}/${product.platform === 'android' ? 'mobile' : 'desktop'}`);
  if (!SLOTS.includes(slot)) return null;
  const mods = product.present.filter((m) => m.entries[slot]);
  const imports = mods.map((m) => `import * as ${m.id} from ${JSON.stringify(posix(m.entries[slot]))};`);
  if (slot === 'i18n') {
    // One merged dictionary per locale, and the phone's layer the same way; an
    // absent module simply has no keys.
    const merged = (/** @type {string} */ at) => mods.map((m) => `...${m.id}.translations${at}`).join(', ');
    return [
      ...imports,
      'export const translations = {',
      `  en: { ${merged('.en')} },`,
      `  ru: { ${merged('.ru')} },`,
      `  mobile: { en: { ${merged('.mobile.en')} }, ru: { ${merged('.mobile.ru')} } },`,
      '};',
      // The files of the other languages, fetched one language at a time (core/i18n.ts).
      `export const locales = [${mods.map((m) => `${m.id}.locales`).join(', ')}];`,
    ].join('\n');
  }
  return [...imports, `export const modules = { ${mods.map((m) => m.id).join(', ')} };`].join('\n');
}

/**
 * Types of the virtual modules for svelte-check and the editor: written by
 * scripts/ui.mjs for each product (the file names the product's modules).
 * The name differs from this plugin's file name on purpose.
 * @param {Product} product
 */
export function virtualTypes(product) {
  const dir = path.dirname(path.resolve(UI, product.typesFile));
  const from = (/** @type {string} */ file) => JSON.stringify(posix(path.relative(dir, file)).replace(/\.ts$/, ''));
  const mods = (/** @type {string} */ slot) => product.present.filter((m) => m.entries[slot]);
  const lines = [
    `// Generated by scripts/ui.mjs for product "${product.name}"; do not edit.`,
    '// Types of the virtual modules of vite-veydan-modules.js.',
    '',
    `declare module 'virtual:veydan-modules/i18n' {`,
    '  export const translations: {',
  ];
  const dict = (/** @type {string} */ at, /** @type {string} */ indent) => {
    for (const locale of ['en', 'ru']) {
      const parts = mods('i18n').map((m) => `(typeof import(${from(m.entries.i18n)}))['translations']${at}['${locale}']`);
      lines.push(`${indent}${locale}: ${parts.length ? parts.join(' & ') : 'Record<never, string>'};`);
    }
  };
  dict('', '    ');
  lines.push('    mobile: {');
  dict("['mobile']", '      ');
  lines.push(
    '    };',
    '  };',
    '  export const locales: Record<string, () => Promise<{ desktop: Record<string, string>; mobile: Record<string, string> }>>[];',
    '}',
    '',
  );
  for (const slot of ['desktop', 'mobile']) {
    lines.push(`declare module 'virtual:veydan-modules/${slot}' {`, '  export const modules: {');
    for (const m of mods(slot)) lines.push(`    ${m.id}: typeof import(${from(m.entries[slot])});`);
    lines.push('  };', '}', '');
  }
  // One file serves both platforms: the build's own slot is either of them.
  lines.push(
    `declare module 'virtual:veydan-modules/platform' {`,
    "  export const modules: typeof import('virtual:veydan-modules/desktop')['modules'] | typeof import('virtual:veydan-modules/mobile')['modules'];",
    '}',
    '',
    `declare module 'virtual:veydan-modules/shell' {`,
    "  const Shell: import('svelte').Component<{ children: import('svelte').Snippet }>;",
    '  export default Shell;',
    '}',
    '',
  );
  return lines.join('\n');
}

/** @param {Product} product @returns {import('vite').Plugin} */
export function veydanModules(product) {
  /** @param {string} file @returns {{ module: ModuleEntry, stub: string } | null} */
  function absentRoute(file) {
    for (const m of product.absent) {
      if (!m.routes.some((r) => inside(file, r))) continue;
      const base = path.basename(file);
      if (/^\+(page|layout|error)\.svelte$/.test(base)) return { module: m, stub: STUB_COMPONENT };
      if (/^\+(page|layout)\.(ts|js)$/.test(base)) return { module: m, stub: STUB_LOAD };
      // +server.ts, +page.server.ts and the like have no meaning in this SPA; fail loudly.
      throw new Error(`[veydan-modules] no stub for ${rel(file)} of absent module "${m.id}"`);
    }
    return null;
  }

  // Hook filters: without them the hooks run for every import of the build and
  // take more than half of the plugins' time. An import reaches an absent
  // module only through a path naming its directory (`$lib` is expanded
  // before a `pre` plugin sees the resolved id).
  const esc = (/** @type {string} */ t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const names = [...new Set(product.absent.flatMap((m) => [m.ui, ...m.routes]).map((d) => esc(path.basename(d))))];
  const resolveFilter = { id: new RegExp(`^${esc(VIRTUAL)}` + (names.length ? `|[\\\\/](${names.join('|')})([\\\\/?]|$)` : '')) };
  const loadFilter = { id: new RegExp(`^\\0${esc(VIRTUAL)}` + (names.length ? `|[\\\\/](${names.join('|')})[\\\\/]` : '')) };

  return {
    name: 'veydan-modules',
    enforce: 'pre',

    async config() {
      /** @type {import('vite').UserConfig & { test?: object }} */
      const extra = { define: productDefines(product) };
      if (process.env.VITEST && product.absent.length) {
        const { configDefaults } = await import('vitest/config');
        extra.test = {
          exclude: [...configDefaults.exclude, ...product.absent.map((m) => `${relUi(m.ui)}/**`)],
        };
      }
      return extra;
    },

    resolveId: {
      filter: resolveFilter,
      async handler(source, importer, options) {
        if (source === VIRTUAL || source.startsWith(VIRTUAL + '/')) return '\0' + source;
        if (product.absent.length === 0 || source.startsWith('\0')) return null;

        const resolved = await this.resolve(source, importer, { ...options, skipSelf: true });
        if (!resolved || resolved.external) return null;
        const file = resolved.id.split('?', 1)[0];
        if (!path.isAbsolute(file)) return null;

        if (absentRoute(file)) return null; // the route file is stubbed in `load`
        // The dev server's dependency scan reads route files from disk, past the
        // stubs of `load`: what an absent module's route or file imports is never
        // served or built, so the scan may follow it without an error.
        const from = importer?.split('?', 1)[0];
        // `scan` is set by the scanner, outside the public type of the options.
        const scanning = /** @type {{ scan?: boolean }} */ (options)?.scan === true;
        if (scanning && from && (absentRoute(from) || product.absent.some((m) => inside(from, m.ui)))) return null;
        for (const m of product.absent) {
          if (!inside(file, m.ui)) continue;
          this.error(
            `[veydan-modules] product "${product.name}" (${product.platform}) does not include module "${m.id}", ` +
              `but ${importer ? rel(importer.split('?', 1)[0]) : '<entry>'} imports ${JSON.stringify(source)} ` +
              `(${rel(file)}). Go through the registry ($lib/core/registry) or the directory ($lib/core/directory), ` +
              `or add "${m.id}" to the product in ${rel(MANIFEST)}.`,
          );
        }
        return null;
      },
    },

    // The product's logo and favicon (`brand`) win over ui/static/. In dev, a
    // middleware answers their paths before SvelteKit's and Vite's static
    // handlers: what a `pre` plugin adds here comes first.
    configureServer(server) {
      if (Object.keys(product.brand).length === 0) return;
      server.middlewares.use((req, res, next) => {
        const at = (req.url ?? '').split(/[?#]/, 1)[0].replace(/^\/+/, '');
        const file = Object.hasOwn(product.brand, at) ? product.brand[at] : undefined;
        if (!file || (req.method !== 'GET' && req.method !== 'HEAD')) return next();
        res.setHeader('Content-Type', at.endsWith('.png') ? 'image/png' : 'image/x-icon');
        res.setHeader('Cache-Control', 'no-cache');
        res.end(req.method === 'HEAD' ? undefined : fs.readFileSync(file));
      });
    },

    // In a build, the client build emits them under the same names.
    generateBundle() {
      if (this.environment?.config.build.ssr) return;
      for (const [fileName, file] of Object.entries(product.brand)) {
        this.emitFile({ type: 'asset', fileName, source: fs.readFileSync(file) });
      }
    },

    load: {
      filter: loadFilter,
      handler(id) {
        if (id.startsWith('\0' + VIRTUAL)) {
          const code = virtualCode(product, id.slice(1));
          if (code === null) this.error(`[veydan-modules] unknown virtual module ${id.slice(1)}`);
          return code;
        }
        if (product.absent.length === 0) return null;
        const [file, query] = id.split('?', 2);
        if (query || !path.isAbsolute(file)) return null;
        const route = absentRoute(file);
        if (route) return route.stub;
        const m = product.absent.find((a) => inside(file, a.ui));
        if (m) this.error(`[veydan-modules] ${rel(file)} of absent module "${m.id}" was loaded in product "${product.name}"`);
        return null;
      },
    },
  };
}

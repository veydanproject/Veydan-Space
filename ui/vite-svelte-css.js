import fs from "node:fs";
import { compile, preprocess } from "svelte/compiler";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// Virtual style id used by vite-plugin-svelte.
const STYLE_ID = /[?&]svelte&type=style&lang\.css$/;
const FS_PREFIX = "/@fs/";

const preprocessor = vitePreprocess();
const cache = new Map();

/** @param {string} id */
function filenameOf(id) {
  let filename = id.split("?", 1)[0];
  if (filename.startsWith(FS_PREFIX)) filename = filename.slice(FS_PREFIX.length);
  return filename;
}

// Compile scoped CSS ourselves. The official plugin sometimes misses its
// in-memory CSS map and Vite then serves the raw .svelte file as a stylesheet.
/** @param {string} filename */
async function compiledCss(filename) {
  const mtime = fs.statSync(filename).mtimeMs;
  const hit = cache.get(filename);
  if (hit && hit.mtime === mtime) return hit.css;

  const source = fs.readFileSync(filename, "utf8");
  const preprocessed = await preprocess(source, preprocessor, { filename });
  const result = compile(preprocessed.code ?? source, {
    filename,
    css: "external",
    dev: true,
    hmr: true,
    generate: "client",
  });
  const css = result.css?.code ?? "";
  cache.set(filename, { mtime, css });
  return css;
}

/** @returns {import('vite').Plugin} */
export function svelteCssGuard() {
  return {
    name: "veydan-svelte-css-guard",
    apply: "serve",
    enforce: "pre",
    async load(id) {
      if (!STYLE_ID.test(id)) return;
      const filename = filenameOf(id);
      if (!filename.endsWith(".svelte")) return;
      this.addWatchFile(filename);
      return {
        code: await compiledCss(filename),
        moduleType: "css",
      };
    },
  };
}

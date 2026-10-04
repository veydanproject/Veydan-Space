// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import path from "node:path";
import { productFromEnv } from "./vite-veydan-modules.js";

// The product (VEYDAN_PRODUCT, set by scripts/ui.mjs) decides the output
// directories: data/build/<product> of the root (<product>-android for the phone
// UI) and .svelte-kit/<product>-<platform>
// (platform-spec 11.4). Without it the configuration fails on purpose.
const product = productFromEnv();
const posix = (p) => p.split(path.sep).join("/");

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    outDir: product.outDir,
    adapter: adapter({
      pages: product.pages,
      fallback: "index.html",
    }),
    typescript: {
      // Types of virtual:veydan-modules/* (written by scripts/ui.mjs) for
      // svelte-check and the editor; the files of absent modules are not checked.
      config(tsconfig) {
        tsconfig.include.push(posix(path.relative(product.outDir, product.typesFile)));
        for (const m of product.absent) {
          for (const dir of [m.ui, ...m.routes]) {
            tsconfig.exclude.push(posix(path.relative(product.outDir, dir)) + "/**");
          }
        }
      },
    },
  },
};

export default config;

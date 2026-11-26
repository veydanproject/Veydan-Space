import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { svelteCssGuard } from "./vite-svelte-css.js";
import { productFromEnv, testExcludes, veydanModules } from "./vite-veydan-modules.js";

const host = process.env.TAURI_DEV_HOST;

// Target platform set by the Tauri CLI (linux/windows/darwin/android/ios).
// Only this value reaches the frontend; other TAURI_* env stays in the build process.
const platform = process.env.TAURI_ENV_PLATFORM ?? "unknown";

// The product (VEYDAN_PRODUCT, set by scripts/ui.mjs) decides the modules,
// the dev ports and the cache directory (platform-spec 11.2, 11.4).
const product = productFromEnv();

// Separate ports per product and platform, so products and platforms can run
// at the same time (desktop / Android / iOS: Space 1420/1430/1432, Notes
// 1440/1450/1452, Pass 1460/1470/1472, Chat 1480/1490/1492); the HMR port of a
// phone is the next one.
const isMobile = product.platform !== "desktop";
const port = product.port;
const hmrPort = isMobile || host ? product.hmrPort : port;
// Separate optimizer caches per product and platform (data/vite/<product>-<platform>).
// One shared cache lets Android dev poison desktop CSS (raw .svelte files get
// served as stylesheets).
const cacheDir = product.cacheDir;

// https://vite.dev/config/
export default defineConfig(async () => ({
  ...(process.env.VITEST ? await testProjects() : {}),
  plugins: [veydanModules(product), svelteCssGuard(), sveltekit()],
  cacheDir,
  // Dependencies reached only through lazy modules (terminal, inspector, QR
  // scanner) are bundled at the start of the dev server: found later, they make
  // Vite reload the page while it starts, and the webview of a cold start
  // shows "Importing a module script failed" or a 500 (ROO-176).
  optimizeDeps: {
    include: ["@xterm/xterm", "@xterm/addon-fit", "@xterm/addon-web-links", "html2canvas", "jsqr"],
  },
  define: {
    __TAURI_PLATFORM__: JSON.stringify(platform),
    // The product of this build (src/lib/core/product.ts) is defined by the
    // plugin veydanModules.
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port,
    strictPort: true,
    // Bind IPv4 loopback explicitly: on dual-stack hosts `false`/localhost binds
    // only IPv6 (::1), and the WebKitGTK dev webview's HMR websocket resolves
    // localhost to 127.0.0.1 — so hot reload never connects and edits don't show.
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: hmrPort,
        }
      : { protocol: "ws", host: "127.0.0.1", port: hmrPort },
    // 3. Vite watches its root, the UI project, alone: the Rust sources,
    //    cargo's build directories (data/target alone holds more files than the
    //    system lets one process watch), the products' build output, the Vite
    //    caches (data/) and the toolchains are outside it.
    // 4. ...and not the SvelteKit folders of the other dev servers
    //    (.svelte-kit/<product>-<platform>, 11.4): each writes its own
    //    tsconfig.json there at its start, which a server watching it took
    //    for a change of its own config — the cache cleared and the page of
    //    its device reloaded (Android beside iOS, 2026-11-26).
    watch: {
      ignored: [new RegExp(`[\\\\/]\\.svelte-kit[\\\\/](?!${product.outDir.split("/").pop()}[\\\\/])`)],
    },
  },
}));

// vitest keeps its defaults: its root is the UI project, so the build and
// work folders of the repository (data/, tmp/ with publish.sh's snapshots
// and clones) are outside what it searches for tests; the modules a product
// has not are left out. A `*.client.test.ts` runs on Svelte's client build
// (vitest-env-client.js), Svelte itself read through Vite by the browser's
// conditions: its `$effect`s run and `$state` keeps its proxies, as in the
// app. Every other test keeps the server build.
async function testProjects() {
  const { configDefaults } = await import("vitest/config");
  const exclude = testExcludes(product, configDefaults.exclude);
  return {
    test: {
      projects: [
        { extends: true, test: { name: "node", exclude: [...exclude, "**/*.client.test.ts"] } },
        {
          extends: true,
          resolve: { conditions: ["browser"] },
          test: {
            name: "client",
            include: ["**/*.client.test.ts"],
            exclude,
            environment: "./vitest-env-client.js",
            server: { deps: { inline: ["svelte"] } },
          },
        },
      ],
    },
  };
}

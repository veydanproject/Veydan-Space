import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { svelteCssGuard } from "./vite-svelte-css.js";
import { productFromEnv, veydanModules } from "./vite-veydan-modules.js";

const host = process.env.TAURI_DEV_HOST;

// Target platform set by the Tauri CLI (linux/windows/darwin/android/ios).
// Only this value reaches the frontend; other TAURI_* env stays in the build process.
const platform = process.env.TAURI_ENV_PLATFORM ?? "unknown";

// The product (VEYDAN_PRODUCT, set by scripts/ui.mjs) decides the modules,
// the dev ports and the cache directory (platform-spec 11.2, 11.4).
const product = productFromEnv();

// Separate ports per product and platform, so products and platforms can run
// at the same time (Space 1420/1430, Notes 1440/1450, Pass 1460/1470, Chat
// 1480/1490); the HMR port of Android is the next one.
const isAndroid = product.platform === "android";
const port = product.port;
const hmrPort = isAndroid || host ? product.hmrPort : port;
// Separate optimizer caches per product and platform (data/vite/<product>-<platform>).
// One shared cache lets Android dev poison desktop CSS (raw .svelte files get
// served as stylesheets).
const cacheDir = product.cacheDir;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [veydanModules(product), svelteCssGuard(), sveltekit()],
  cacheDir,
  // vitest keeps its defaults: its root is the UI project, so the build and
  // work folders of the repository (data/, tmp/ with publish.sh's
  // snapshots and clones) are outside what it searches for tests.
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
  },
}));

# Developing Veydan Space

This page is for people who want to build Veydan Space from source, run its
tests or read its code. To simply use the app, download it from the
[releases](https://github.com/veydanproject/Veydan-Space/releases/latest) and see the [README](../README.md).

## How this repository works

This repository is a **snapshot**. Veydan Space is developed together with the
other Veydan apps in one private workspace, and every release is published
here as a single commit tagged `v<version>`. The code here is complete: it
builds, tests and packages the app on its own.

- **Issues are welcome** — bug reports, ideas, questions.
- **Pull requests cannot be merged here**, because nobody commits to a
  snapshot by hand. If you have a fix, open an issue and describe it (a patch
  in the issue is fine).

## Prerequisites

| Tool | Version |
|---|---|
| Rust | the one pinned in `rust-toolchain.toml` (`rustup` installs it on first use) |
| Node.js | 22 or newer |
| pnpm | 11 |
| Linux only | WebKitGTK 4.1, GTK 3, libsoup 3, librsvg, libayatana-appindicator, OpenSSL |

On Debian or Ubuntu the Linux libraries are:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
  libayatana-appindicator3-dev libssl-dev patchelf build-essential
```

No root? `scripts/dev.sh` can set up a self-contained toolchain and the
system libraries inside the project directory (`data/`).

On Windows you need the Microsoft C++ Build Tools and WebView2 (preinstalled
on Windows 10 and 11); on macOS, the Xcode command line tools.

## Run in development

```bash
pnpm --dir ui install --frozen-lockfile   # the UI project and the Tauri CLI, in ui/
scripts/dev.sh space                       # dev server on port 1420 and a debug build of the app
```

The window opens when the build is ready; edits to the UI reload live.
Stop with Ctrl+C. To try the app without touching your own data, give it a
separate profile:

```bash
WORKDIR=tmp/try scripts/dev.sh space
```

Settings → *Load demo data* fills a fresh profile with sample content.

## Build the app

```bash
scripts/tauri.sh space build              # installers and packages under data/target/release/bundle/
scripts/tauri.sh space build --no-bundle  # just the release binary: data/target/release/veydanspace
```

Always build through the wrappers in `scripts/`. The Tauri CLI comes from
the UI project (`ui/node_modules/.bin/tauri`) and must run from the root of
the repository with `TAURI_APP_PATH` pointing at the app crate
(`apps/space`) and `TAURI_FRONTEND_PATH` at `ui/`; `scripts/tauri.sh`,
`scripts/dev.sh`, `scripts/update.sh` and `scripts/android/*.sh` take
the product as their first argument and set all of that for you.

`scripts/update.sh space` builds the release binary and installs it for
your user on Linux (with a menu entry).

## Android

```bash
scripts/android/apk.sh space build --target aarch64   # a debug-signed APK, not installed
```

The APK lands in
`apps/space/gen/android/app/build/outputs/apk/universal/release/`; install
it with `adb install -r <file>`. `scripts/android/apk.sh space release`
makes a release-signed one when the signing key is configured. Run one
Android build at a time.

## Checks and tests

These are the commands CI runs on every push:

```bash
node scripts/ui.mjs space svelte-check     # type check of the UI
node scripts/ui.mjs space vitest run       # unit tests of the UI
node scripts/ui.mjs space build            # the UI, into data/build/space/ (the phone UI: data/build/space-android/)
cargo check -p veydanspace --locked
cargo test --workspace --all-targets --locked
cargo test --workspace --all-targets --locked --manifest-path crates/messenger/Cargo.toml   # the chat's own workspace
bash scripts/boundaries.sh                 # which crate and UI folder may depend on which
```

## Repository layout

```
apps/space/       The app crate: Tauri config, icons, the Android project, the list of modules
crates/           The platform (core, lock, sync, shell) and the modules of the app
ui/               The SvelteKit frontend: src/lib/core and one folder per module
products.json     The product: its modules, their UI folders, routes and crates
scripts/          Build, run and check scripts
docs/             This page and the pictures of the README
data/             Everything a build makes (not in git): target/, build/, toolchains
```

Veydan Space is a **Tauri 2** app: a Rust backend (the crates) and a
**Svelte 5 / SvelteKit** frontend (TypeScript) rendered by the system
webview. The data lives in SQLite on the device (notes are plain Markdown files); optional sync
encrypts everything on the device before it reaches your folder, S3 bucket
or WebDAV share.

`apps/space/extension/` holds the web clipper, a Firefox extension the app embeds.

The chat lives in `crates/messenger/`: a Cargo workspace of its own,
free of Tauri, with its own lock file; the app takes its crates by path.

## License

Veydan Space is source-available under the
[PolyForm Perimeter License 1.0.1](../LICENSE) — a summary is in
[LICENSE-SUMMARY.md](../LICENSE-SUMMARY.md), and the licences of everything
it is built from are in [THIRD-PARTY-LICENSES.md](../THIRD-PARTY-LICENSES.md).

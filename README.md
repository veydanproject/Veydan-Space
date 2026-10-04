# Veydan Space

> A workspace for multi-accounting.

This repository holds the sources of **Veydan Space 5.0.0** and its releases.
It is a snapshot: the product is developed together with the other Veydan
apps, and each release is published here as one commit with the tag
`v<version>`. Nobody commits here by hand, so pull requests cannot be merged
in this repository; issues are welcome.

Built with **Tauri 2** (Rust) and **Svelte 5 / SvelteKit** (TypeScript).
Local-first: the data lives on your machine, with no telemetry.

## Build

Prerequisites: **Rust** (stable), **Node.js** with **pnpm**, and on Linux
WebKitGTK 4.1, GTK 3 and libsoup 3 (`scripts/dev.sh` can bootstrap a
self-contained toolchain and the system libraries into the project directory
without root).

```bash
pnpm --dir ui install --frozen-lockfile       # the UI project and the Tauri CLI, in ui/
scripts/dev.sh space                             # run in development
scripts/tauri.sh space build                     # native bundles under data/target/
```

The Tauri CLI is the one of the UI project (`ui/node_modules/.bin/tauri`)
and always runs from the root of the repository with `TAURI_APP_PATH`
pointing at the crate of the product and `TAURI_FRONTEND_PATH` at `ui/`;
`scripts/tauri.sh`, `scripts/dev.sh`, `scripts/update.sh` and
`scripts/android/*.sh` take the product as their first word and set them.

```bash
cargo check -p veydanspace                    # the crate of the product
node scripts/ui.mjs space build            # its UI, into data/build/space/ (the phone UI: data/build/space-android/)
node scripts/ui.mjs space svelte-check     # type check
node scripts/ui.mjs space vitest run       # unit tests of the UI
cargo test --workspace --all-targets        # tests of the crates
bash scripts/boundaries.sh                  # which crate and which UI folder may depend on which
```

## Layout

```
products.json     The product: its modules, their UI folders, routes and crates
apps/space/        The crate of the product: Tauri config, icons, gen/android, the module list
crates/           The platform (core, lock, sync, shell) and the modules of the product
ui/               SvelteKit frontend: src/lib/core and one folder per module
scripts/          Build, run and check scripts
data/             Everything the build makes (not in git): target/, build/, the toolchains
```

## License

Copyright © 2026 **Veydan Project**.

Veydan Space is **source-available** software, licensed under the
[PolyForm Perimeter License 1.0.1](https://polyformproject.org/licenses/perimeter/1.0.1):
see [`LICENSE`](LICENSE), a summary in [`LICENSE-SUMMARY.md`](LICENSE-SUMMARY.md)
and the licences of what it is built from in
[`THIRD-PARTY-LICENSES.md`](THIRD-PARTY-LICENSES.md).

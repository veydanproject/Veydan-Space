#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The product a wrapper works on (docs/platform-spec.md 13.2) — meant to be
# *sourced* after ROOT_DIR is set:
#
#   product_env <product>       # space, notes, pass or chat
#
# Refuses a name products.json does not hold as a product and a product
# whose crate does not exist yet: the Tauri CLI silently builds another
# product when TAURI_APP_PATH points nowhere. On success it exports
# TAURI_APP_PATH (the product) and TAURI_FRONTEND_PATH (the UI project,
# ui/: the CLI runs beforeDevCommand and beforeBuildCommand there and
# reads the versions of the JS packages there), and sets:
#
#   PRODUCT                     the name, as in products.json
#   APP_DIR                     the absolute path of apps/<product>
#   PRODUCT_BIN                 the package of the crate: the binary, the icon
#                               and the desktop entry are named after it
#   PRODUCT_NAME                productName of tauri.conf.json
#   PRODUCT_IDENTIFIER          the identifier on a computer
#   PRODUCT_ANDROID_IDENTIFIER  the applicationId on Android
#   PRODUCT_DEV_PORT            the dev server on a computer (HMR: the next port)
#   PRODUCT_ANDROID_DEV_PORT    the dev server for Android (HMR: the next port)
#   PRODUCT_VERSION             apps/<product>/VERSION
#   TAURI_CLI                   the Tauri CLI of the UI project
#                               (ui/node_modules/.bin/tauri)
#
# The wrappers then run "$TAURI_CLI" from ROOT_DIR, after tauri_cli_ready;
# by hand: scripts/tauri.sh <product> <args>. dev_workdir <dir> checks and
# exports the data directory of a dev instance (VEYDAN_WORKDIR).

product_env() {
  local name="${1:-space}"
  : "${ROOT_DIR:?ROOT_DIR must be set before product_env}"
  if ! command -v node >/dev/null 2>&1; then
    source "$ROOT_DIR/scripts/toolchain.sh" >/dev/null
  fi
  local vars
  if ! vars=$(node - "$ROOT_DIR" "$name" <<'EOF'
const fs = require("fs");
const path = require("path");
const [root, name] = process.argv.slice(2);
const fail = (why) => { console.error(`>> ${why}`); process.exit(1); };
const manifest = JSON.parse(fs.readFileSync(path.join(root, "products.json"), "utf8"));
const products = Object.entries(manifest.targets).filter(([, t]) => t.kind === "product").map(([n]) => n);
const t = manifest.targets[name];
if (!t || t.kind !== "product") fail(`unknown product '${name}' (products: ${products.join(", ")})`);
const dir = path.join(root, t.app);
const cargo = path.join(dir, "Cargo.toml");
const conf = path.join(dir, "tauri.conf.json");
if (!fs.existsSync(cargo) || !fs.existsSync(conf)) fail(`${t.app}: no crate of the product '${name}' yet`);
const pkg = /^\[package\][^[]*?^name\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(cargo, "utf8"));
if (!pkg) fail(`${t.app}/Cargo.toml: no package name`);
const config = JSON.parse(fs.readFileSync(conf, "utf8"));
const versionFile = path.join(dir, "VERSION");
const version = fs.existsSync(versionFile) ? fs.readFileSync(versionFile, "utf8").trim() : "";
const quote = (v) => `'${String(v ?? "").replace(/'/g, `'\\''`)}'`;
const out = {
  PRODUCT: name,
  APP_DIR: dir,
  PRODUCT_BIN: pkg[1],
  PRODUCT_NAME: config.productName,
  PRODUCT_IDENTIFIER: t.identifier,
  PRODUCT_ANDROID_IDENTIFIER: t.androidIdentifier ?? t.identifier,
  PRODUCT_DEV_PORT: t.devPort,
  PRODUCT_ANDROID_DEV_PORT: t.androidDevPort,
  PRODUCT_VERSION: version,
};
for (const [k, v] of Object.entries(out)) console.log(`${k}=${quote(v)}`);
EOF
  ); then
    return 1
  fi
  eval "$vars"
  export TAURI_APP_PATH="$APP_DIR"
  export TAURI_FRONTEND_PATH="$ROOT_DIR/ui"
  TAURI_CLI="$ROOT_DIR/ui/node_modules/.bin/tauri"
}

# Fails, naming the fix, when the UI project's dependencies (and with them
# the Tauri CLI) are not installed. The wrappers that run the CLI call it
# before "$TAURI_CLI".
tauri_cli_ready() {
  if [ ! -x "$TAURI_CLI" ]; then
    echo ">> ${TAURI_CLI#"$ROOT_DIR"/} is missing: run 'pnpm --dir ui install --frozen-lockfile' first" >&2
    return 1
  fi
}

# A field of the desktop entry template of the product (linux/<bin>.desktop).
product_desktop_field() {
  local template="$APP_DIR/linux/$PRODUCT_BIN.desktop"
  [ -f "$template" ] || return 0
  sed -n "s/^$1=//p" "$template" | head -n1
}

# The data directory of a dev instance (WORKDIR=<dir>, --workdir): exports
# VEYDAN_WORKDIR as the absolute path, a relative one counted from where the
# command was typed. Refuses a folder inside the repository unless it is
# under tmp/: anywhere else the profile's files would show in git as untracked.
dev_workdir() {
  local dir root
  dir="$(realpath -m "$1")"
  root="$(realpath -m "$ROOT_DIR")"
  case "$dir/" in
    "$root/tmp/"?*) ;;
    "$root/"*)
      echo ">> WORKDIR $dir is inside the repository: use tmp/<name> (e.g. WORKDIR=tmp/${dir##*/}) or a folder outside it" >&2
      return 1
      ;;
  esac
  VEYDAN_WORKDIR="$dir"
  export VEYDAN_WORKDIR
}

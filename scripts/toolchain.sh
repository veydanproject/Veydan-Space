#!/usr/bin/env bash
# Project-local toolchain bootstrap + environment.
#
# Installs Rust and Node entirely inside data/toolchains/ so the host system
# (/usr, $HOME) stays untouched. Everything lives under the project dir, in
# data/, which git ignores as a whole. Idempotent — safe to source on every run.
#
# This file is meant to be *sourced* (it exports env vars): by build-env.sh,
# and by the scripts that need only cargo and node.

# The root of the repository, from where this file is (scripts/), whoever
# sources it and from wherever.
VEYDAN_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export VEYDAN_ROOT

TC_DIR="$VEYDAN_ROOT/data/toolchains"
export RUSTUP_HOME="$TC_DIR/rustup"
export CARGO_HOME="$TC_DIR/cargo"
NODE_DIR="$TC_DIR/node"
NODE_VERSION="v24.18.0"

mkdir -p "$TC_DIR"

# --- Rust (rustup + cargo, minimal profile) ---
if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  echo ">> Installing Rust into data/toolchains/ ..."
  curl -sSf --proto '=https' --tlsv1.2 https://sh.rustup.rs -o /tmp/_rb_rustup.sh
  RUSTUP_HOME="$RUSTUP_HOME" CARGO_HOME="$CARGO_HOME" \
    sh /tmp/_rb_rustup.sh -y --no-modify-path --profile minimal >/dev/null
  rm -f /tmp/_rb_rustup.sh
fi
export PATH="$CARGO_HOME/bin:$PATH"
# The version of Rust is the one rust-toolchain.toml names, the same CI
# installs; rustup reads the file from the working directory upwards. It is
# installed here, by name, so that a build never starts with another version
# and does not depend on rustup fetching a missing toolchain on demand.
if ! (cd "$VEYDAN_ROOT" && rustup which rustc >/dev/null 2>&1); then
  echo ">> Installing the Rust of rust-toolchain.toml into data/toolchains/ ..."
  (cd "$VEYDAN_ROOT" && rustup toolchain install >/dev/null)
fi

# --- Node (portable tarball) ---
if [ ! -x "$NODE_DIR/bin/node" ]; then
  echo ">> Installing Node $NODE_VERSION into data/toolchains/ ..."
  tarball="node-$NODE_VERSION-linux-x64"
  curl -sSL "https://nodejs.org/dist/$NODE_VERSION/$tarball.tar.xz" -o /tmp/_rb_node.tar.xz
  mkdir -p "$NODE_DIR"
  tar -xJf /tmp/_rb_node.tar.xz -C "$NODE_DIR" --strip-components=1
  rm -f /tmp/_rb_node.tar.xz
fi
export PATH="$NODE_DIR/bin:$PATH"

# --- pnpm (installed globally *inside* the portable Node prefix) ---
if [ ! -x "$NODE_DIR/bin/pnpm" ]; then
  echo ">> Installing pnpm into data/toolchains/node ..."
  npm install -g pnpm@latest >/dev/null 2>&1
fi

# Keep the pnpm content-addressable store inside the project too, in data/.
# pnpm 12 reads pnpm_config_store_dir and ignores npm_config_store_dir (with
# only the latter it used ~/.local/share/pnpm/store); an older pnpm reads the
# second. Not storeDir in ui/pnpm-workspace.yaml (nor store-dir in ui/.npmrc,
# which pnpm 12 does not read): that file goes to the snapshots and to CI.
export pnpm_config_store_dir="$VEYDAN_ROOT/data/pnpm-store"
export npm_config_store_dir="$pnpm_config_store_dir"

#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# Dependency boundaries of the platform (docs/platform-spec.md, section 6;
# for the messenger also docs/messenger-spec.md §4.2). Fails when a crate
# depends on what its row of the matrix does not allow, when a formal rule
# of the module contract is broken, or when a UI file imports what the
# matrix of the frontend (6.3) does not allow it.
#
# Dependencies are read from `cargo metadata`, not from the text of the
# manifests: a dependency under a target, renamed, or inherited from a
# workspace is seen like any other. Source text is read by
# scripts/boundaries.mjs. A check that cannot run fails the script.
#
# The same script runs in the published snapshot of one product
# (scripts/publish.sh; its products.json says `"snapshot": "<product>"`).
# A snapshot holds the crates of that product only: the rows of the matrix
# whose directory is not there are left out, and so is the messenger when
# the product has none. Everything that is there is checked as here.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD

# cargo and node come from PATH (CI) or from the project's own toolchain.
if ! command -v cargo >/dev/null 2>&1 || ! command -v node >/dev/null 2>&1; then
  source ./scripts/toolchain.sh
fi

fail=0
say() { echo "::error::$*"; fail=1; }

# The declared dependencies of every package of a workspace, one per line:
#   <package> <its directory> <dependency> <normal|dev|build> <directory of a path dependency>
# with `-` for what a line does not have. The first line of a package names
# no dependency and ends with the path of its build script. Directories and
# paths are relative to the repository.
deps() {
  local meta
  meta=$(cargo metadata --no-deps --offline --format-version 1 --manifest-path "$1") || {
    echo "::error::cargo cannot load $1" >&2
    return 1
  }
  node -e '
    const path = require("path");
    const meta = JSON.parse(require("fs").readFileSync(0, "utf8"));
    const rel = (p) => path.relative(process.argv[1], p);
    for (const p of meta.packages) {
      const dir = rel(path.dirname(p.manifest_path));
      const build = p.targets.find((t) => t.kind.includes("custom-build"));
      console.log([p.name, dir, "-", "-", build ? rel(build.src_path) : "-"].join("\t"));
      for (const d of p.dependencies)
        console.log([p.name, dir, d.name, d.kind || "normal", d.path ? rel(d.path) : "-"].join("\t"));
    }' "$ROOT" <<<"$meta"
}
# The same for a folder that may have moved to a repository of its own.
deps_if_here() { if [ -f "$1" ]; then deps "$1"; fi; }

# The edges of a lock file: <package> <a package it depends on>.
lock_edges() {
  awk '
    /^name = / { gsub(/"/, "", $3); pkg = $3 }
    /^ "/      { gsub(/[",]/, "", $1); print pkg "\t" $1 }' "$1"
}

# Everything a package of a lock file reaches, itself included.
closure() {
  awk -F'\t' -v start="$2" '
    { edges[$1] = edges[$1] " " $2 }
    END {
      seen[start] = 1; queue[1] = start
      for (head = tail = 1; head <= tail; head++) {
        print queue[head]
        n = split(edges[queue[head]], next_ones, " ")
        for (i = 1; i <= n; i++) if (!(next_ones[i] in seen)) { seen[next_ones[i]] = 1; queue[++tail] = next_ones[i] }
      }
    }' <<<"$1"
}

# A lock file holds every dependency the manifests of its workspace declare,
# for every target and feature. One it does not hold means the lock is behind
# the manifests, and the rule on Tauri would be read from an old graph.
behind() {
  local missing
  missing=$(awk -F'\t' '
    $0 == "--" { manifests = 1; next }
    !manifests { held[$1 "\t" $2] = 1; next }
    $3 != "-" && !(($1 "\t" $3) in held) { print $1 " -> " $3 }' <<<"$2"$'\n--\n'"$3")
  [ -z "$missing" ] || while IFS= read -r edge; do
    say "$1 does not hold $edge: the lock file is behind the manifests; let cargo update it and commit it"
  done <<<"$missing"
}

# A scan of scripts/boundaries.mjs: every line it prints is a violation.
scan() {
  local out line
  if ! out=$(node scripts/boundaries.mjs "$@"); then
    say "scripts/boundaries.mjs $1 ${*: -1}: the scan failed"
    return 0
  fi
  [ -z "$out" ] || while IFS= read -r line; do say "$line"; done <<<"$out"
}

is_tauri() { case "$1" in tauri|tauri-*) return 0 ;; *) return 1 ;; esac; }

# The product a snapshot was published for; empty in the monorepo.
if ! snapshot=$(node -e '
  const m = JSON.parse(require("fs").readFileSync("products.json", "utf8"));
  if (m.snapshot !== undefined && !(m.targets[m.snapshot] && m.targets[m.snapshot].kind === "product")) process.exit(1);
  console.log(m.snapshot ?? "")'); then
  echo "::error::products.json: cannot be read, or its \"snapshot\" names no product of its own"
  echo "boundaries: FAILED"
  exit 1
fi
# The messenger is a workspace of its own; a snapshot of a product without
# the module does not hold it.
messenger=yes
if [ -n "$snapshot" ] && [ ! -e crates/messenger ]; then messenger=""; fi

for lock in Cargo.lock ${messenger:+crates/messenger/Cargo.lock}; do
  if [ ! -f "$lock" ]; then
    echo "::error::$lock is missing: the rule on Tauri is read from it"
    echo "boundaries: FAILED"
    exit 1
  fi
done

# The infrastructure, from products.json: <target> <its folder> <what its
# crates may take by path outside the folder, separated by spaces>.
if ! infra_targets=$(node -e '
  const m = JSON.parse(require("fs").readFileSync("products.json", "utf8"));
  for (const [name, t] of Object.entries(m.targets)) {
    if (t.kind !== "infra") continue;
    if (!Array.isArray(t.paths) || t.paths.length !== 1 || !Array.isArray(t.builtOn)) process.exit(1);
    console.log([name, t.paths[0], t.builtOn.join(" ")].join("\t"));
  }'); then
  echo "::error::products.json: an infra target needs one folder in \"paths\" and a \"builtOn\" list"
  echo "boundaries: FAILED"
  exit 1
fi
if [ -z "$snapshot" ] && [ -z "$infra_targets" ]; then
  echo "::error::products.json: no infra target; the rule on the infrastructure reads nothing"
  echo "boundaries: FAILED"
  exit 1
fi

root_deps=$(deps Cargo.toml)
messenger_deps=""
messenger_lock=""
if [ -n "$messenger" ]; then
  messenger_deps=$(deps crates/messenger/Cargo.toml)
  messenger_lock=$(lock_edges crates/messenger/Cargo.lock)
fi
# The dependencies of every infra folder that is here, in one list.
infra_deps=""
while IFS=$'\t' read -r _ infra_dir _; do
  [ -n "$infra_dir" ] || continue
  infra_deps+=$(deps_if_here "$infra_dir/Cargo.toml")$'\n'
done <<<"$infra_targets"
root_lock=$(lock_edges Cargo.lock)

# The directory of a package a workspace of this script holds?
is_package() { grep -q "^[^	]*	$1	-	" <<<"$root_deps"$'\n'"$messenger_deps"; }

# ═════════════════════════════════════════════════════════════════════════════
# Crates
# ═════════════════════════════════════════════════════════════════════════════

# 1. The matrix of section 6.1: one row per crate of the root workspace.
#    A stage that adds a crate adds its row here and in the spec.
#
#      directory   package   kind   Tauri   crates of the repository it may depend on
#
#    lib      a crate of the platform
#    module   the crate of a module: it depends on no other module (its row
#             names none) and its SQL names only the tables it creates; the
#             messenger keeps its data in a database of its own and creates
#             none, so its adapter holds no SQL at all
#    plugin   a Tauri plugin, the only kind whose package is named tauri-plugin-*
#    product  the crate tauri-build and generate_context! run in
#
#    Only the adapter of the messenger reaches into crates/messenger. The
#    product names the push plugin itself: tauri-build finds the Android
#    library of a plugin through the direct dependencies of the product.
MATRIX='
crates/build-cfg                 veydan-build-cfg          lib      no
crates/lock                      veydan-lock               lib      no
crates/sync                      veydan-sync               lib      no
crates/desktop-notify            desktop-notify            lib      no
crates/core                      veydan-core               lib      yes  crates/build-cfg crates/lock
crates/sync-host                 veydan-sync-host          lib      yes  crates/build-cfg crates/core crates/lock crates/sync
crates/shell                     veydan-shell              lib      yes  crates/build-cfg crates/core crates/lock crates/sync crates/sync-host
crates/pass                      veydan-pass               module   yes  crates/build-cfg crates/core crates/lock crates/sync-host crates/shell
crates/notes                     veydan-notes              module   yes  crates/build-cfg crates/core crates/lock crates/sync crates/sync-host crates/shell
crates/messenger-app             veydan-messenger-app      module   yes  crates/build-cfg crates/core crates/lock crates/shell crates/messenger/core crates/messenger/runtime crates/messenger/notify crates/desktop-notify crates/tauri-plugin-veydan-push
crates/tauri-plugin-veydan-push  tauri-plugin-veydan-push  plugin   yes  crates/build-cfg
apps/space                       veydanspace               product  yes  crates/shell crates/core crates/lock crates/sync crates/sync-host crates/pass crates/notes crates/messenger-app crates/tauri-plugin-veydan-push
apps/notes                       veydannotes               product  yes  crates/shell crates/notes
apps/pass                        veydanpass                product  yes  crates/shell crates/pass
apps/chat                        veydanchat                product  yes  crates/shell crates/messenger-app crates/tauri-plugin-veydan-push
'
# A snapshot holds the rows of its product only.
if [ -n "$snapshot" ]; then
  MATRIX=$(while read -r dir rest; do
    if [ -n "$dir" ] && [ -e "$dir/Cargo.toml" ]; then echo "$dir $rest"; fi
  done <<<"$MATRIX")
  grep -q " product " <<<"$MATRIX" || { echo "::error::the snapshot holds no product crate of the matrix"; echo "boundaries: FAILED"; exit 1; }
fi
row() {
  local dir rest
  while read -r dir rest; do
    if [ "$dir" = "$1" ]; then echo "$rest"; return 0; fi
  done <<<"$MATRIX"
  return 1
}

while IFS=$'\t' read -r pkg dir dep kind path; do
  if ! r=$(row "$dir"); then
    if [ "$dep" = - ]; then
      say "$pkg ($dir): not in the dependency matrix (add its row to scripts/boundaries.sh and to section 6.1 of docs/platform-spec.md)"
    fi
    continue
  fi
  read -r row_pkg row_kind row_tauri row_allow <<<"$r"
  if [ "$dep" = - ]; then
    [ "$pkg" = "$row_pkg" ] || say "$dir: the package is named '$pkg', the matrix says '$row_pkg'"
    case "$pkg" in
      tauri-plugin-*) [ "$row_kind" = plugin ] || say "$pkg: only a Tauri plugin is named tauri-plugin-*; the Tauri macros take the commands of such a crate for a plugin's" ;;
    esac
    continue
  fi
  if is_tauri "$dep" && [ "$row_tauri" = no ]; then
    say "$pkg: depends on $dep ($kind); the crate stays free of Tauri"
  fi
  if [ "$path" != - ]; then
    case " $row_allow " in
      *" $path "*) ;;
      *) say "$pkg: depends on $dep at '$path' ($kind); its row of the matrix does not allow it" ;;
    esac
    # A crate at an allowed path that no workspace holds would have
    # dependencies of its own that nothing reads.
    is_package "$path" || say "$pkg: depends on $dep at '$path' ($kind), a package of neither the root workspace nor the messenger's; nothing checks what it depends on"
  fi
done <<<"$root_deps"

while read -r dir row_pkg row_kind row_tauri _; do
  [ -n "$dir" ] || continue
  grep -q "^$row_pkg	$dir	-	" <<<"$root_deps" || say "$dir ($row_pkg): in the dependency matrix, not in the workspace"
  # Tauri must not arrive through a crate of the registry either.
  if [ "$row_tauri" = no ]; then
    reached=$(closure "$root_lock" "$row_pkg")
    while read -r one; do
      if is_tauri "$one"; then
        say "$row_pkg: reaches $one through its dependencies (Cargo.lock); the crate stays free of Tauri"
      fi
    done <<<"$reached"
  fi
done <<<"$MATRIX"
behind Cargo.lock "$root_lock" "$root_deps"

# A crate of the repository is taken by path: a dependency on it from git or
# from a registry has no path, and the matrix would not see it.
repo_packages=$(awk -F'\t' '$3 == "-" { print $1 }' <<<"$root_deps"$'\n'"$messenger_deps"$'\n'"$infra_deps" | sort -u)
while IFS=$'\t' read -r pkg dir dep kind path; do
  if [ -n "$dep" ] && [ "$dep" != - ] && [ "$path" = - ] && grep -qxF "$dep" <<<"$repo_packages"; then
    say "$pkg ($dir): depends on $dep ($kind), a crate of this repository, and not by path"
  fi
done <<<"$root_deps"$'\n'"$messenger_deps"$'\n'"$infra_deps"

# 2. The product crate (5.2, 5.5).
while read -r dir row_pkg row_kind _; do
  [ "$row_kind" = product ] || continue
  # Commands of library crates pass without capabilities only while the
  # product has no permissions of its own.
  [ ! -e "$dir/permissions" ] || say "$dir/permissions: a product has no permissions of its own; with them every command of a module needs a capability entry"
  # tauri-build learns a plugin's permissions only from the direct
  # dependencies of the crate it runs in. A capability is a file of
  # capabilities/ or an entry of the Tauri config.
  for file in "$dir"/capabilities/* "$dir"/[Tt]auri*; do
    [ -e "$file" ] || continue
    case "$file" in
      *.json) ;;
      "$dir"/capabilities/*) say "$file: not a .json capability; scripts/boundaries.sh cannot read the permissions it names"; continue ;;
      *) say "$file: not a .json Tauri config; scripts/boundaries.sh cannot read the capabilities it may hold"; continue ;;
    esac
    if ! plugins=$(node -e '
      const plugins = new Set();
      const visit = (value) => {
        if (Array.isArray(value)) return value.forEach(visit);
        if (value === null || typeof value !== "object") return;
        if (Array.isArray(value.permissions)) {
          for (const p of value.permissions) {
            const id = typeof p === "string" ? p : p.identifier;
            if (id.includes(":") && !id.startsWith("core:")) plugins.add(id.split(":")[0]);
          }
        }
        Object.values(value).forEach(visit);
      };
      visit(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")));
      console.log([...plugins].join(" "));' "$file"); then
      say "$file: cannot be read as JSON"
      continue
    fi
    for plugin in $plugins; do
      grep -q "^$row_pkg	$dir	tauri-plugin-$plugin	normal	" <<<"$root_deps" ||
        say "$file names a permission of '$plugin', and $row_pkg does not depend on tauri-plugin-$plugin directly"
    done
  done
done <<<"$MATRIX"

# 3. Formal rules of the module contract (6.2, item 9): no command is
#    renamed, a product hands no app manifest to tauri-build, a library that
#    uses cfg(desktop) or cfg(mobile) has a build script calling the helper,
#    and so has, for the Windows manifest of its tests, every library that
#    depends on Tauri.
while read -r dir row_pkg row_kind row_tauri _; do
  [ -n "$dir" ] || continue
  build=$(awk -F'\t' -v dir="$dir" '$2 == dir && $3 == "-" { print $5 }' <<<"$root_deps")
  scan rules "$row_kind" "$row_pkg" "$dir" "${build:--}" "$row_tauri"
done <<<"$MATRIX"

# 8. The plugins of a product in products.json are its direct `tauri-plugin-*`
#    dependencies, no more and no less (5.5, 6.4 p. 8): the UI launcher and the
#    publish script read the list from the manifest, tauri-build from Cargo.toml.
#    A product whose crate does not exist yet (apps/notes before stage 10) is skipped.
if ! product_plugins=$(node -e '
  const fs = require("fs");
  const m = JSON.parse(fs.readFileSync("products.json", "utf8"));
  for (const [name, t] of Object.entries(m.targets)) {
    if (t.kind !== "product" || !fs.existsSync(t.app + "/Cargo.toml")) continue;
    console.log([name, t.app, [...(t.plugins ?? [])].sort().join(",")].join("\t"));
  }'); then
  say "products.json: cannot be read"
  product_plugins=""
fi
[ -n "$product_plugins" ] || say "products.json: no product with a crate; the plugin check reads nothing"
while IFS=$'\t' read -r name app listed; do
  [ -n "$name" ] || continue
  actual=$(awk -F'\t' -v dir="$app" '$2 == dir && $3 ~ /^tauri-plugin-/ && $4 == "normal" { sub(/^tauri-plugin-/, "", $3); print $3 }' <<<"$root_deps" | sort -u | paste -sd,)
  [ "$actual" = "$listed" ] ||
    say "products.json: the plugins of $name are [$listed], and $app/Cargo.toml depends directly on [$actual]"
done <<<"$product_plugins"

# 4. Owners of commands (section 22). The prefix of a command names its
#    module; where it would mislead, the command is named here one by one.
owner() {
  case "$1" in
    app_start_error|host_info|open_url|clipboard_write_text|update_supported|update_check|update_open|window_minimize|media_grant_access) echo shell ;;
    tray_*|app_locale_*|modules_*|labels_*) echo shell ;;
    demo_seed|app_clear_data) echo shell ;;
    lock_*|vault_replaced_keys|vault_replaced_keys_open) echo shell ;;
    sync_conflict_get|sync_conflict_resolve|sync_attachment_cancel) echo notes ;;
    sync_profile_files_take_remote|sync_profile_files_push_mine) echo browser ;;
    sync_*) echo shell ;;
    notes_capture_rules_get|notes_capture_rules_set) echo capture ;;
    notes_get_dir|notes_set_dir|notes_attachment_policy_get|notes_attachment_policy_set|open_quick_capture|clipboard_file_paths) echo notes ;;
    note_*) echo notes ;;
    password_*|totp_*|pwgen_history_*) echo pass ;;
    messenger_*) echo messenger ;;
    fingerprint_presets) echo browser ;;
    workspace_*|profile_*|profiles_*|proxy_*|proxies_*|camoufox_*) echo browser ;;
    ssh_*|sftp_*|fs_*) echo ssh ;;
    backup_*) echo backup ;;
    *) echo "" ;;
  esac
}
# The commands of a product with the module that answers each and the
# platforms that have it (Space: src/modules/commands.golden.txt; the other
# products: src/commands.golden.txt). A test of the product crate holds the
# file equal to the router's table of the platform the tests run on; the
# platform of a command is read here from the `module!` lists, which no
# test on a computer sees for a phone. The crates of modules are read with
# the product that lists them: those among its direct dependencies.
while read -r dir row_pkg row_kind _; do
  [ "$row_kind" = product ] || continue
  golden=$dir/src/modules/commands.golden.txt
  [ -f "$golden" ] || golden=$dir/src/commands.golden.txt
  [ -f "$golden" ] || { say "$dir: no commands.golden.txt under src/modules/ or src/"; continue; }
  module_srcs=()
  while read -r mod_dir mod_pkg mod_kind _; do
    [ "$mod_kind" = module ] || continue
    grep -q "^$row_pkg	$dir	$mod_pkg	normal	$mod_dir$" <<<"$root_deps" || continue
    module_srcs+=("$mod_dir/src=${mod_pkg//-/_}")
  done <<<"$MATRIX"
  while read -r command module _; do
    want=$(owner "$command")
    if [ -z "$want" ]; then
      say "command $command: no module owns its prefix (name it in owner() of scripts/boundaries.sh and in section 22 of docs/platform-spec.md)"
    elif [ "$want" != "$module" ]; then
      say "command $command is declared by module '$module'; section 22 gives it to '$want'"
    fi
  done <"$golden"
  scan commands "$golden" "$dir/src" crates/shell/src "${module_srcs[@]}"
done <<<"$MATRIX"

# 4a. The SQL of a module names the tables of the module only (6.2, item 8);
#     that of a library its own and the core's: the lock no longer reads
#     the passwords of pass. Tables put into SQL by format! are read from
#     the literals of the same file; tests are not read.
srcs=()
while read -r dir _ row_kind _; do
  case "$row_kind" in lib | module | product) [ -d "$dir/src" ] && srcs+=("$dir/src") ;; esac
done <<<"$MATRIX"
while read -r dir row_pkg row_kind _; do
  case "$row_kind" in lib | module) ;; *) continue ;; esac
  others=()
  for src in "${srcs[@]}"; do
    if [ "$src" = "$dir/src" ]; then continue
    elif [ "$row_kind" = lib ] && [ "$src" = crates/core/src ]; then others+=("+$src")
    else others+=("$src")
    fi
  done
  scan tables "$row_kind" "$dir/src" "${others[@]}"
done <<<"$MATRIX"

# 4b. The internal modules of a product are not scanned for their tables
#     until they are crates (5.10), but none names the core's table `labels`:
#     owners publish and retract their labels through the entity directory
#     (10.2).
while read -r dir _ row_kind _; do
  [ "$row_kind" = product ] || continue
  scan tables product "$dir/src" labels
done <<<"$MATRIX"

# 5. The messenger: its crates depend neither on Tauri nor on the platform.
#    One path leaves crates/messenger: the crate `vlink` is built on the
#    client of VLink (services/link/crates/vlink-proto,
#    services/link/crates/vlink-client).
#    Nothing else of VLink, and no other crate, may follow.
#    The crate in crates/messenger/<x> is the package messenger-<x>.
#    Per-crate allow-list of dependencies inside the messenger:
allowed() {
  case "$1" in
    core)      echo "" ;;
    vlink)     echo "" ;;
    http)      echo "messenger-core messenger-vlink" ;;
    links)     echo "messenger-core" ;;
    preview)   echo "messenger-core messenger-http" ;;
    store)     echo "messenger-core" ;;
    transport) echo "messenger-core messenger-http messenger-vlink" ;;
    identity)  echo "messenger-core messenger-store" ;;
    ingress)   echo "messenger-core messenger-store messenger-identity" ;;
    contacts)  echo "messenger-core messenger-store messenger-http" ;;
    dm)        echo "messenger-core messenger-store messenger-contacts messenger-links" ;;
    media)     echo "messenger-core messenger-store messenger-http" ;;
    groups)    echo "messenger-core messenger-store messenger-media messenger-dm messenger-links" ;;
    push)      echo "messenger-core messenger-http" ;;
    notify)    echo "messenger-core messenger-store messenger-ingress messenger-contacts messenger-dm messenger-groups messenger-media messenger-transport messenger-vlink" ;;
    runtime)   echo "messenger-core messenger-store messenger-transport messenger-identity messenger-ingress messenger-contacts messenger-dm messenger-media messenger-groups messenger-links messenger-preview messenger-push messenger-notify messenger-vlink messenger-http" ;;
    testkit)   echo "messenger-core messenger-store messenger-runtime messenger-notify messenger-dm messenger-groups messenger-vlink" ;;
    *)         echo "__unknown__" ;;
  esac
}
[ -z "$messenger" ] || while IFS=$'\t' read -r pkg dir dep kind path; do
  crate=${dir#crates/messenger/}
  allow=$(allowed "$crate")
  if [ "$dep" = - ]; then
    [ "$allow" != "__unknown__" ] || say "$crate: not in the dependency matrix of the messenger (update scripts/boundaries.sh and docs/messenger-spec.md)"
    [ "$pkg" = "messenger-$crate" ] || say "$dir: the package is named '$pkg'; a crate of the messenger is named after its directory, 'messenger-$crate'"
    continue
  fi
  if is_tauri "$dep"; then say "$crate: depends on $dep ($kind)"; fi
  case "$dep" in
    veydan-*|veydanspace) say "$crate: depends on $dep, a crate of the host" ;;
  esac
  case "$path" in
    -) ;;
    crates/messenger/*)
      # The dependency is told by its directory, whatever the package in it
      # is named. dev-dependencies may use testkit freely.
      is_package "$path" || say "$crate: depends on '$path' ($kind), which is not a crate of the messenger's workspace"
      if [ "$kind" != dev ] && [ "$allow" != "__unknown__" ]; then
        case " $allow " in
          *" messenger-${path#crates/messenger/} "*) ;;
          *) say "$crate: 'messenger-${path#crates/messenger/}' is not an allowed dependency" ;;
        esac
      fi ;;
    services/link/crates/vlink-proto|services/link/crates/vlink-client)
      [ "$crate" = vlink ] || say "$crate: path dependency outside crates/messenger ('$path')" ;;
    *) say "$crate: path dependency outside crates/messenger ('$path')" ;;
  esac
done <<<"$messenger_deps"
if [ -n "$messenger" ]; then
  messenger_locked=$(sed -nE 's/^name = "([^"]*)"$/\1/p' crates/messenger/Cargo.lock)
  while read -r name; do
    if is_tauri "$name"; then
      say "crates/messenger/Cargo.lock holds $name: the messenger stays free of Tauri"
    fi
  done <<<"$messenger_locked"
  behind crates/messenger/Cargo.lock "$messenger_lock" "$messenger_deps"
fi

# 6. The infrastructure (6.1, the last two rows; the targets of kind "infra"
#    in products.json): the crates of a folder take by path only what is in
#    the folder and what its "builtOn" names — the bridge stands alone, the
#    hub is built on the crates of the bridge, the push server on nothing —
#    and none of them knows Tauri. A folder that has moved to a repository
#    of its own is not checked here.
while IFS=$'\t' read -r infra_name infra_dir infra_on; do
  [ -n "$infra_name" ] || continue
  while IFS=$'\t' read -r pkg dir dep kind path; do
    [ -n "$dep" ] && [ "$dep" != - ] || continue
    case "$dir/" in "$infra_dir"/*) ;; *) continue ;; esac
    if is_tauri "$dep"; then say "$pkg ($infra_dir): depends on $dep ($kind)"; fi
    [ "$path" != - ] || continue
    inside=""
    for prefix in "$infra_dir" $infra_on; do
      case "$path/" in "$prefix"/*) inside=yes ;; esac
    done
    [ -n "$inside" ] || say "$pkg ($infra_dir): path dependency on '$path' leaves what $infra_dir may be built on"
  done <<<"$infra_deps"
done <<<"$infra_targets"

# ═════════════════════════════════════════════════════════════════════════════
# Frontend
# ═════════════════════════════════════════════════════════════════════════════
# The matrix of section 6.3 for the whole of ui/src/ (6.4, p. 7): the core imports
# no module, a module only itself and the core, the messenger a closed list of
# the core's primitives and outside packages, a module's route files that
# module and the core, the shared routes the core alone. Every import form is
# read — `import … from` and `export … from` over any number of lines,
# `import type`, an import of a file alone, `import()` — and an `import()`
# of a computed path is an error. The owners are read from products.json.
if ! ui_problems=$(node scripts/boundaries.mjs ui products.json); then
  say "scripts/boundaries.mjs ui products.json: the scan failed"
  ui_problems=""
fi
while IFS= read -r line; do
  [ -n "$line" ] || continue
  say "$line"
done <<<"$ui_problems"
# The same scan against planted imports of every form, in a scratch tree: a
# form it stops catching (or a permitted import it starts to report) fails here.
if ! ui_selftest=$(node scripts/boundaries.mjs ui-selftest); then
  say "scripts/boundaries.mjs ui-selftest: the self-test failed to run"
  ui_selftest=""
fi
while IFS= read -r line; do
  [ -n "$line" ] || continue
  say "$line"
done <<<"$ui_selftest"

if [ "$fail" -ne 0 ]; then
  echo "boundaries: FAILED"
  exit 1
fi
echo "boundaries: ok"

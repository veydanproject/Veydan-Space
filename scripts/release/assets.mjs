#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The files of a release of one product (internal/platform-spec.md 14.3). A
// build job gathers what the Tauri bundler wrote and names it for a GitHub
// release; the publishing job writes latest.json over what the build jobs
// gathered. Nothing here knows where the release is published.
//
//   assets.mjs collect --bundle <dir> --name "Veydan Notes" --version 5.0.0 [--target <triple>] --pubkey <key> --out <dir>
//   assets.mjs latest  --dir <dir> --prefix Veydan.Notes --version 5.0.0 --keys "linux-x86_64 …" --base-url <url>
//
// collect: <dir> is cargo's `…/release/bundle`. It takes the installers and
// the updater signatures of this product and this version only — the target
// directory is cached and shared by the products — and gives each the name
// the release shows: the productName with dots for spaces, and the
// architecture in the name of the macOS updater archive, which the bundler
// writes without one (`Veydan Notes.app.tar.gz` for either).
//
// Every updater signature is checked against --pubkey, the public key of
// plugins.updater.pubkey in the product's tauri.conf.json: made by that
// key (its id), over that bundle (Ed25519 over the BLAKE2b-512 of the file),
// with its trusted comment intact. Tauri signs with whatever key the secret
// TAURI_SIGNING_PRIVATE_KEY holds and compares it with nothing; a build
// signed with another key would install, and its first update would be
// refused on every machine.
//
// latest: the latest.json of the Tauri updater for the platforms in --keys,
// each with the signature of its bundle and <base-url>/<bundle>.

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** What a release offers: installers, updater archives and their signatures. */
const BUNDLE = /\.(deb|rpm|AppImage|AppImage\.tar\.gz|exe|msi|nsis\.zip|msi\.zip|dmg|app\.tar\.gz)(\.sig)?$/;
const MAC_ARCH = { 'aarch64-apple-darwin': 'aarch64', 'x86_64-apple-darwin': 'x64' };

/** The name of a release asset: GitHub turns every other character into a dot. */
export const assetName = (name) => name.replace(/[^A-Za-z0-9._+-]/g, '.');

/** The asset name of a bundler file of the product, or null when the file is not one of its bundles. */
export function releaseName(file, { name, version, target = '' }) {
  if (!BUNDLE.test(file)) return null;
  // The updater archive of macOS carries neither the version nor the architecture.
  for (const tail of ['.app.tar.gz', '.app.tar.gz.sig']) {
    if (file === `${name}${tail}`) {
      const arch = MAC_ARCH[target];
      if (!arch) throw new Error(`${file}: a macOS updater archive needs --target (${Object.keys(MAC_ARCH).join(', ')})`);
      return assetName(`${name}_${version}_${arch}${tail}`);
    }
  }
  // Veydan Notes_5.0.0_amd64.deb, Veydan Notes-5.0.0-1.x86_64.rpm
  if (file.startsWith(`${name}_${version}_`) || file.startsWith(`${name}-${version}-`)) return assetName(file);
  return null;
}

/** The 8 bytes of a minisign key id as minisign prints it (`BDF4EB57F29DF004`). */
const keyIdText = (id) => Buffer.from(id).reverse().toString('hex').toUpperCase();

/**
 * The updater's public key as tauri.conf.json carries it: base64 of the text
 * of a minisign public key, whose second line is base64 of `Ed`, the key id
 * (8 bytes) and the Ed25519 key (32 bytes).
 * @returns {{ id: Buffer, idText: string, key: crypto.KeyObject }}
 */
export function parsePublicKey(pubkey) {
  const lines = Buffer.from(String(pubkey).trim(), 'base64').toString('utf8').split('\n');
  const raw = Buffer.from((lines[1] ?? '').trim(), 'base64');
  if (!/^untrusted comment: /.test(lines[0] ?? '') || raw.length !== 42 || raw.subarray(0, 2).toString('latin1') !== 'Ed') {
    throw new Error('the updater public key is not a minisign public key (plugins.updater.pubkey of tauri.conf.json)');
  }
  const key = crypto.createPublicKey({ key: { kty: 'OKP', crv: 'Ed25519', x: raw.subarray(10).toString('base64url') }, format: 'jwk' });
  return { id: raw.subarray(2, 10), idText: keyIdText(raw.subarray(2, 10)), key };
}

/**
 * What is wrong with the updater signature `sig` (the text of a .sig file:
 * base64 of a minisign signature) of `data`, for the key `pub`; null when
 * it is right. `ED` signs the BLAKE2b-512 of the file, `Ed` (legacy) the
 * file; the global signature covers the signature and the trusted comment.
 */
export function signatureProblem(data, sig, pub) {
  const lines = Buffer.from(String(sig).trim(), 'base64').toString('utf8').split('\n');
  const raw = Buffer.from((lines[1] ?? '').trim(), 'base64');
  if (!/^untrusted comment: /.test(lines[0] ?? '') || raw.length !== 74) return 'not a minisign signature';
  const algorithm = raw.subarray(0, 2).toString('latin1');
  if (algorithm !== 'ED' && algorithm !== 'Ed') return `an unknown signature algorithm '${algorithm}'`;
  const id = raw.subarray(2, 10);
  if (!id.equals(pub.id)) return `signed with the key ${keyIdText(id)}, and the updater trusts ${pub.idText} (is TAURI_SIGNING_PRIVATE_KEY the key of plugins.updater.pubkey?)`;
  const signature = raw.subarray(10);
  const signed = algorithm === 'ED' ? crypto.createHash('blake2b512').update(data).digest() : data;
  if (!crypto.verify(null, signed, pub.key, signature)) return `the signature of key ${pub.idText} does not match the file`;
  const trusted = /^trusted comment: (.*)$/.exec(lines[2] ?? '');
  const global = Buffer.from((lines[3] ?? '').trim(), 'base64');
  if (!trusted || global.length !== 64 || !crypto.verify(null, Buffer.concat([signature, Buffer.from(trusted[1], 'utf8')]), pub.key, global)) {
    return 'the trusted comment of the signature is not signed by the key';
  }
  return null;
}

/** Copies the bundles of the product from `bundle` into `out`; returns the asset names. */
export function collect({ bundle, name, version, target = '', pubkey, out }) {
  // Read first: a key that is not one stops the build before anything is copied.
  const pub = parsePublicKey(pubkey ?? '');
  if (!fs.existsSync(bundle)) throw new Error(`${bundle}: no bundle directory (did the build run?)`);
  const found = new Map();
  for (const kind of fs.readdirSync(bundle, { withFileTypes: true })) {
    if (!kind.isDirectory()) continue;
    for (const entry of fs.readdirSync(path.join(bundle, kind.name), { withFileTypes: true })) {
      if (!entry.isFile()) continue;
      const asset = releaseName(entry.name, { name, version, target });
      if (!asset) continue;
      if (found.has(asset)) throw new Error(`${asset}: two files of ${bundle} get this name`);
      found.set(asset, path.join(bundle, kind.name, entry.name));
    }
  }
  const names = [...found.keys()].sort();
  if (!names.some((n) => !n.endsWith('.sig'))) throw new Error(`${bundle}: no bundle of ${name} ${version}`);
  // The updater installs only what is signed: a build without signatures
  // (no TAURI_SIGNING_PRIVATE_KEY) must not look like a release.
  if (!names.some((n) => n.endsWith('.sig'))) throw new Error(`${bundle}: no updater signature of ${name} ${version} (is TAURI_SIGNING_PRIVATE_KEY set?)`);
  for (const sig of names.filter((n) => n.endsWith('.sig'))) {
    if (!found.has(sig.slice(0, -4))) throw new Error(`${sig}: the bundle it signs is missing`);
    const problem = signatureProblem(fs.readFileSync(found.get(sig.slice(0, -4))), fs.readFileSync(found.get(sig), 'utf8'), pub);
    if (problem) throw new Error(`${sig}: ${problem}`);
  }
  fs.mkdirSync(out, { recursive: true });
  for (const [asset, file] of found) fs.copyFileSync(file, path.join(out, asset));
  return names;
}

/** The signature the updater of a platform should be offered, in the order of preference. */
const UPDATER_BUNDLE = {
  'linux-x86_64': [/\.AppImage\.sig$/, /\.AppImage\.tar\.gz\.sig$/],
  'windows-x86_64': [/-setup\.exe\.sig$/, /\.nsis\.zip\.sig$/],
  'darwin-aarch64': [/_aarch64\.app\.tar\.gz\.sig$/],
  'darwin-x86_64': [/_x64\.app\.tar\.gz\.sig$/],
};

/** latest.json over the assets of `dir`: only bundles whose names start with the product's prefix. */
export function latest({ dir, prefix, version, keys, baseUrl, now = new Date() }) {
  const names = fs.readdirSync(dir).sort();
  const own = names.filter((n) => n.startsWith(`${prefix}_`) || n.startsWith(`${prefix}-`));
  const platforms = {};
  for (const key of keys) {
    const patterns = UPDATER_BUNDLE[key];
    if (!patterns) throw new Error(`${key}: not a platform of the updater (${Object.keys(UPDATER_BUNDLE).join(', ')})`);
    let sig;
    for (const pattern of patterns) {
      sig = own.find((n) => pattern.test(n));
      if (sig) break;
    }
    if (!sig) throw new Error(`no updater signature of ${prefix} for ${key}`);
    const asset = sig.slice(0, -4);
    if (!names.includes(asset)) throw new Error(`${asset}: the bundle of ${key} is missing next to its signature`);
    platforms[key] = {
      signature: fs.readFileSync(path.join(dir, sig), 'utf8'),
      url: `${baseUrl.replace(/\/$/, '')}/${asset}`,
    };
  }
  return {
    version,
    notes: 'See the assets below to download and install this version.',
    pub_date: now.toISOString(),
    platforms,
  };
}

function options(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || argv[i + 1] === undefined) throw new Error(`unexpected '${argv[i]}'`);
    out[argv[i].slice(2)] = argv[i + 1];
  }
  return out;
}

function need(opts, ...names) {
  for (const n of names) if (!opts[n]) throw new Error(`--${n} is required`);
}

function main([command, ...rest]) {
  const opts = options(rest);
  if (command === 'collect') {
    need(opts, 'bundle', 'name', 'version', 'pubkey', 'out');
    const names = collect({ bundle: opts.bundle, name: opts.name, version: opts.version, target: opts.target ?? '', pubkey: opts.pubkey, out: opts.out });
    for (const n of names) console.log(n);
  } else if (command === 'latest') {
    need(opts, 'dir', 'prefix', 'version', 'keys', 'base-url');
    const doc = latest({ dir: opts.dir, prefix: opts.prefix, version: opts.version, keys: opts.keys.split(/\s+/).filter(Boolean), baseUrl: opts['base-url'] });
    fs.writeFileSync(path.join(opts.dir, 'latest.json'), `${JSON.stringify(doc, null, 2)}\n`);
    console.log(`latest.json: ${opts.version}, ${Object.keys(doc.platforms).join(' ')}`);
  } else {
    throw new Error('usage: assets.mjs collect|latest … (see the head of the file)');
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`::error::${e.message}`);
    process.exit(1);
  }
}

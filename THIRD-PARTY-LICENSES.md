# Third-Party Licenses & Acknowledgements

Veydan Space is built on the shoulders of the open-source community. This
software would not be possible without the following projects. We are deeply
grateful to their authors and maintainers.

This document lists every third-party dependency bundled or linked into
Veydan Space, together with its license. It is provided to satisfy the
attribution requirements of the MIT, Apache-2.0, BSD, ISC, MPL-2.0 and other
licenses under which these components are distributed.

- **Rust / Cargo crates:** 678
- **JavaScript / npm packages:** 156
- **Total third-party components:** 834

> Entries marked with an asterisk (`*`) are platform-specific dependencies
> (Windows / macOS / Android / WASM targets) that are **not** compiled into the
> Linux build. Their license is recorded from upstream crate metadata for
> completeness of attribution across all supported platforms.

All bundled dependencies use permissive or file-level-copyleft licenses
(MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, MPL-2.0, CC0, BSL-1.0; the fonts OFL-1.1). **No GPL, LGPL,
AGPL or SSPL code is included.** MPL-2.0 components (`cssparser`, `selectors`,
`webpki-root-certs`, `lightningcss`, and related crates) are used unmodified;
their file-level copyleft imposes no obligations on Veydan Space's own code.

---

## License summary — Rust crates

| License | Count |
|---|---:|
| MIT OR Apache-2.0 | 275 |
| MIT | 133 |
| MIT OR Apache-2.0 (platform dep) | 96 |
| Apache-2.0 OR MIT | 80 |
| Unicode-3.0 | 18 |
| Apache-2.0 | 7 |
| MPL-2.0 | 6 |
| ISC | 6 |
| Unlicense OR MIT | 6 |
| BSD-3-Clause | 5 |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | 3 |
| Apache-2.0 OR ISC OR MIT | 3 |
| MIT OR Apache-2.0 OR Zlib | 5 |
| Zlib OR Apache-2.0 OR MIT | 2 |
| Zlib | 3 |
| BSD-3-Clause OR MIT OR Apache-2.0 | 2 |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | 2 |
| Apache-2.0 WITH LLVM-exception | 2 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (platform dep) | 2 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| ISC AND (Apache-2.0 OR ISC) | 1 |
| ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) | 1 |
| BSD-3-Clause AND MIT | 1 |
| BSD-3-Clause OR MIT | 1 |
| Apache-2.0 AND MIT | 1 |
| (Apache-2.0 OR MIT) AND BSD-3-Clause | 1 |
| MIT OR Apache-2.0 OR BSD-1-Clause | 1 |
| Apache-2.0  OR  MIT | 1 |
| bzip2-1.0.6 | 1 |
| MIT OR Zlib OR Apache-2.0 | 1 |
| CC0-1.0 | 1 |
| CC0-1.0 OR MIT-0 | 1 |
| Apache-2.0 AND ISC | 1 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 |
| Apache-2.0 OR BSL-1.0 OR MIT | 1 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| BSL-1.0 (platform dep) | 2 |

## License summary — npm packages

| License | Count |
|---|---:|
| MIT | 137 |
| Apache-2.0 OR MIT | 6 |
| Apache-2.0 | 6 |
| MIT OR Apache-2.0 | 3 |
| MPL-2.0 | 2 |
| ISC | 1 |
| BSD-3-Clause | 1 |

---

## Full acknowledgements — Rust crates (678)

Thank you to the authors of the following Cargo crates:

| Crate | Version | License |
|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| aead | 0.6.1 | MIT OR Apache-2.0 |
| aes | 0.9.1 | MIT OR Apache-2.0 |
| aes-gcm | 0.11.0 | Apache-2.0 OR MIT |
| aho-corasick | 1.1.4 | Unlicense OR MIT |
| alloc-no-stdlib | 2.0.4 | BSD-3-Clause |
| alloc-stdlib | 0.2.4 | BSD-3-Clause |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 |
| android_system_properties | 0.1.5 | MIT OR Apache-2.0 * |
| anyhow | 1.0.103 | MIT OR Apache-2.0 |
| arboard | 3.6.1 | MIT OR Apache-2.0 |
| argon2 | 0.6.0-rc.8 | MIT OR Apache-2.0 |
| async-trait | 0.1.89 | MIT OR Apache-2.0 |
| atk | 0.18.2 | MIT |
| atk-sys | 0.18.2 | MIT |
| atoi | 2.0.0 | MIT |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| aws-lc-rs | 1.17.1 | ISC AND (Apache-2.0 OR ISC) |
| aws-lc-sys | 0.42.0 | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) |
| base16ct | 1.0.0 | Apache-2.0 OR MIT |
| base32 | 0.5.1 | MIT OR Apache-2.0 |
| base64 | 0.21.7 | MIT OR Apache-2.0 * |
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| base64ct | 1.8.3 | Apache-2.0 OR MIT |
| bcrypt-pbkdf | 0.11.0 | MIT OR Apache-2.0 |
| bit-set | 0.8.0 | Apache-2.0 OR MIT |
| bit-vec | 0.8.0 | Apache-2.0 OR MIT |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| bitflags | 2.13.0 | MIT OR Apache-2.0 |
| blake2 | 0.11.0-rc.6 | MIT OR Apache-2.0 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 |
| block-padding | 0.4.2 | MIT OR Apache-2.0 |
| block2 | 0.6.2 | MIT OR Apache-2.0 * |
| blowfish | 0.10.0 | MIT OR Apache-2.0 |
| brotli | 8.0.4 | BSD-3-Clause AND MIT |
| brotli-decompressor | 5.0.3 | BSD-3-Clause/MIT |
| bs58 | 0.5.1 | MIT/Apache-2.0 |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 |
| bytemuck | 1.25.0 | Zlib OR Apache-2.0 OR MIT * |
| byteorder | 1.5.0 | Unlicense OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| bytes | 1.12.0 | MIT |
| bzip2 | 0.6.1 | MIT OR Apache-2.0 |
| cairo-rs | 0.18.5 | MIT |
| cairo-sys-rs | 0.18.2 | MIT |
| camino | 1.2.4 | MIT OR Apache-2.0 |
| cargo_metadata | 0.19.2 | MIT |
| cargo_toml | 0.22.3 | Apache-2.0 OR MIT |
| cargo-platform | 0.1.9 | MIT OR Apache-2.0 |
| cbc | 0.2.1 | MIT OR Apache-2.0 |
| cc | 1.2.65 | MIT OR Apache-2.0 |
| cesu8 | 1.1.0 | Apache-2.0 OR MIT * |
| cfb | 0.7.3 | MIT |
| cfg_aliases | 0.2.1 | MIT |
| cfg-expr | 0.15.8 | MIT OR Apache-2.0 |
| cfg-if | 1.0.4 | MIT OR Apache-2.0 |
| chacha20 | 0.10.1 | MIT OR Apache-2.0 |
| chrono | 0.4.45 | MIT OR Apache-2.0 |
| cipher | 0.5.2 | MIT OR Apache-2.0 |
| clipboard-win | 5.4.1 | BSL-1.0 (platform dep) |
| cmake | 0.1.58 | MIT OR Apache-2.0 |
| cmov | 0.5.4 | Apache-2.0 OR MIT |
| color_quant | 1.1.0 | MIT |
| combine | 4.6.7 | MIT * |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT |
| const-oid | 0.10.2 | Apache-2.0 OR MIT |
| constant_time_eq | 0.3.1 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| constant_time_eq | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| cookie | 0.18.1 | MIT OR Apache-2.0 |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 (platform dep) |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 (platform dep) |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 (platform dep) |
| core-graphics | 0.25.0 | MIT OR Apache-2.0 (platform dep) |
| core-graphics-types | 0.2.0 | MIT OR Apache-2.0 (platform dep) |
| cpubits | 0.1.1 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| cpufeatures | 0.3.0 | MIT OR Apache-2.0 |
| crc | 3.4.0 | MIT OR Apache-2.0 |
| crc-catalog | 2.5.0 | MIT OR Apache-2.0 |
| crc32fast | 1.5.0 | MIT OR Apache-2.0 |
| crossbeam-channel | 0.5.15 | MIT OR Apache-2.0 |
| crossbeam-queue | 0.3.12 | MIT OR Apache-2.0 |
| crossbeam-utils | 0.8.21 | MIT OR Apache-2.0 |
| crypto-bigint | 0.7.5 | Apache-2.0 OR MIT |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 |
| crypto-primes | 0.7.2 | Apache-2.0 OR MIT |
| cssparser | 0.36.0 | MPL-2.0 |
| cssparser-macros | 0.6.1 | MPL-2.0 |
| ctor | 0.8.0 | Apache-2.0 OR MIT |
| ctor-proc-macro | 0.0.7 | Apache-2.0 OR MIT |
| ctr | 0.10.1 | MIT OR Apache-2.0 |
| ctutils | 0.4.2 | Apache-2.0 OR MIT |
| curve25519-dalek | 5.0.0-rc.1 | BSD-3-Clause |
| curve25519-dalek-derive | 0.1.1 | MIT/Apache-2.0 |
| darling | 0.23.0 | MIT |
| darling_core | 0.23.0 | MIT |
| darling_macro | 0.23.0 | MIT |
| data-encoding | 2.11.0 | MIT |
| dbus | 0.9.12 | Apache-2.0/MIT |
| deflate64 | 0.1.12 | MIT |
| delegate | 0.13.5 | MIT OR Apache-2.0 |
| der | 0.8.0 | Apache-2.0 OR MIT |
| deranged | 0.5.8 | MIT OR Apache-2.0 |
| derive_more | 2.1.1 | MIT |
| derive_more-impl | 2.1.1 | MIT |
| des | 0.9.0 | MIT OR Apache-2.0 |
| diffy | 0.5.0 | MIT OR Apache-2.0 |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| digest | 0.11.3 | MIT OR Apache-2.0 |
| dirs | 6.0.0 | MIT OR Apache-2.0 |
| dirs-sys | 0.5.0 | MIT OR Apache-2.0 |
| dispatch2 | 0.3.1 | MIT OR Apache-2.0 * |
| displaydoc | 0.2.6 | MIT OR Apache-2.0 |
| dlopen2 | 0.8.2 | MIT |
| dlopen2_derive | 0.4.3 | MIT |
| dom_query | 0.27.0 | MIT |
| dotenvy | 0.15.7 | MIT |
| downcast-rs | 1.2.1 | MIT OR Apache-2.0 |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| dtoa | 1.0.11 | MIT OR Apache-2.0 |
| dtoa-short | 0.3.5 | MPL-2.0 |
| dtor | 0.3.0 | Apache-2.0 OR MIT |
| dtor-proc-macro | 0.0.6 | Apache-2.0 OR MIT |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 |
| ecdsa | 0.17.0 | Apache-2.0 OR MIT |
| ed25519 | 3.0.0 | Apache-2.0 OR MIT |
| ed25519-dalek | 3.0.0-rc.1 | BSD-3-Clause |
| either | 1.16.0 | MIT OR Apache-2.0 |
| elliptic-curve | 0.14.1 | Apache-2.0 OR MIT |
| embed_plist | 1.2.2 | MIT OR Apache-2.0 * |
| embed-resource | 3.0.11 | MIT |
| encoding_rs | 0.8.35 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| enum_dispatch | 0.3.13 | MIT OR Apache-2.0 |
| equivalent | 1.0.2 | Apache-2.0 OR MIT |
| erased-serde | 0.4.10 | MIT OR Apache-2.0 |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| error-code | 3.4.0 | BSL-1.0 (platform dep) |
| etcetera | 0.11.0 | MIT OR Apache-2.0 * |
| event-listener | 5.4.1 | Apache-2.0 OR MIT |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 |
| fastrand | 2.4.1 | Apache-2.0 OR MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| ff | 0.14.0 | MIT/Apache-2.0 |
| fiat-crypto | 0.3.0 | MIT OR Apache-2.0 OR BSD-1-Clause |
| field-offset | 0.3.6 | MIT OR Apache-2.0 |
| find-msvc-tools | 0.1.9 | MIT OR Apache-2.0 |
| fixedbitset | 0.5.7 | MIT OR Apache-2.0 |
| flate2 | 1.1.9 | MIT OR Apache-2.0 |
| flume | 0.12.0 | Apache-2.0/MIT |
| fnv | 1.0.7 | Apache-2.0 / MIT |
| foldhash | 0.1.5 | Zlib |
| foldhash | 0.2.0 | Zlib |
| foreign-types | 0.5.0 | MIT OR Apache-2.0 * |
| foreign-types-macros | 0.2.3 | MIT OR Apache-2.0 * |
| foreign-types-shared | 0.3.1 | MIT OR Apache-2.0 * |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 |
| fs_extra | 1.3.0 | MIT |
| fsevent-sys | 4.1.0 | MIT * |
| futures | 0.3.32 | MIT OR Apache-2.0 |
| futures-channel | 0.3.32 | MIT OR Apache-2.0 |
| futures-core | 0.3.32 | MIT OR Apache-2.0 |
| futures-executor | 0.3.32 | MIT OR Apache-2.0 |
| futures-intrusive | 0.5.0 | MIT OR Apache-2.0 |
| futures-io | 0.3.32 | MIT OR Apache-2.0 |
| futures-macro | 0.3.32 | MIT OR Apache-2.0 |
| futures-sink | 0.3.32 | MIT OR Apache-2.0 |
| futures-task | 0.3.32 | MIT OR Apache-2.0 |
| futures-util | 0.3.32 | MIT OR Apache-2.0 |
| gdk | 0.18.2 | MIT |
| gdk-pixbuf | 0.18.5 | MIT |
| gdk-pixbuf-sys | 0.18.0 | MIT |
| gdk-sys | 0.18.2 | MIT |
| gdkwayland-sys | 0.18.2 | MIT |
| gdkx11 | 0.18.2 | MIT |
| gdkx11-sys | 0.18.2 | MIT |
| generic-array | 0.14.7 | MIT |
| generic-array | 1.4.3 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| getrandom | 0.3.4 | MIT OR Apache-2.0 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 |
| ghash | 0.6.0 | Apache-2.0 OR MIT |
| gif | 0.14.2 | MIT OR Apache-2.0 |
| gio | 0.18.4 | MIT |
| gio-sys | 0.18.1 | MIT |
| glib | 0.18.5 | MIT |
| glib-macros | 0.18.5 | MIT |
| glib-sys | 0.18.1 | MIT |
| glob | 0.3.3 | MIT OR Apache-2.0 |
| gobject-sys | 0.18.0 | MIT |
| group | 0.14.0 | MIT/Apache-2.0 |
| gtk | 0.18.2 | MIT |
| gtk-sys | 0.18.2 | MIT |
| gtk3-macros | 0.18.2 | MIT |
| h2 | 0.4.15 | MIT |
| hashbrown | 0.12.3 | MIT OR Apache-2.0 |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hashlink | 0.11.1 | MIT OR Apache-2.0 |
| heck | 0.4.1 | MIT OR Apache-2.0 |
| heck | 0.5.0 | MIT OR Apache-2.0 |
| hex | 0.4.3 | MIT OR Apache-2.0 |
| hex-literal | 1.1.0 | MIT OR Apache-2.0 |
| hkdf | 0.13.0 | MIT OR Apache-2.0 |
| hmac | 0.12.1 | MIT OR Apache-2.0 |
| hmac | 0.13.0 | MIT OR Apache-2.0 |
| html5ever | 0.38.0 | MIT OR Apache-2.0 |
| http | 1.4.2 | MIT OR Apache-2.0 |
| http-body | 1.0.1 | MIT |
| http-body-util | 0.1.3 | MIT |
| httparse | 1.10.1 | MIT OR Apache-2.0 |
| hybrid-array | 0.4.13 | MIT OR Apache-2.0 |
| hyper | 1.10.1 | MIT |
| hyper-rustls | 0.27.9 | Apache-2.0 OR ISC OR MIT |
| hyper-util | 0.1.20 | MIT |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 * |
| ico | 0.5.0 | MIT |
| icu_collections | 2.2.0 | Unicode-3.0 |
| icu_locale_core | 2.2.0 | Unicode-3.0 |
| icu_normalizer | 2.2.0 | Unicode-3.0 |
| icu_normalizer_data | 2.2.0 | Unicode-3.0 |
| icu_properties | 2.2.0 | Unicode-3.0 |
| icu_properties_data | 2.2.0 | Unicode-3.0 |
| icu_provider | 2.2.0 | Unicode-3.0 |
| ident_case | 1.0.1 | MIT/Apache-2.0 |
| idna | 1.1.0 | MIT OR Apache-2.0 |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT |
| image | 0.25.10 | MIT OR Apache-2.0 |
| image-webp | 0.2.4 | MIT OR Apache-2.0 |
| indexmap | 1.9.3 | Apache-2.0 OR MIT |
| indexmap | 2.14.0 | Apache-2.0 OR MIT |
| infer | 0.19.0 | MIT |
| inotify | 0.11.2 | ISC |
| inotify-sys | 0.1.7 | ISC |
| inout | 0.2.2 | MIT OR Apache-2.0 |
| internal-russh-num-bigint | 0.5.0 | MIT OR Apache-2.0 |
| ipnet | 2.12.0 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| javascriptcore-rs | 1.1.2 | MIT |
| javascriptcore-rs-sys | 1.1.1 | MIT |
| jni | 0.21.1 | MIT OR Apache-2.0 (platform dep) |
| jni | 0.22.4 | MIT OR Apache-2.0 (platform dep) |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 (platform dep) |
| jni-sys | 0.3.1 | MIT OR Apache-2.0 (platform dep) |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 (platform dep) |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 (platform dep) |
| jobserver | 0.1.34 | MIT OR Apache-2.0 |
| js-sys | 0.3.103 | MIT OR Apache-2.0 (platform dep) |
| json-patch | 3.0.1 | MIT/Apache-2.0 |
| jsonptr | 0.6.3 | MIT OR Apache-2.0 |
| keccak | 0.2.0 | Apache-2.0 OR MIT |
| kem | 0.3.0 | Apache-2.0 OR MIT |
| keyboard-types | 0.7.0 | MIT OR Apache-2.0 |
| kqueue | 1.2.0 | MIT * |
| kqueue-sys | 1.1.2 | MIT * |
| libappindicator | 0.9.0 | Apache-2.0 OR MIT |
| libappindicator-sys | 0.9.0 | Apache-2.0 OR MIT |
| libbz2-rs-sys | 0.2.5 | bzip2-1.0.6 |
| libc | 0.2.186 | MIT OR Apache-2.0 |
| libdbus-sys | 0.2.7 | Apache-2.0/MIT |
| libloading | 0.7.4 | ISC |
| libredox | 0.1.18 | MIT * |
| libsqlite3-sys | 0.37.0 | MIT |
| litemap | 0.8.2 | Unicode-3.0 |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| log | 0.4.33 | MIT OR Apache-2.0 |
| lru-slab | 0.1.2 | MIT OR Apache-2.0 OR Zlib |
| lz4_flex | 0.13.1 | MIT |
| lzma-rust2 | 0.16.4 | Apache-2.0 |
| markup5ever | 0.38.0 | MIT OR Apache-2.0 |
| md-5 | 0.11.0 | MIT OR Apache-2.0 |
| md5 | 0.8.0 | Apache-2.0/MIT |
| memchr | 2.8.2 | Unlicense OR MIT |
| memoffset | 0.9.1 | MIT |
| mime | 0.3.17 | MIT OR Apache-2.0 |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| mio | 1.2.1 | MIT |
| ml-kem | 0.3.2 | Apache-2.0 OR MIT |
| module-lattice | 0.2.3 | Apache-2.0 OR MIT |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| muda | 0.19.3 | Apache-2.0 OR MIT |
| ndk | 0.9.0 | MIT OR Apache-2.0 (platform dep) |
| ndk-sys | 0.6.0+11769913 | MIT OR Apache-2.0 (platform dep) |
| new_debug_unreachable | 1.0.6 | MIT |
| nix | 0.31.3 | MIT |
| nom | 7.1.3 | MIT |
| nom | 8.0.0 | MIT |
| notify | 8.2.0 | CC0-1.0 |
| notify-types | 2.1.0 | MIT OR Apache-2.0 |
| num_enum | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 * |
| num_enum_derive | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 * |
| num-bigint | 0.4.7 | MIT OR Apache-2.0 |
| num-conv | 0.2.2 | MIT OR Apache-2.0 |
| num-integer | 0.1.46 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| objc2 | 0.6.4 | MIT OR Apache-2.0 (platform dep) |
| objc2-app-kit | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-cloud-kit | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-core-data | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-core-foundation | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-core-graphics | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-core-image | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-core-location | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-core-text | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-encode | 4.1.0 | MIT OR Apache-2.0 (platform dep) |
| objc2-exception-helper | 0.1.1 | MIT OR Apache-2.0 (platform dep) |
| objc2-foundation | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-io-surface | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-quartz-core | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-ui-kit | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-user-notifications | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| objc2-web-kit | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| openssl-probe | 0.2.1 | MIT OR Apache-2.0 |
| option-ext | 0.2.0 | MPL-2.0 |
| os_pipe | 1.2.3 | MIT |
| p256 | 0.14.0-rc.15 | Apache-2.0 OR MIT |
| p384 | 0.14.0-rc.15 | Apache-2.0 OR MIT |
| p521 | 0.14.0-rc.15 | Apache-2.0 OR MIT |
| pageant | 0.2.1 | MIT OR Apache-2.0 * |
| pango | 0.18.3 | MIT |
| pango-sys | 0.18.0 | MIT |
| parking | 2.2.1 | Apache-2.0 OR MIT |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| password-hash | 0.6.1 | MIT OR Apache-2.0 |
| pbkdf2 | 0.13.0 | MIT OR Apache-2.0 |
| pem-rfc7468 | 1.0.0 | Apache-2.0 OR MIT |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| petgraph | 0.8.3 | MIT OR Apache-2.0 |
| phc | 0.6.1 | Apache-2.0 OR MIT |
| phf | 0.13.1 | MIT |
| phf_codegen | 0.13.1 | MIT |
| phf_generator | 0.13.1 | MIT |
| phf_macros | 0.13.1 | MIT |
| phf_shared | 0.13.1 | MIT |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| pkcs1 | 0.8.0-rc.4 | Apache-2.0 OR MIT |
| pkcs5 | 0.8.1 | Apache-2.0 OR MIT |
| pkcs8 | 0.11.0 | Apache-2.0 OR MIT |
| pkg-config | 0.3.33 | MIT OR Apache-2.0 |
| plist | 1.9.0 | MIT |
| png | 0.17.16 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| poly1305 | 0.9.0 | Apache-2.0 OR MIT |
| polyval | 0.7.1 | Apache-2.0 OR MIT |
| potential_utf | 0.1.5 | Unicode-3.0 |
| powerfmt | 0.2.0 | MIT OR Apache-2.0 |
| ppmd-rust | 1.4.0 | CC0-1.0 OR MIT-0 |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 |
| precomputed-hash | 0.1.1 | MIT |
| primefield | 0.14.0 | Apache-2.0 OR MIT |
| primeorder | 0.14.0 | Apache-2.0 OR MIT |
| proc-macro-crate | 1.3.1 | MIT OR Apache-2.0 |
| proc-macro-crate | 2.0.2 | MIT OR Apache-2.0 |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 * |
| proc-macro-error | 1.0.4 | MIT OR Apache-2.0 |
| proc-macro-error-attr | 1.0.4 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.106 | MIT OR Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| quick-error | 2.0.1 | MIT/Apache-2.0 |
| quick-xml | 0.39.4 | MIT |
| quick-xml | 0.41.0 | MIT |
| quinn | 0.11.11 | MIT OR Apache-2.0 |
| quinn-proto | 0.11.15 | MIT OR Apache-2.0 |
| quinn-udp | 0.5.14 | MIT OR Apache-2.0 |
| quote | 1.0.46 | MIT OR Apache-2.0 |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later * |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later * |
| rand | 0.9.4 | MIT OR Apache-2.0 |
| rand | 0.10.2 | MIT OR Apache-2.0 |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 |
| rand_core | 0.9.5 | MIT OR Apache-2.0 |
| rand_core | 0.10.1 | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| redox_syscall | 0.5.18 | MIT * |
| redox_users | 0.5.2 | MIT * |
| ref-cast | 1.0.25 | MIT OR Apache-2.0 |
| ref-cast-impl | 1.0.25 | MIT OR Apache-2.0 |
| regex | 1.12.4 | MIT OR Apache-2.0 |
| regex-automata | 0.4.14 | MIT OR Apache-2.0 |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 |
| reqwest | 0.13.4 | MIT OR Apache-2.0 |
| rfc6979 | 0.6.0 | Apache-2.0 OR MIT |
| rfd | 0.16.0 | MIT |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rsa | 0.10.0-rc.18 | MIT OR Apache-2.0 |
| rsqlite-vfs | 0.1.1 | MIT OR Apache-2.0 * |
| rusqlite | 0.39.0 | MIT |
| russh | 0.62.1 | Apache-2.0 |
| russh-cryptovec | 0.62.0 | Apache-2.0 |
| russh-util | 0.52.0 | Apache-2.0 |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT |
| rustls | 0.23.41 | Apache-2.0 OR ISC OR MIT |
| rustls-native-certs | 0.8.4 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.0 | MIT OR Apache-2.0 |
| rustls-platform-verifier | 0.7.0 | MIT OR Apache-2.0 |
| rustls-platform-verifier-android | 0.1.1 | MIT OR Apache-2.0 * |
| rustls-webpki | 0.103.13 | ISC |
| rustversion | 1.0.22 | MIT OR Apache-2.0 |
| salsa20 | 0.11.0 | MIT OR Apache-2.0 |
| same-file | 1.0.6 | Unlicense/MIT |
| schannel | 0.1.29 | MIT * |
| schemars | 0.8.22 | MIT |
| schemars | 0.9.0 | MIT |
| schemars | 1.2.1 | MIT |
| schemars_derive | 0.8.22 | MIT |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| scrypt | 0.12.0 | MIT OR Apache-2.0 |
| sec1 | 0.8.1 | Apache-2.0 OR MIT |
| security-framework | 3.7.0 | MIT OR Apache-2.0 * |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 * |
| selectors | 0.36.1 | MPL-2.0 |
| semver | 1.0.28 | MIT OR Apache-2.0 |
| serde | 1.0.228 | MIT OR Apache-2.0 |
| serde_core | 1.0.228 | MIT OR Apache-2.0 |
| serde_derive | 1.0.228 | MIT OR Apache-2.0 |
| serde_derive_internals | 0.29.1 | MIT OR Apache-2.0 |
| serde_json | 1.0.150 | MIT OR Apache-2.0 |
| serde_repr | 0.1.20 | MIT OR Apache-2.0 |
| serde_spanned | 0.6.9 | MIT OR Apache-2.0 |
| serde_spanned | 1.1.1 | MIT OR Apache-2.0 |
| serde_with | 3.21.0 | MIT OR Apache-2.0 |
| serde_with_macros | 3.21.0 | MIT OR Apache-2.0 |
| serde-untagged | 0.1.9 | MIT OR Apache-2.0 |
| serdect | 0.4.3 | Apache-2.0 OR MIT |
| serialize-to-javascript | 0.1.2 | MIT OR Apache-2.0 |
| serialize-to-javascript-impl | 0.1.2 | MIT OR Apache-2.0 |
| servo_arc | 0.4.3 | MIT OR Apache-2.0 |
| sha1 | 0.10.6 | MIT OR Apache-2.0 |
| sha1 | 0.11.0 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| sha2 | 0.11.0 | MIT OR Apache-2.0 |
| sha3 | 0.11.0 | MIT OR Apache-2.0 |
| shlex | 2.0.1 | MIT OR Apache-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| signature | 3.0.0 | Apache-2.0 OR MIT |
| simd_cesu8 | 1.1.1 | Apache-2.0 OR MIT * |
| simd-adler32 | 0.3.9 | MIT |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 * |
| siphasher | 1.0.3 | MIT/Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.15.2 | MIT OR Apache-2.0 |
| socket2 | 0.6.4 | MIT OR Apache-2.0 |
| softbuffer | 0.4.8 | MIT OR Apache-2.0 * |
| soup3 | 0.5.0 | MIT |
| soup3-sys | 0.5.0 | MIT |
| spin | 0.9.8 | MIT |
| spki | 0.8.0 | Apache-2.0 OR MIT |
| sqlite-wasm-rs | 0.5.5 | MIT OR Apache-2.0 * |
| sqlx | 0.9.0 | MIT OR Apache-2.0 |
| sqlx-core | 0.9.0 | MIT OR Apache-2.0 |
| sqlx-macros | 0.9.0 | MIT OR Apache-2.0 |
| sqlx-macros-core | 0.9.0 | MIT OR Apache-2.0 |
| sqlx-mysql | 0.9.0 | MIT OR Apache-2.0 |
| sqlx-postgres | 0.9.0 | MIT OR Apache-2.0 |
| sqlx-sqlite | 0.9.0 | MIT OR Apache-2.0 |
| ssh-cipher | 0.3.0 | Apache-2.0 OR MIT |
| ssh-encoding | 0.3.0 | Apache-2.0 OR MIT |
| ssh-key | 0.7.0-rc.11 | Apache-2.0 OR MIT |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 |
| string_cache | 0.9.0 | MIT OR Apache-2.0 |
| string_cache_codegen | 0.6.1 | MIT OR Apache-2.0 |
| stringprep | 0.1.5 | MIT/Apache-2.0 |
| strsim | 0.11.1 | MIT |
| subtle | 2.6.1 | BSD-3-Clause |
| swift-rs | 1.0.7 | MIT OR Apache-2.0 * |
| syn | 1.0.109 | MIT OR Apache-2.0 |
| syn | 2.0.118 | MIT OR Apache-2.0 |
| sync_wrapper | 1.0.2 | Apache-2.0 |
| synstructure | 0.13.2 | MIT |
| system-configuration | 0.7.0 | MIT OR Apache-2.0 * |
| system-configuration-sys | 0.6.0 | MIT OR Apache-2.0 * |
| system-deps | 6.2.2 | MIT OR Apache-2.0 |
| tao | 0.35.3 | Apache-2.0 |
| tao-macros | 0.1.3 | MIT OR Apache-2.0 * |
| target-lexicon | 0.12.16 | Apache-2.0 WITH LLVM-exception |
| tauri | 2.11.5 | Apache-2.0 OR MIT |
| tauri-build | 2.6.3 | Apache-2.0 OR MIT |
| tauri-codegen | 2.6.3 | Apache-2.0 OR MIT |
| tauri-macros | 2.6.3 | Apache-2.0 OR MIT |
| tauri-plugin | 2.6.3 | Apache-2.0 OR MIT |
| tauri-plugin-dialog | 2.7.1 | Apache-2.0 OR MIT |
| tauri-plugin-fs | 2.5.1 | Apache-2.0 OR MIT |
| tauri-runtime | 2.11.3 | Apache-2.0 OR MIT |
| tauri-runtime-wry | 2.11.4 | Apache-2.0 OR MIT |
| tauri-utils | 2.9.3 | Apache-2.0 OR MIT |
| tauri-winres | 0.3.6 | MIT |
| tendril | 0.5.0 | MIT OR Apache-2.0 |
| thiserror | 1.0.69 | MIT OR Apache-2.0 |
| thiserror | 2.0.18 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.18 | MIT OR Apache-2.0 |
| time | 0.3.53 | MIT OR Apache-2.0 |
| time-core | 0.1.9 | MIT OR Apache-2.0 |
| time-macros | 0.2.31 | MIT OR Apache-2.0 |
| tinystr | 0.8.3 | Unicode-3.0 |
| tinyvec | 1.11.0 | Zlib OR Apache-2.0 OR MIT |
| tinyvec_macros | 0.1.1 | MIT OR Apache-2.0 OR Zlib |
| tokio | 1.52.3 | MIT |
| tokio-macros | 2.7.0 | MIT |
| tokio-rustls | 0.26.4 | MIT OR Apache-2.0 |
| tokio-stream | 0.1.18 | MIT |
| tokio-util | 0.7.18 | MIT |
| toml | 0.8.2 | MIT OR Apache-2.0 |
| toml | 0.9.12+spec-1.1.0 | MIT OR Apache-2.0 |
| toml | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_datetime | 0.6.3 | MIT OR Apache-2.0 |
| toml_datetime | 0.7.5+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_datetime | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_edit | 0.19.15 | MIT OR Apache-2.0 |
| toml_edit | 0.20.2 | MIT OR Apache-2.0 |
| toml_edit | 0.25.12+spec-1.1.0 | MIT OR Apache-2.0 * |
| toml_parser | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_writer | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 |
| totp-rs | 5.7.2 | MIT |
| tower | 0.5.3 | MIT |
| tower-http | 0.6.11 | MIT |
| tower-layer | 0.3.3 | MIT |
| tower-service | 0.3.3 | MIT |
| tracing | 0.1.44 | MIT |
| tracing-attributes | 0.1.31 | MIT |
| tracing-core | 0.1.36 | MIT |
| tray-icon | 0.24.1 | MIT OR Apache-2.0 |
| tree_magic_mini | 3.2.2 | MIT |
| try-lock | 0.2.5 | MIT |
| twox-hash | 2.1.2 | MIT |
| typed-path | 0.12.3 | MIT OR Apache-2.0 |
| typeid | 1.0.3 | MIT OR Apache-2.0 |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| unic-char-property | 0.9.0 | MIT/Apache-2.0 |
| unic-char-range | 0.9.0 | MIT/Apache-2.0 |
| unic-common | 0.9.0 | MIT/Apache-2.0 |
| unic-ucd-ident | 0.9.0 | MIT/Apache-2.0 |
| unic-ucd-version | 0.9.0 | MIT/Apache-2.0 |
| unicode-bidi | 0.3.18 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-normalization | 0.1.25 | MIT OR Apache-2.0 |
| unicode-properties | 0.1.4 | MIT/Apache-2.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| universal-hash | 0.6.1 | MIT OR Apache-2.0 |
| untrusted | 0.7.1 | ISC |
| untrusted | 0.9.0 | ISC |
| url | 2.5.8 | MIT OR Apache-2.0 |
| urlencoding | 2.1.3 | MIT |
| urlpattern | 0.3.0 | MIT |
| utf-8 | 0.7.6 | MIT OR Apache-2.0 |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT |
| uuid | 1.23.4 | Apache-2.0 OR MIT |
| vcpkg | 0.2.15 | MIT/Apache-2.0 |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| version-compare | 0.2.1 | MIT |
| vswhom | 0.1.0 | MIT * |
| vswhom-sys | 0.1.3 | MIT * |
| walkdir | 2.5.0 | Unlicense/MIT |
| want | 0.3.1 | MIT |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (platform dep) |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (platform dep) |
| wasm-bindgen | 0.2.126 | MIT OR Apache-2.0 (platform dep) |
| wasm-bindgen-futures | 0.4.76 | MIT OR Apache-2.0 (platform dep) |
| wasm-bindgen-macro | 0.2.126 | MIT OR Apache-2.0 (platform dep) |
| wasm-bindgen-macro-support | 0.2.126 | MIT OR Apache-2.0 (platform dep) |
| wasm-bindgen-shared | 0.2.126 | MIT OR Apache-2.0 (platform dep) |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 * |
| wayland-backend | 0.3.17 | MIT |
| wayland-client | 0.31.15 | MIT |
| wayland-protocols | 0.32.13 | MIT |
| wayland-protocols-wlr | 0.3.12 | MIT |
| wayland-scanner | 0.31.11 | MIT |
| wayland-sys | 0.31.11 | MIT |
| web_atoms | 0.2.5 | MIT OR Apache-2.0 |
| web-sys | 0.3.103 | MIT OR Apache-2.0 (platform dep) |
| web-time | 1.1.0 | MIT OR Apache-2.0 (platform dep) |
| webkit2gtk | 2.0.2 | MIT |
| webkit2gtk-sys | 2.0.2 | MIT |
| webpki-root-certs | 1.0.8 | MPL-2.0 * |
| webview2-com | 0.38.2 | MIT * |
| webview2-com-macros | 0.8.1 | MIT * |
| webview2-com-sys | 0.38.2 | MIT * |
| weezl | 0.1.12 | MIT OR Apache-2.0 |
| which | 8.0.4 | MIT |
| whoami | 2.1.2 | Apache-2.0 OR BSL-1.0 OR MIT |
| winapi | 0.3.9 | MIT OR Apache-2.0 (platform dep) |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT OR Apache-2.0 (platform dep) |
| winapi-util | 0.1.11 | MIT OR Apache-2.0 (platform dep) |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT OR Apache-2.0 (platform dep) |
| window-vibrancy | 0.6.0 | MIT OR Apache-2.0 * |
| windows | 0.61.3 | MIT OR Apache-2.0 (platform dep) |
| windows | 0.62.2 | MIT OR Apache-2.0 (platform dep) |
| windows_aarch64_gnullvm | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_aarch64_gnullvm | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_aarch64_msvc | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_aarch64_msvc | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_gnu | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_gnu | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_gnullvm | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_msvc | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_i686_msvc | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_gnu | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_gnu | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_gnullvm | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_gnullvm | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_msvc | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows_x86_64_msvc | 0.53.1 | MIT OR Apache-2.0 (platform dep) |
| windows-collections | 0.2.0 | MIT OR Apache-2.0 (platform dep) |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| windows-core | 0.61.2 | MIT OR Apache-2.0 (platform dep) |
| windows-core | 0.62.2 | MIT OR Apache-2.0 (platform dep) |
| windows-future | 0.2.1 | MIT OR Apache-2.0 (platform dep) |
| windows-future | 0.3.2 | MIT OR Apache-2.0 (platform dep) |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 (platform dep) |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 (platform dep) |
| windows-link | 0.1.3 | MIT OR Apache-2.0 (platform dep) |
| windows-link | 0.2.1 | MIT OR Apache-2.0 (platform dep) |
| windows-numerics | 0.2.0 | MIT OR Apache-2.0 (platform dep) |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 (platform dep) |
| windows-registry | 0.6.1 | MIT OR Apache-2.0 (platform dep) |
| windows-result | 0.3.4 | MIT OR Apache-2.0 (platform dep) |
| windows-result | 0.4.1 | MIT OR Apache-2.0 (platform dep) |
| windows-strings | 0.4.2 | MIT OR Apache-2.0 (platform dep) |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 (platform dep) |
| windows-sys | 0.45.0 | MIT OR Apache-2.0 (platform dep) |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 (platform dep) |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 (platform dep) |
| windows-sys | 0.60.2 | MIT OR Apache-2.0 (platform dep) |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 (platform dep) |
| windows-targets | 0.42.2 | MIT OR Apache-2.0 (platform dep) |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 (platform dep) |
| windows-targets | 0.53.5 | MIT OR Apache-2.0 (platform dep) |
| windows-threading | 0.1.0 | MIT OR Apache-2.0 (platform dep) |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 (platform dep) |
| windows-version | 0.1.7 | MIT OR Apache-2.0 (platform dep) |
| winnow | 0.5.40 | MIT |
| winnow | 0.7.15 | MIT |
| winnow | 1.0.3 | MIT |
| winreg | 0.55.0 | MIT OR Apache-2.0 (platform dep) |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception * |
| wl-clipboard-rs | 0.9.3 | MIT OR Apache-2.0 |
| wnaf | 0.14.0 | Apache-2.0 OR MIT |
| writeable | 0.6.3 | Unicode-3.0 |
| wry | 0.55.1 | Apache-2.0 OR MIT |
| x11 | 2.21.0 | MIT |
| x11-dl | 2.21.0 | MIT |
| yoke | 0.8.3 | Unicode-3.0 |
| yoke-derive | 0.8.2 | Unicode-3.0 |
| zerocopy | 0.8.52 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.52 | BSD-2-Clause OR Apache-2.0 OR MIT * |
| zerofrom | 0.1.8 | Unicode-3.0 |
| zerofrom-derive | 0.1.7 | Unicode-3.0 |
| zeroize | 1.9.0 | Apache-2.0 OR MIT |
| zerotrie | 0.2.4 | Unicode-3.0 |
| zerovec | 0.11.6 | Unicode-3.0 |
| zerovec-derive | 0.11.3 | Unicode-3.0 |
| zip | 8.6.0 | MIT |
| zlib-rs | 0.6.5 | Zlib |
| zmij | 1.0.21 | MIT |
| zopfli | 0.8.3 | Apache-2.0 |
| zstd | 0.13.3 | MIT |
| zstd-safe | 7.2.4 | MIT OR Apache-2.0 |
| zstd-sys | 2.0.16+zstd.1.5.7 | MIT/Apache-2.0 |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |

---

## Full acknowledgements — npm packages (156)

Thank you to the authors of the following npm packages:

| Package | Version | License |
|---|---|---|
| @esbuild/linux-x64 | 0.25.12 | MIT |
| @jridgewell/gen-mapping | 0.3.13 | MIT |
| @jridgewell/remapping | 2.3.5 | MIT |
| @jridgewell/resolve-uri | 3.1.2 | MIT |
| @jridgewell/sourcemap-codec | 1.5.5 | MIT |
| @jridgewell/trace-mapping | 0.3.31 | MIT |
| @oxc-project/types | 0.138.0 | MIT |
| @polka/url | 1.0.0-next.29 | MIT |
| @rolldown/binding-linux-x64-gnu | 1.1.4 | MIT |
| @rolldown/pluginutils | 1.0.1 | MIT |
| @floating-ui/core | 1.8.0 | MIT |
| @floating-ui/dom | 1.8.0 | MIT |
| @floating-ui/utils | 0.2.12 | MIT |
| @rollup/rollup-linux-x64-gnu | 4.61.1 | MIT |
| @rollup/rollup-linux-x64-gnu | 4.62.2 | MIT |
| @standard-schema/spec | 1.1.0 | MIT |
| @sveltejs/acorn-typescript | 1.0.10 | MIT |
| @sveltejs/adapter-static | 3.0.10 | MIT |
| @sveltejs/kit | 2.63.0 | MIT |
| @sveltejs/kit | 2.69.1 | MIT |
| @sveltejs/load-config | 0.1.1 | MIT |
| @sveltejs/load-config | 0.2.0 | MIT |
| @sveltejs/vite-plugin-svelte | 5.1.1 | MIT |
| @sveltejs/vite-plugin-svelte | 7.1.2 | MIT |
| @sveltejs/vite-plugin-svelte-inspector | 4.0.1 | MIT |
| @tauri-apps/api | 2.11.0 | Apache-2.0 OR MIT |
| @tauri-apps/api | 2.11.1 | Apache-2.0 OR MIT |
| @tauri-apps/cli | 2.11.2 | Apache-2.0 OR MIT |
| @tauri-apps/cli | 2.11.4 | Apache-2.0 OR MIT |
| @tauri-apps/cli-linux-x64-gnu | 2.11.2 | Apache-2.0 OR MIT |
| @tauri-apps/cli-linux-x64-gnu | 2.11.4 | Apache-2.0 OR MIT |
| @tauri-apps/plugin-dialog | 2.7.1 | MIT OR Apache-2.0 |
| @tauri-apps/plugin-fs | 2.5.1 | MIT OR Apache-2.0 |
| @tauri-apps/plugin-opener | 2.5.4 | MIT OR Apache-2.0 |
| @tiptap/core | 3.31.3 | MIT |
| @tiptap/extension-blockquote | 3.31.3 | MIT |
| @tiptap/extension-bold | 3.31.3 | MIT |
| @tiptap/extension-bullet-list | 3.31.3 | MIT |
| @tiptap/extension-code | 3.31.3 | MIT |
| @tiptap/extension-code-block | 3.31.3 | MIT |
| @tiptap/extension-document | 3.31.3 | MIT |
| @tiptap/extension-dropcursor | 3.31.3 | MIT |
| @tiptap/extension-gapcursor | 3.31.3 | MIT |
| @tiptap/extension-hard-break | 3.31.3 | MIT |
| @tiptap/extension-heading | 3.31.3 | MIT |
| @tiptap/extension-horizontal-rule | 3.31.3 | MIT |
| @tiptap/extension-image | 3.31.3 | MIT |
| @tiptap/extension-italic | 3.31.3 | MIT |
| @tiptap/extension-link | 3.31.3 | MIT |
| @tiptap/extension-list | 3.31.3 | MIT |
| @tiptap/extension-list-item | 3.31.3 | MIT |
| @tiptap/extension-list-keymap | 3.31.3 | MIT |
| @tiptap/extension-ordered-list | 3.31.3 | MIT |
| @tiptap/extension-paragraph | 3.31.3 | MIT |
| @tiptap/extension-strike | 3.31.3 | MIT |
| @tiptap/extension-table | 3.31.3 | MIT |
| @tiptap/extension-task-item | 3.31.3 | MIT |
| @tiptap/extension-task-list | 3.31.3 | MIT |
| @tiptap/extension-text | 3.31.3 | MIT |
| @tiptap/extension-underline | 3.31.3 | MIT |
| @tiptap/extensions | 3.31.3 | MIT |
| @tiptap/markdown | 3.31.3 | MIT |
| @tiptap/pm | 3.31.3 | MIT |
| @tiptap/starter-kit | 3.31.3 | MIT |
| @types/cookie | 0.6.0 | MIT |
| @types/estree | 1.0.9 | MIT |
| @types/node | 25.9.1 | MIT |
| @types/node | 25.9.4 | MIT |
| @types/node | 26.1.0 | MIT |
| @types/trusted-types | 2.0.7 | MIT |
| @xterm/addon-fit | 0.10.0 | MIT |
| @xterm/addon-fit | 0.11.0 | MIT |
| @xterm/addon-web-links | 0.11.0 | MIT |
| @xterm/addon-web-links | 0.12.0 | MIT |
| @xterm/xterm | 5.5.0 | MIT |
| @xterm/xterm | 6.0.0 | MIT |
| acorn | 8.16.0 | MIT |
| acorn | 8.17.0 | MIT |
| aria-query | 5.3.1 | Apache-2.0 |
| axobject-query | 4.1.0 | Apache-2.0 |
| base64-arraybuffer | 1.0.2 | MIT |
| chokidar | 4.0.3 | MIT |
| clsx | 2.1.1 | MIT |
| cookie | 0.6.0 | MIT |
| css-line-break | 2.1.0 | MIT |
| debug | 4.4.3 | MIT |
| deepmerge | 4.3.1 | MIT |
| detect-libc | 2.1.2 | Apache-2.0 |
| devalue | 5.8.1 | MIT |
| esbuild | 0.25.12 | MIT |
| esm-env | 1.2.2 | MIT |
| esrap | 2.2.11 | MIT |
| esrap | 2.2.13 | MIT |
| fdir | 6.5.0 | MIT |
| html2canvas | 1.4.1 | MIT |
| is-reference | 3.0.3 | MIT |
| jsqr | 1.4.0 | Apache-2.0 |
| kleur | 4.1.5 | MIT |
| lightningcss | 1.32.0 | MPL-2.0 |
| lightningcss-linux-x64-gnu | 1.32.0 | MPL-2.0 |
| linkifyjs | 4.3.3 | MIT |
| locate-character | 3.0.0 | MIT |
| magic-string | 0.30.21 | MIT |
| marked | 17.0.6 | MIT |
| mri | 1.2.0 | MIT |
| mrmime | 2.0.1 | MIT |
| ms | 2.1.3 | MIT |
| nanoid | 3.3.12 | MIT |
| nanoid | 3.3.15 | MIT |
| obug | 2.1.3 | MIT |
| orderedmap | 2.1.1 | MIT |
| picocolors | 1.1.1 | ISC |
| picomatch | 4.0.4 | MIT |
| picomatch | 4.0.5 | MIT |
| postcss | 8.5.15 | MIT |
| postcss | 8.5.16 | MIT |
| prosemirror-changeset | 2.4.3 | MIT |
| prosemirror-commands | 1.7.2 | MIT |
| prosemirror-dropcursor | 1.8.3 | MIT |
| prosemirror-gapcursor | 1.4.1 | MIT |
| prosemirror-history | 1.5.0 | MIT |
| prosemirror-inputrules | 1.5.1 | MIT |
| prosemirror-keymap | 1.2.3 | MIT |
| prosemirror-model | 1.25.11 | MIT |
| prosemirror-schema-list | 1.5.1 | MIT |
| prosemirror-state | 1.4.4 | MIT |
| prosemirror-tables | 1.8.5 | MIT |
| prosemirror-transform | 1.12.1 | MIT |
| prosemirror-view | 1.42.4 | MIT |
| readdirp | 4.1.2 | MIT |
| rolldown | 1.1.4 | MIT |
| rollup | 4.61.1 | MIT |
| rollup | 4.62.2 | MIT |
| rope-sequence | 1.3.4 | MIT |
| sade | 1.8.1 | MIT |
| set-cookie-parser | 3.1.0 | MIT |
| set-cookie-parser | 3.1.1 | MIT |
| sirv | 3.0.2 | MIT |
| source-map-js | 1.2.1 | BSD-3-Clause |
| svelte | 5.56.2 | MIT |
| svelte | 5.56.4 | MIT |
| svelte-check | 4.6.0 | MIT |
| svelte-check | 4.7.1 | MIT |
| text-segmentation | 1.0.3 | MIT |
| tinyglobby | 0.2.17 | MIT |
| totalist | 3.0.1 | MIT |
| typescript | 5.6.3 | Apache-2.0 |
| typescript | 6.0.3 | Apache-2.0 |
| undici-types | 7.24.6 | MIT |
| undici-types | 8.3.0 | MIT |
| utrie | 1.0.2 | MIT |
| vite | 6.4.3 | MIT |
| vite | 8.1.3 | MIT |
| vitefu | 1.1.3 | MIT |
| w3c-keyname | 2.2.8 | MIT |
| zimmerframe | 1.1.4 | MIT |

---

## Fonts

Bundled with the UI, under the SIL Open Font License 1.1 (OFL-1.1):

- **Manrope** — @fontsource-variable/manrope
- **JetBrains Mono** — @fontsource-variable/jetbrains-mono
- **Noto Color Emoji** — @fontsource/noto-color-emoji
- **Noto Sans SC, Noto Sans JP, Noto Sans KR** — github.com/google/fonts, cut down to the characters of the UI's Chinese, Japanese and Korean strings (`ui/src/lib/core/fonts`, made by `scripts/fonts-cjk.mjs`); also in the web clipper of Veydan Space

---

## Icons

The brand icons of social networks in the profile (`ui/src/lib/core/Icon.svelte`, `brand-*`) are the paths of **simple-icons** 16.34.0 (LinkedIn from 13.21.0), https://simpleicons.org, under CC0-1.0. The names and logos are trademarks of their owners and only point to a person's page there.

---

## Full license texts

The complete text of each referenced license is available at:

- MIT — https://opensource.org/license/mit
- Apache-2.0 — https://www.apache.org/licenses/LICENSE-2.0
- BSD-2-Clause — https://opensource.org/license/bsd-2-clause
- BSD-3-Clause — https://opensource.org/license/bsd-3-clause
- ISC — https://opensource.org/license/isc-license-txt
- MPL-2.0 — https://www.mozilla.org/en-US/MPL/2.0/
- Zlib — https://opensource.org/license/zlib
- Unicode-3.0 — https://www.unicode.org/license.txt
- CC0-1.0 — https://creativecommons.org/publicdomain/zero/1.0/legalcode
- BSL-1.0 — https://www.boost.org/LICENSE_1_0.txt
- OFL-1.1 — https://openfontlicense.org/open-font-license-official-text/

Copies of the Apache-2.0 and MIT license texts, where required, are retained in
the corresponding package directories under `ui/node_modules/` and the Cargo
registry cache. This file is regenerated as dependencies change.

---

*Generated for Veydan Space. If you are an author of a listed project and
notice an attribution error, please open an issue — we want to credit you
correctly.*

# Third-party material

This file lists what the editor depends on or is derived from, the licence of each, and notes
for anyone who redistributes the editor. It is not legal advice.

The editor itself is licensed under **GPL-2.0-or-later** (`license` in `Cargo.toml`; the licence
text is in `LICENSE`, the same text as in the KeeperFX repository).

## 1. Rust dependencies

The list below covers every third-party package in `Cargo.lock` (522 packages; the 15 workspace
crates are not listed). The licence of each was read from the `license` field of the package's
own `Cargo.toml`. The lock file includes packages that only some platforms build (Windows, macOS,
Android, Redox, WebAssembly), packages behind optional features, and test-only packages
(`egui_kittest` and its dependencies), so a given build links fewer of them; to list what one
platform links, run `cargo tree -e normal --target <triple> -p kfx-editor`.

The main groups, by purpose:

* user interface and rendering: `egui`, `eframe`, `wgpu`, `winit`, `naga` and their platform
  crates (`wayland-*`, `x11rb`, `windows-*`, `objc2-*`), the accessibility stack (`accesskit*`,
  and on Linux `atspi` and `zbus` with their async runtime crates), `rfd` (file dialogs),
  `arboard` (clipboard);
* fonts: `egui_system_font_provider`, `fontique`, `skrifa`, `read-fonts`, and
  `yeslogic-fontconfig-sys`, which loads fontconfig at run time;
* audio: `symphonia` (decoding), `cpal` with `alsa`, CoreAudio and WASAPI bindings (output);
* Workshop: `ureq` with `rustls`, `ring` and `webpki-roots` (HTTPS), `url` and `idna` (with the
  ICU4X crates), `zip` with `bzip2`, `sevenz-rust2` and `lzma-rust2` (archives), `sha2`;
* data: `serde`, `serde_json`, `toml`, `toml_edit`, `ron`, `encoding_rs`, `image`, `png`,
  `directories`.

Licence expressions as declared by each package:

**MIT OR Apache-2.0** (239)

accesskit 0.25.2, accesskit_atspi_common 0.21.1, accesskit_consumer 0.39.2, accesskit_ios 0.2.2, accesskit_macos 0.27.2, accesskit_unix 0.24.1, accesskit_windows 0.35.2, ahash 0.8.12, allocator-api2 0.2.21, android-activity 0.6.1, android_system_properties 0.1.6, arbitrary 1.5.0, arboard 3.7.0, arrayvec 0.7.8, as-raw-xcb-connection 1.0.1, ash 0.38.0+1.3.281, async-broadcast 0.7.2, async-recursion 1.2.0, async-trait 0.1.92, base64 0.22.1, bitflags 2.13.2, block-buffer 0.10.4, bumpalo 3.20.3, bzip2 0.5.2, cc 1.11.1, cfg-if 1.0.5, chrono 0.4.45, core-foundation 0.9.4, core-foundation-sys 0.8.7, core-graphics 0.23.2, core-graphics-types 0.1.3, cpufeatures 0.2.17, crc32fast 1.5.5, crossbeam-utils 0.8.23, crypto-common 0.1.7, dasp_sample 0.11.0, deranged 0.5.9, derive_arbitrary 1.5.0, digest 0.10.7, directories 6.0.0, dirs-sys 0.5.0, displaydoc 0.2.7, document-features 0.2.12, ecolor 0.37.0, eframe 0.37.0, egui 0.37.0, egui-wgpu 0.37.0, egui-winit 0.37.0, egui_glow 0.37.0, egui_kittest 0.37.0, egui_system_font_provider 0.37.0, either 1.19.0, emath 0.37.0, enumflags2 0.7.12, enumflags2_derive 0.7.12, enumn 0.1.14, epaint 0.37.0, errno 0.3.14, euclid 0.22.14, fdeflate 0.3.7, find-msvc-tools 0.1.15, fixedbitset 0.5.9, flate2 1.1.11, font-types 0.12.7, form_urlencoded 1.2.2, futures-core 0.3.34, futures-io 0.3.34, futures-macro 0.3.34, futures-task 0.3.34, futures-util 0.3.34, getrandom 0.2.17, getrandom 0.3.4, getrandom 0.4.3, gif 0.14.2, glam 0.30.10, gpu-allocator 0.28.0, half 2.7.1, hashbrown 0.15.5, hashbrown 0.16.1, hashbrown 0.17.1, hermit-abi 0.5.3, hex 0.4.3, idna 1.1.0, image 0.25.10, itertools 0.15.0, itoa 1.0.19, jni 0.22.4, jni-macros 0.22.4, jni-sys 0.3.1, jni-sys 0.4.1, jni-sys-macros 0.4.1, jobserver 0.1.35, js-sys 0.3.107, kittest 0.5.0, lazy_static 1.5.1, libc 0.2.191, litrs 1.0.1, lock_api 0.4.14, log 0.4.35, memmap2 0.9.12, naga 30.0.1, naga-types 30.0.1, ndk 0.9.0, ndk-context 0.1.1, ndk-sys 0.6.0+11769913, num-conv 0.2.2, num-derive 0.4.2, num-traits 0.2.19, once_cell 1.21.4, ordered-stream 0.2.0, parking_lot 0.12.5, parking_lot_core 0.9.12, percent-encoding 2.3.2, petgraph 0.8.3, piper 0.2.5, pkg-config 0.3.34, png 0.17.16, png 0.18.1, poll-promise 0.3.0, polycool 0.4.0, powerfmt 0.2.1, ppv-lite86 0.2.21, presser 0.3.1, proc-macro-crate 3.5.0, proc-macro2 1.0.107, profiling 1.0.18, quote 1.0.47, rand 0.9.5, rand_chacha 0.9.0, rand_core 0.9.5, range-alloc 0.1.5, raw-window-metal 1.1.0, read-fonts 0.41.0, renderdoc-sys 1.1.0, ron 0.12.3, roxmltree 0.21.2, rustc_version 0.4.1, rustls-pki-types 1.16.0, rustversion 1.0.23, scopeguard 1.2.0, semver 1.0.28, serde 1.0.229, serde_core 1.0.229, serde_derive 1.0.229, serde_json 1.0.154, serde_repr 0.1.21, serde_spanned 0.6.9, serde_spanned 1.1.3, sha2 0.10.9, shlex 2.0.1, signal-hook-registry 1.4.8, simdutf8 0.1.5, siphasher 1.0.4, skrifa 0.44.0, smallvec 1.16.5, smol_str 0.2.2, stable_deref_trait 1.2.1, static_assertions 1.1.0, syn 2.0.119, syn 3.0.10, sys-locale 0.3.2, tempfile 3.27.0, thiserror 1.0.69, thiserror 2.0.21, thiserror-impl 1.0.69, thiserror-impl 2.0.21, time 0.3.55, time-core 0.1.9, time-macros 0.2.32, toml 0.8.23, toml 1.2.1+spec-1.1.0, toml_datetime 0.6.11, toml_datetime 1.1.3+spec-1.1.0, toml_edit 0.22.27, toml_edit 0.25.19+spec-1.1.0, toml_parser 1.1.5+spec-1.1.0, toml_write 0.1.2, typeid 1.0.3, typenum 1.20.1, unicode-script 0.5.9, unicode-segmentation 1.13.3, unicode-width 0.2.3, ureq 2.12.1, url 2.5.8, wasm-bindgen 0.2.130, wasm-bindgen-futures 0.4.80, wasm-bindgen-macro 0.2.130, wasm-bindgen-macro-support 0.2.130, wasm-bindgen-shared 0.2.130, web-sys 0.3.107, web-time 1.1.0, weezl 0.1.12, wgpu 30.0.1, wgpu-core 30.0.1, wgpu-core-deps-apple 30.0.1, wgpu-core-deps-emscripten 30.0.1, wgpu-core-deps-wasm 30.0.1, wgpu-core-deps-windows-linux-android 30.0.1, wgpu-hal 30.0.1, wgpu-naga-bridge 30.0.1, wgpu-types 30.0.1, windows 0.54.0, windows 0.62.2, windows-collections 0.3.2, windows-core 0.54.0, windows-core 0.62.2, windows-future 0.3.2, windows-implement 0.60.2, windows-interface 0.59.3, windows-link 0.2.1, windows-numerics 0.3.1, windows-result 0.1.2, windows-result 0.4.1, windows-strings 0.5.1, windows-sys 0.45.0, windows-sys 0.52.0, windows-sys 0.59.0, windows-sys 0.61.2, windows-targets 0.42.2, windows-targets 0.52.6, windows-threading 0.2.1, windows_aarch64_gnullvm 0.42.2, windows_aarch64_gnullvm 0.52.6, windows_aarch64_msvc 0.42.2, windows_aarch64_msvc 0.52.6, windows_i686_gnu 0.42.2, windows_i686_gnu 0.52.6, windows_i686_gnullvm 0.52.6, windows_i686_msvc 0.42.2, windows_i686_msvc 0.52.6, windows_x86_64_gnu 0.42.2, windows_x86_64_gnu 0.52.6, windows_x86_64_gnullvm 0.42.2, windows_x86_64_gnullvm 0.52.6, windows_x86_64_msvc 0.42.2, windows_x86_64_msvc 0.52.6, wl-clipboard-rs 0.9.5, x11rb 0.13.2, x11rb-protocol 0.13.2

**MIT** (94)

alsa-sys 0.3.1, android-properties 0.2.2, block2 0.5.1, block2 0.6.2, bytes 1.12.1, calloop 0.13.0, calloop-wayland-source 0.3.0, cfg_aliases 0.2.2, color_quant 1.1.0, combine 4.6.9, crunchy 0.2.4, dispatch 0.2.0, dlib 0.5.3, endi 1.1.1, extended 0.1.0, fax 0.2.7, generic-array 0.14.7, glutin-winit 0.5.0, harfrust 0.12.0, libm 0.2.16, libredox 0.1.25, memoffset 0.9.1, nom 8.0.0, objc-sys 0.3.5, objc2 0.5.3, objc2 0.6.5, objc2-app-kit 0.2.2, objc2-cloud-kit 0.2.2, objc2-contacts 0.2.2, objc2-core-data 0.2.2, objc2-core-image 0.2.2, objc2-core-location 0.2.2, objc2-encode 4.1.0, objc2-foundation 0.2.2, objc2-foundation 0.3.2, objc2-link-presentation 0.2.2, objc2-metal 0.2.2, objc2-quartz-core 0.2.2, objc2-symbols 0.2.2, objc2-ui-kit 0.2.2, objc2-uniform-type-identifiers 0.2.2, objc2-user-notifications 0.2.2, orbclient 0.3.55, ordered-float 5.5.1, os_pipe 1.2.3, phf 0.14.0, phf_generator 0.14.0, phf_macros 0.14.0, phf_shared 0.14.0, quick-xml 0.41.0, redox_syscall 0.4.1, redox_syscall 0.5.18, redox_syscall 0.9.5, redox_users 0.5.3, rfd 0.18.0, simd-adler32 0.3.10, slab 0.4.12, smithay-client-toolkit 0.19.2, synstructure 0.14.0, tiff 0.11.3, tokio 1.54.1, tracing 0.1.44, tracing-attributes 0.1.31, tracing-core 0.1.36, tree_magic_mini 3.2.2, uds_windows 1.2.1, wayland-backend 0.3.17, wayland-client 0.31.15, wayland-csd-frame 0.3.0, wayland-cursor 0.31.14, wayland-protocols 0.32.13, wayland-protocols-plasma 0.3.12, wayland-protocols-wlr 0.3.12, wayland-scanner 0.31.11, wayland-sys 0.31.11, winnow 0.7.15, winnow 1.0.5, x11-dl 2.21.0, xcursor 0.3.11, xkbcommon-dl 0.4.2, xml-rs 0.8.29, yeslogic-fontconfig-sys 6.0.1, zbus 5.19.0, zbus-lockstep 0.5.2, zbus-lockstep-macros 0.5.2, zbus_macros 5.19.0, zbus_names 4.3.5, zbus_xml 5.2.1, zcheapstr 1.2.0, zip 2.6.1, zmij 1.0.23, zvariant 5.15.1, zvariant_derive 5.15.1, zvariant_utils 4.2.0

**Apache-2.0 OR MIT** (51)

async-channel 2.5.0, async-executor 1.14.1, async-io 2.6.0, async-lock 3.5.0, async-process 2.5.0, async-signal 0.2.14, async-task 4.7.1, atomic-waker 1.1.2, atspi 0.29.0, atspi-common 0.13.0, atspi-proxies 0.13.0, autocfg 1.5.1, bit-set 0.10.0, bit-set 0.8.0, bit-vec 0.8.0, bit-vec 0.9.1, blocking 1.7.0, color 0.3.3, concurrent-queue 2.5.0, equivalent 1.0.3, event-listener 5.4.2, event-listener-strategy 0.5.4, fastrand 2.5.0, fearless_simd 0.4.1, fontique 0.11.1, futures-lite 2.6.1, glifo 0.3.0, idna_adapter 1.2.2, indexmap 2.14.2, kurbo 0.13.1, linebender_resource_handle 0.1.1, multiversion_no_op 1.0.0, nohash-hasher 0.2.0, nt-time 0.11.2, parking 2.2.1, parlance 0.1.1, peniko 0.6.1, pin-project 1.1.13, pin-project-internal 1.1.13, pin-project-lite 0.2.17, polling 3.11.0, portable-atomic 1.15.0, portable-atomic-util 0.2.8, rustc-hash 2.1.3, simd_cesu8 1.2.0, utf8_iter 1.1.0, utf_types 1.0.0, uuid 1.29.2, vello_common 0.2.0, vello_cpu 0.2.0, zeroize 1.9.1

**Unicode-3.0** (18)

icu_collections 2.4.0, icu_locale_core 2.4.0, icu_normalizer 2.4.0, icu_normalizer_data 2.4.0, icu_properties 2.4.0, icu_properties_data 2.4.0, icu_provider 2.4.0, litemap 0.8.3, potential_utf 0.1.6, tinystr 0.8.5, writeable 0.6.4, yoke 0.8.4, yoke-derive 0.8.4, zerofrom 0.1.8, zerofrom-derive 0.1.8, zerotrie 0.2.5, zerovec 0.11.9, zerovec-derive 0.11.7

**MIT/Apache-2.0** (17)

bitflags 1.3.2, bzip2-sys 0.1.13+1.0.8, core_detect 1.0.0, coreaudio-rs 0.13.0, downcast-rs 1.2.1, foreign-types 0.5.0, foreign-types-macros 0.2.4, foreign-types-shared 0.3.1, guillotiere 0.7.0, jni 0.21.1, khronos-egl 6.0.0, plain 0.2.3, quick-error 2.0.1, scoped-tls 1.0.1, send_wrapper 0.6.0, type-map 0.5.1, version_check 0.9.5

**Apache-2.0** (16)

accesskit_winit 0.34.2, codespan-reporting 0.13.1, cpal 0.16.0, gethostname 1.1.0, gl_generator 0.14.0, glutin 0.32.3, glutin_egl_sys 0.7.1, glutin_glx_sys 0.6.1, glutin_wgl_sys 0.6.1, khronos_api 3.1.0, lzma-rust2 0.2.2, sevenz-rust2 0.13.2, spirv 0.4.0+sdk-1.4.341.0, unicode-general-category 1.1.0, winit 0.30.13, zopfli 0.8.4

**Zlib OR Apache-2.0 OR MIT** (15)

bytemuck 1.25.2, bytemuck_derive 1.12.1, dispatch2 0.3.1, objc2-app-kit 0.3.2, objc2-audio-toolbox 0.3.2, objc2-core-audio 0.3.2, objc2-core-audio-types 0.3.2, objc2-core-foundation 0.3.2, objc2-core-graphics 0.3.2, objc2-core-text 0.3.2, objc2-io-surface 0.3.2, objc2-metal 0.3.2, objc2-quartz-core 0.3.2, objc2-ui-kit 0.3.2, objc2-uniform-type-identifiers 0.3.2

**MPL-2.0** (12)

option-ext 0.2.0, symphonia 0.5.5, symphonia-bundle-flac 0.5.5, symphonia-bundle-mp3 0.5.5, symphonia-codec-adpcm 0.5.5, symphonia-codec-pcm 0.5.5, symphonia-codec-vorbis 0.5.5, symphonia-core 0.5.5, symphonia-format-ogg 0.5.5, symphonia-format-riff 0.5.5, symphonia-metadata 0.5.5, symphonia-utils-xiph 0.5.5

**Unlicense OR MIT** (8)

byteorder 1.5.0, byteorder-lite 0.1.0, jiff 0.2.43, jiff-core 0.1.5, jiff-static 0.2.43, memchr 2.8.3, termcolor 1.4.1, winapi-util 0.1.11

**Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT** (7)

linux-raw-sys 0.12.1, linux-raw-sys 0.4.15, rustix 0.38.44, rustix 1.1.6, wasi 0.11.1+wasi-snapshot-preview1, wasip2 1.0.4+wasi-0.2.12, wit-bindgen 0.57.1

**MIT OR Apache-2.0 OR Zlib** (6)

cursor-icon 1.2.0, glow 0.17.0, raw-window-handle 0.6.2, xkeysym 0.2.1, zune-core 0.5.3, zune-jpeg 0.5.15

**Apache-2.0/MIT** (4)

alsa 0.9.1, cesu8 1.1.0, pollster 1.0.1, rustc-hash 1.1.0

**Zlib** (4)

foldhash 0.1.5, foldhash 0.2.0, slotmap 1.1.1, zlib-rs 0.6.8

**ISC** (3)

libloading 0.8.9, rustls-webpki 0.103.16, untrusted 0.9.0

**(MIT OR Apache-2.0) AND Unicode-3.0** (2)

unicode-ident 1.0.27, unicode-properties 0.1.5

**BSD-2-Clause OR Apache-2.0 OR MIT** (2)

zerocopy 0.8.63, zerocopy-derive 0.8.63

**BSD-3-Clause OR Apache-2.0** (2)

moxcms 0.8.1, pxfm 0.1.30

**BSD-3-Clause OR MIT OR Apache-2.0** (2)

num_enum 0.7.6, num_enum_derive 0.7.6

**BSL-1.0** (2)

clipboard-win 5.4.3, error-code 3.4.0

**CDLA-Permissive-2.0** (2)

webpki-roots 0.26.11, webpki-roots 1.0.12

**MIT OR Apache-2.0 OR LGPL-2.1-or-later** (2)

r-efi 5.3.0, r-efi 6.0.0

**MIT OR Zlib OR Apache-2.0** (2)

miniz_oxide 0.8.9, miniz_oxide 0.9.1

**Unlicense/MIT** (2)

same-file 1.0.6, walkdir 2.5.0

**(Apache-2.0 OR MIT) AND BSD-3-Clause** (1)

encoding_rs 0.8.44

**(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0** (1)

epaint_default_fonts 0.37.0

**0BSD OR MIT OR Apache-2.0** (1)

adler2 2.0.1

**Apache-2.0 AND ISC** (1)

ring 0.17.14

**Apache-2.0 AND MIT** (1)

dpi 0.1.2

**Apache-2.0 OR GPL-2.0-only** (1)

self_cell 1.3.0

**Apache-2.0 OR ISC OR MIT** (1)

rustls 0.23.46

**BSD-2-Clause OR MIT OR Apache-2.0** (1)

mach2 0.4.3

**BSD-3-Clause** (1)

subtle 2.6.1

**MIT / Apache-2.0** (1)

cgl 0.3.2

### Notes for redistributors

* Every package declares a licence, and none is under a licence that forbids distribution
  together with GPL-2.0-or-later code.
* **Apache-2.0 without an alternative**: `winit`, `glutin` (with `glutin_egl_sys`,
  `glutin_glx_sys`, `glutin_wgl_sys`), `accesskit_winit`, `cpal`, `sevenz-rust2`, `lzma-rust2`,
  `codespan-reporting`, `gethostname`, `gl_generator`, `khronos_api`, `spirv`,
  `unicode-general-category` and `zopfli`; `dpi` (`Apache-2.0 AND MIT`) and `ring`
  (`Apache-2.0 AND ISC`) likewise. Apache-2.0 is compatible with GPL version 3 but not with
  GPL version 2 only. Because the editor is GPL-2.0-or-later, a binary that includes these
  packages can be distributed under the terms of GPL-3.0-or-later; it should not be described
  as GPL-2.0-only. The KeeperFX source the editor derives from is licensed "version 2 of the
  License, or (at your option) any later version", which allows this.
* **Fonts**: `epaint_default_fonts` (`(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`)
  embeds egui's default fonts (Ubuntu Light, Hack, Noto Emoji and an emoji icon font, each with
  its licence file in the package), and the `monochrome_emoji_fonts` feature of `egui` adds more
  font data. The SIL Open Font License and the Ubuntu Font Licence apply to the font data, which
  is included unmodified as a resource; their texts and copyright notices must accompany a
  binary distribution. The editor bundles no other fonts: `kfx-fonts` uses the fonts installed on
  the system.
* **MPL-2.0**: `symphonia` and its codec and format packages, and `option-ext`. MPL-2.0 is a
  file-level copyleft that allows combination with GPL code (section 3.3); none of these packages
  carries an "Incompatible With Secondary Licenses" notice. Modified files of these packages
  would stay under MPL-2.0, and their source must remain available.
* **CDLA-Permissive-2.0**: `webpki-roots` (the Mozilla root certificate list used for HTTPS). Its
  licence text must accompany redistribution.
* **Permissive licences with notice requirements**: MIT, Apache-2.0, BSD-3-Clause (`subtle`, and a
  part of `encoding_rs`), ISC (`libloading`, `rustls-webpki`, `untrusted`), BSL-1.0
  (`error-code`, `clipboard-win`), Zlib, Unicode-3.0 (the ICU4X crates). Their copyright and
  licence notices must be reproduced in binary distributions where the licence asks for it.
* **Dual licences with a GPL or LGPL option**: `self_cell` (`Apache-2.0 OR GPL-2.0-only`) and
  `r-efi` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`); either can be used under one of its
  permissive or GPL-compatible options.
* This file is not a combined notice file. A binary release should include the licence texts of
  its dependencies, for example generated with `cargo about`.

## 2. Material derived from KeeperFX

The files in `data/` and the generated Rust tables are extracted from the
[KeeperFX](https://github.com/dkfans/keeperfx) project and embedded in the binary. KeeperFX is
licensed under the GPL, version 2 or (at the user's option) any later version, so the tables
extracted from its source and the code ported from it are distributed under the same terms.

| Material | Source | Licence |
|---|---|---|
| `data/script_tables.json` | `src/lvl_script_commands.c`, `src/lvl_script_lib.h`, `src/map_locations.c`, `src/config_campaigns.c` | GPL-2.0-or-later (KeeperFX) |
| `data/script_c_comments.json` | comments of `src/lvl_script_commands.c` | GPL-2.0-or-later (KeeperFX) |
| `data/config_schema.json` | the named-field tables and loaders of `src/config_*.c`, comments of the shipped config files, descriptions from the KeeperFX wiki | GPL-2.0-or-later for the parts from the source and the shipped files; see the wiki note below for the descriptions |
| `data/lua_c_api.json` | `src/lua_api*.c`, `src/lua_params.c`, `src/lua_triggers.c` | GPL-2.0-or-later (KeeperFX) |
| `data/lua_api.json` | index of the Lua stub files that KeeperFX ships in `fxdata/lua` (from `config/fxdata/lua` of the repository) | GPL-2.0-or-later (KeeperFX) |
| `data/script_arg_overlay.json` | written by hand from the `*_check` functions of `src/lvl_script_commands.c` | derived from GPL-2.0-or-later source |
| `crates/kfx-gfx/src/gen_creature_list.rs`, `gen_sprite_names.rs` | `src/creature_graphics.c`, `src/creature_graphics.h`, `src/sprites.h` | GPL-2.0-or-later (KeeperFX) |
| `crates/kfx-editor/src/fptd.rs` | the first-person to top-down animation table of `src/engine_arrays.c` | GPL-2.0-or-later (KeeperFX) |
| `crates/kfx-map` (slab placement, columns, doors, validation and other rules) | ports of KeeperFX logic (`src/map_blocks.c`, `src/map_columns.c`, `src/thing_doors.c`, ...); each function cites the C function it follows | derivative work of GPL-2.0-or-later code |
| `crates/kfx-formats/src/rnc.rs` | port of `bflib_dernc.c` (RNC method 1 unpacker), whose header names Tomasz Lis and Jon Skeet as authors | derivative work of GPL-2.0-or-later code |

### Note: documentation text from the KeeperFX wiki

`data/script_docs.json` holds the script command documentation of the
[KeeperFX wiki](https://github.com/dkfans/keeperfx/wiki) (repository
`https://github.com/dkfans/keeperfx.wiki`, commit `c3461a0443dd60a2ac95302034dce5fd55a04225`,
recorded in the file's `source` field). The editor shows it in hover texts and the command
reference. `data/config_schema.json` contains configuration key descriptions from the same wiki.
The wiki does not state a licence (the extractor records `license_note: "not stated"`). Anyone
redistributing the editor should be aware that the licence of this text has not been stated by
its authors, the KeeperFX contributors.

## 3. Unearth

[Unearth](https://github.com/rainlizard/Unearth) is a map editor for Dungeon Keeper and KeeperFX
by rainlizard, licensed under the MIT licence. The editor reads Unearth's `.une` custom slab files
and its `custom_slabs.cfg`, and keeps a `.une` up to date when it saves a map that has one
(`crates/kfx-map/src/unearth.rs`). These formats were implemented from their file layouts; the
editor includes no Unearth source code. If Unearth material is ever included, its licence text
must be added.

## 4. Documentation images

The pictures in `docs/manual/img/` were made with the editor's headless screenshot mode on a
KeeperFX install (mostly the first level of the original campaign, "Eversmile"). They show
Dungeon Keeper game art (terrain textures, creature and object sprites, room and creature icons,
the land view) and level script text, which belong to their owners; they are included only to
illustrate the editor. The level script visible in one picture carries the GPL notice of the
KeeperFX project. Remove the pictures from a distribution where their use is not acceptable.

## 5. What the editor does not contain

The editor ships no game assets and no sounds, and no fonts of its own beyond egui's default
fonts. It reads palettes, textures, sprites, sounds and configuration from the user's own
KeeperFX install at run time, and never copies them into this repository or into the maps it
writes.

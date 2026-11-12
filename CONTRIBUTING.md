# Contributing

Bug reports, fixes and improvements are welcome. This file explains how to build and test the
editor, how the code is laid out, the conventions the code follows, and how the generated data is
made.

## Building

The workspace is a plain Cargo workspace. You need Rust 1.99 or newer (`rust-version` in
`Cargo.toml`); the workspace uses edition 2021.

On Linux the build needs the development packages of the window system and of ALSA (sound
previews). On Debian and Ubuntu, as in CI:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev libx11-dev libasound2-dev
```

fontconfig is loaded at run time for fallback fonts; it is not needed to build.

```sh
cargo build -p kfx-editor              # debug build
cargo build --release -p kfx-editor    # release build: target/release/kfx-editor
```

The full workspace is large; if your machine is shared or small, limit the parallel jobs with
`-j N`.

## Running

```sh
KFX_INSTALL=/path/to/keeperfx cargo run --release -p kfx-editor -- [path/to/map00001.slb]
cargo run -p kfx-editor -- --help
```

`KFX_INSTALL` overrides the KeeperFX folder saved in the settings. `KFX_EDITOR_CONFIG_DIR` points
the editor at another settings folder, which keeps experiments, headless runs and tests away from
your own settings, backups and autosaves.

## Testing

```sh
cargo test --workspace
cargo test -p kfx-map             # one crate
cargo fmt --all --check           # formatting, as checked in CI
```

The suite is meant to pass on a machine without any game data: tests that need data they cannot
find return early and pass. The following environment variables enable more of it.

### `KFX_INSTALL`: a KeeperFX install as test data

Set it to a complete KeeperFX install (with the original game data). It is only read; tests copy
whatever they modify into a scratch folder. Never copy files from it into the repository: they
are copyrighted.

Without it these tests skip:

* round trips of the binary and FX level files over every map of the install;
* corpus tests over everything shipped: every level script and Lua file parsed and checked, the
  validator run over every shipped map, damaged copies of real files loaded without a panic;
* the autotile golden test, which regenerates the columns of every shipped map and compares them
  with the shipped `.clm` / `.dat`;
* tests that load the install's configuration, sprites, textures, sound banks, campaigns and land
  views;
* most UI tests of the editor and of the widget crates, which open real maps;
* the Workshop packaging and installation tests on a private copy of the install.

### `KFX_GAME_TEST=1`: tests with the real game

These tests save maps with the editor's code, start the actual game on them and read its log
(`keeperfx.log`), comparing it with the log of an unmodified map. They need `KFX_INSTALL` to hold a
**native Linux build of the game** (an executable named `keeperfx`; the harness does not use
`wine`) and run on Unix only. The game runs with SDL's offscreen video driver and dummy audio, so
no window opens and no sound plays; only one game runs at a time, with a hard timeout. Each test
builds a private working copy of the install (symbolic links to the read-only folders, a fresh
`levels/personal`) in the `kfx-map-gametest` folder of the test scratch root (see `KFX_TEST_TMP`
below), or under `KFX_GAME_RIG_ROOT` when that is set; the install itself is never written. The
harness is in `crates/kfx-map/tests/support/game.rs`.

Game tests exist in `kfx-map` (for example `cargo test -p kfx-map --test game_load`),
`kfx-editor`, `kfx-analysis`, `kfx-config-ui`, `kfx-campaign-ui`, `kfx-logic-ui`, `kfx-prefab`
and `kfx-live`. `KFX_GAME_ONLY=<text>` limits the `game_load` cases to those whose name contains
the text.

### `KFX_SOURCE`: the KeeperFX C source

A checkout of the [KeeperFX repository](https://github.com/dkfans/keeperfx). The tests that read
it (the Lua corpus test over the Lua files of the source tree in `kfx-script`, and the creature
configuration tests in `kfx-map`) skip without it, and the generator scripts in `tools/` take it
as input.

### Other variables

| Variable | Used for |
|---|---|
| `KFX_TEST_TMP` | Root of the scratch folders of tests and examples. Default: `target/tmp` of the build. |
| `KFX_WORKSHOP_SURVEY` | A folder of downloaded Workshop archives (in a `files/` subfolder); the Workshop classification test runs over every archive in it. `KFX_SURVEY_MAX_BYTES` caps the archive size it reads. |
| `KFX_WORKSHOP_CACHE` | The Workshop client's HTTP cache folder, for the Workshop screenshot test and the ignored test that fills the cache from the live site. |
| `KFX_LUA_EXTRA_DIR` | Extra Lua files to lex and parse (for example the `meta` stubs of lua-language-server). |
| `KFX_LUA_REGEN=1` | Rewrite `data/lua_api.json` instead of checking it (see below). |
| `KFX_EDITOR_GAME_MAP` | A `mapNNNNN.slb` saved by the editor window, to load in the editor's game test. |
| `KFX_SWEEP=1` | Open every map of every campaign and pack in the editor's sweep test (by default every seventh). |
| `KFX_SHOTS_DIR` | Where the editor's UI tests write their screenshots. |
| `KFX_GEN_SEEDS` | Number of seeds the map generator tests try. |
| `KFX_FUZZ_ITERS`, `KFX_FUZZ_SEED`, `KFX_FUZZ_SECS` | Iterations, seed and time budget of the `kfx-map` fuzz tests. |
| `KFX_GOLDEN_DETAIL`, `KFX_GOLDEN_ONLY`, `KFX_GOLDEN_KINDS`, `KFX_GOLDEN_CLASS`, `KFX_GOLDEN_OLD_SLABSET` | Diagnostics of the autotile golden test (see the header of `crates/kfx-map/tests/golden.rs`). |
| `KFX_SCRIPT_UNION`, `KFX_SCRIPT_VERBOSE` | Variants and detail of the level script corpus test. |
| `KFX_AUDIO_PLAYBACK_TEST=1` | Open a real audio output in the playback test (it plays at volume 0). Off by default. |
| `KFX_DUMP_SPRITES=1` | Write a few decoded sprite frames as PNG files (a debugging aid of the editor's sprite tests). |

### UI tests and the GPU

The UI tests use `egui_kittest` and render with wgpu, so they need a graphics adapter. A machine
without a GPU can use a software Vulkan driver; the Linux CI job installs Mesa's lavapipe
(`mesa-vulkan-drivers`, `libvulkan1`). Tests that render skip when no adapter is found.

### CI

`.github/workflows/ci.yml` checks formatting, builds release executables for Linux x86_64,
Windows x86_64 and macOS (arm64 and x86_64), and runs the whole test suite on Linux, Windows and
macOS, without a KeeperFX install. `.github/workflows/release.yml` builds the release archives when
a `v*` tag is pushed.

## Code layout

```text
crates/      the workspace crates (below)
data/        generated reference data embedded in the binary, the starter prefabs, the UI catalogue
tools/       Python scripts that extract data/ and two Rust tables from the KeeperFX source
docs/        user manual (docs/manual), file formats, architecture, features, roadmap
packaging/   desktop entry, AppStream metadata, MIME type, Arch PKGBUILD, Windows and macOS notes
```

| Crate | Role |
|---|---|
| `kfx-formats` | Readers and writers of the KeeperFX file formats: RNC, the binary level files, the TOML-based FX files, `.lof` / `.lif`, campaign files, the `.cfg` dialect, string tables, `translation.toml`; crash-safe file writes. No GUI, no game logic. |
| `kfx-gfx` | Decoders for palettes, texture atlases, keeper sprites, sprite sheets and custom sprite archives, to RGBA. |
| `kfx-map` | The map document: slab placement (a port of the game's `src/map_blocks.c`), columns, decoration, things, transforms, undo patches, validation, configuration layers and mods, the editor's sidecar file, `.une` import, legacy output. |
| `kfx-script` | DK level script and Lua tooling without UI: parsing, diagnostics, completion, hover, outline, formatting, the Lua API index. |
| `kfx-script-ui` | The egui script editor widget and the per-level script workspace. |
| `kfx-config-ui` | egui editors for configuration overrides (rules, creatures, terrain, slab sets, column sets, cubes, tilesets, sprites, sounds). |
| `kfx-logic-ui` | The visual Events editor for DK scripts: model, import / export, analysis, Lua export, waves, egui widget. |
| `kfx-campaign-ui` | Campaign and map pack model, validation and egui editor. |
| `kfx-analysis` | Map analysis (reachability, distances, economy, fairness, symmetry, heroes) and the map generator. |
| `kfx-prefab` | Prefabs (`*.kfxprefab`), the level text translation grid, and the editor's own i18n runtime and string scanner. |
| `kfx-workshop` | KeeperFX Workshop client, safe archive reading, installer and packager, with an egui browser. |
| `kfx-audio` | Sound banks, sound resolution as the game does it, decoding, preview playback and the sound browser. |
| `kfx-live` | Client for the game's TCP JSON API and the Live link panel. |
| `kfx-fonts` | System fallback fonts for egui; no bundled fonts. |
| `kfx-editor` | The application: window, renderer, tools, panels, play-testing, settings, the command line and the headless mode. |

[docs/architecture.md](docs/architecture.md) describes how the crates fit together and how the
slab placement port is validated; [docs/formats.md](docs/formats.md) documents the file formats.

## Conventions

* **No `unsafe` code.**
* **Malformed input never panics.** Readers return `Result` with a message saying what is wrong;
  a file the editor cannot read is reported, not silently replaced.
* **Integers in files are little-endian** unless the format says otherwise.
* **Every file is loaded through `kfx_formats::rnc::load_file`**, because any game file may be
  RNC-packed.
* **File names in an install are resolved case-insensitively** (`MAP00007.SLB` and
  `map00001.slb` both occur); use the existing helpers (`kfx_gfx::install::resolve`,
  `kfx_map::gamedata::find_ci`, and `kfx_formats::testutil::resolve_ci` in tests) rather than
  joining paths directly.
* **Rules taken from the game cite the KeeperFX C source** in their doc comments, as a path in the
  KeeperFX repository with a line number, for example `src/map_blocks.c:1646`. Match the game's
  behaviour, quirks included, and say so where the editor deliberately differs.
* **Saves are atomic.** Write files through `kfx_formats::atomic` (`atomic_write` for one file,
  `SaveTransaction` for the files of a level), never with a plain `std::fs::write` over an
  existing file, and back files up before their first overwrite in a session.
* **Formatting** follows `rustfmt.toml` (`cargo fmt --all`). Generated sources are excluded with
  `#[rustfmt::skip]` on their module declarations.
* **User interface texts** go through `kfx_prefab::i18n`: `trd("crate.key", "English text")`,
  `trf` with `{name}` placeholders, `trn` for plurals. After adding or changing a text,
  regenerate the English catalogue (a test fails when it is out of date):

  ```sh
  cargo run -p kfx-prefab --example extract_strings
  ```

  When the meaning of a text changes, give it a new key so that old translations are not used for
  it. [docs/manual/languages.md](docs/manual/languages.md) describes the catalogue format.

## Generated data

The files in `data/` are generated from the KeeperFX source, its wiki or an install, checked in
and embedded in the binary. Regenerate them when KeeperFX changes, from the repository root. The
scripts only read their inputs and use the Python standard library. `KFX_SOURCE` is a checkout of
the KeeperFX C repository, `KFX_WIKI` the folder of a checkout of
`https://github.com/dkfans/keeperfx.wiki` that holds the Markdown pages, and `KFX_INSTALL` a
KeeperFX install.

| File | Content | Command |
|---|---|---|
| `data/script_tables.json` | Level script commands, argument types, constants and named tables | `python3 -I tools/extract_script_tables.py "$KFX_SOURCE" data/script_tables.json` |
| `data/script_docs.json` | Documentation of each script command, from the wiki | `python3 -I tools/extract_script_docs.py "$KFX_WIKI" data/script_tables.json data/script_docs.json` |
| `data/script_c_comments.json` | The C source comments of each script command | `python3 -I tools/extract_script_c_comments.py "$KFX_SOURCE/src" data/script_c_comments.json` |
| `data/config_schema.json` | Schema of the `.cfg` and `.toml` configuration files, from the C loaders, the shipped files and the wiki | `python3 -I tools/extract_config_schema.py --src "$KFX_SOURCE" --install "$KFX_INSTALL" --wiki "$KFX_WIKI" --out data/config_schema.json` |
| `data/lua_c_api.json` | The Lua API the C code registers | `python3 -I tools/extract_lua_api.py "$KFX_SOURCE" data/lua_c_api.json` |
| `data/lua_api.json` | Index of the Lua annotation stubs shipped in an install's `fxdata/lua` | `KFX_LUA_REGEN=1 KFX_INSTALL=... cargo test -p kfx-script --test lua_index_gen` (without `KFX_LUA_REGEN` the test fails when the file is stale) |
| `data/script_arg_overlay.json` | Argument names and kinds of the script commands | Written by hand from the `*_check` functions of `src/lvl_script_commands.c` |
| `crates/kfx-gfx/src/gen_*.rs` | Creature animation list and GUI sprite names | `python3 -I tools/gen_gfx_tables.py "$KFX_SOURCE" crates/kfx-gfx/src` |
| `data/prefabs/*.kfxprefab`, `crates/kfx-prefab/src/starter_files.rs` | The starter prefab set and the list that embeds it | `KFX_INSTALL=... cargo run -p kfx-prefab --example make_starter` |
| `data/i18n/en.toml` | The English UI catalogue, the template for translations | `cargo run -p kfx-prefab --example extract_strings` |

Do not edit generated files by hand. Licence notes for this data are in
[THIRD_PARTY.md](THIRD_PARTY.md).

## Submitting changes

* Keep a change focused on one thing, with a short description of what it changes and why.
* Run `cargo fmt --all`, the tests of the crates you changed, and, for UI changes, the editor's
  tests with `KFX_INSTALL` set if you have an install.
* Describe in the pull request which tests you ran and with which data (install, game tests).
* Add an entry to [CHANGELOG.md](CHANGELOG.md) for user-visible changes.

By contributing you agree that your contribution is licensed under the project's licence,
GPL-2.0-or-later.

## Reporting bugs

Open an issue at <https://github.com/eddoww/keeperfx-editor/issues> with:

* the editor version (Help > About);
* the operating system;
* the KeeperFX version (Preferences > Files shows the game version read from `keeperfx.log`);
* what you did, what happened and what you expected, as steps that reproduce it;
* the editor's output: the editor writes no log file of its own, so start it from a terminal and
  copy what it prints there (errors, and a panic message if it crashed). For a problem in the
  game, the game's `keeperfx.log` from the KeeperFX folder helps too;
* the level involved, zipped, if you can share it (please do not attach files from the original
  game).

Crash-recovery snapshots are kept in the `autosave/` folder of the editor's configuration folder,
and backups in `backups/` (`File > Backups...`), so work is rarely lost after a crash.

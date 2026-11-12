# Architecture

## Crates and dependencies

The workspace has fifteen crates. No library depends on the application. The widget crates
(`*-ui`, and the egui parts of `kfx-audio`, `kfx-live`, `kfx-prefab` and `kfx-workshop`) have no
window or rendering backend of their own: `kfx-editor` hosts them.

| Crate | Role | Depends on (workspace) |
|---|---|---|
| `kfx-formats` | Byte-exact readers and writers of the KeeperFX file formats: RNC, the binary level files, the TOML-based FX files, `.lof` / `.lif`, campaign files, the `.cfg` dialect, string tables, `translation.toml`; crash-safe writes (`atomic`). No GUI, no game logic. | none |
| `kfx-gfx` | Decoders to RGBA8: palette, texture atlases (tmap), keeper sprites (RLE), sprite sheets, custom sprite zips, install file lookup. | `kfx-formats` |
| `kfx-map` | The map document `MapDoc` (slabs, columns, things, action points, lights, properties), the slab placement port of `src/map_blocks.c`, column pool, decoration, rules for things, doors and hero gates, transforms and symmetry, `Patch` based undo, validation, layered game data, mod layers, creature configs, sidecar, `.une` import, legacy output. | `kfx-formats` |
| `kfx-script` | DK script and Lua analysis with no UI: lossless parser, diagnostics, completion, signature help, hover, outline, references, formatting; Lua lexer, parser, type inference and annotation index. Data comes from the embedded JSON in `data/`. | `kfx-formats` |
| `kfx-script-ui` | egui script editor widget (`ScriptEditor`), `ScriptWorkspace` (tabs for a level's `.txt` / `.lua`, dirty state, external changes), snippets, the script wizard. | `kfx-script` |
| `kfx-logic-ui` | The Events editor: the DK script as a logic document (events, parties, variables), import and export that keep untouched lines byte for byte, analysis, Lua export, wave templates and the egui widget. | `kfx-script` |
| `kfx-config-ui` | egui editors for configuration overrides (rules, terrain, slab set, column set, creatures, sprites, tileset, sounds) that write only what differs from the lower layers; file access only through a `ConfigFs` trait. | `kfx-formats`, `kfx-gfx`, `kfx-map` |
| `kfx-campaign-ui` | Campaign and map pack model, file operations, text tables, validation with fixes, land view decoding and the egui editor. | `kfx-formats` |
| `kfx-analysis` | Map analysis (movement and digging costs from the level's rules, distance fields, reachability, economy, fairness, symmetry, heroes) and the constrained map generator. | `kfx-formats`, `kfx-map` |
| `kfx-prefab` | Prefab files, capture and placement as one undoable `Patch`, the starter set, the level text translation grid, and the editor's i18n runtime and string scanner. | `kfx-formats`, `kfx-map`, `kfx-script` |
| `kfx-workshop` | Read-only, cached client of the KeeperFX Workshop, safe archive reading, install plans applied transactionally and undone on uninstall, packaging, egui browser. | `kfx-formats` |
| `kfx-audio` | Sound banks, `sounds.cfg` layering and sound resolution as the game does it, decoding, preview playback (cpal) and the sound browser. | `kfx-formats` |
| `kfx-live` | Client for the game's TCP JSON API on its own thread, play-test actions, reading Lua output from the game log, and the Live link panel. | none |
| `kfx-fonts` | System fallback fonts for egui through the operating system's font fallback; no bundled fonts. | none |
| `kfx-editor` | The application (eframe + wgpu): document handling with undo, 3D and 2D renderer, tools, panels, play-testing, settings, the command line and the headless mode. | all of the above |

Rules that hold across crates (see also [CONTRIBUTING.md](../CONTRIBUTING.md#conventions)): no
`unsafe`; integers little-endian; no panics on malformed input (return `Result`); every file loads
through `rnc::load_file`; file name case is resolved case-insensitively
(`testutil::resolve_ci`, `kfx_gfx::install::resolve`, `kfx_map::gamedata::find_ci`); every format
or rule taken from the game cites the KeeperFX source file and line it comes from in a doc
comment.

## Data flow

```text
install folder (fxdata/, data/, campgns/, levels/, mods/)
  |  kfx_map::mods + gamedata: layer stack (base, mods, campaign, map overrides)
  v
GameData  (terrain.cfg, cubes.cfg, slabset/columnset.toml, spritecolors, name lists)
  |
mapNNNNN.* files --(kfx_formats::level_bin / level_fx, rnc)--> MapDoc ---> Document (kfx-editor)
                                                                              |  edits = Patch (undo/redo journal)
                                                                              v
                                   autotile + decorate + slab objects  (kfx-map)
                                                                              |
                          terrain.rs / mesh.rs / atlas.rs / sprites.rs -> render.rs (wgpu) -> egui viewport
                                                                              |
                                                          MapDoc::save -> mapNNNNN.* (+ sidecar)
```

1. **Loading.** `document::locate` finds the map's campaign / map pack and size, builds the
   configuration layers (`LayerStack`, honouring `mods/load_order.cfg` unless disabled), and
   `GameData::load` applies every config file in the game's order. `MapDoc::load` then reads the
   binary files, tries `.lgtfx` to choose the FX or legacy thing files, and reads the sidecar.
2. **Editing.** The tools produce batches of `Edit`s. `document::apply_edits` places slabs the way
   the game does (`place_slab_type_on_map_f`), regenerates every slab within one of a changed slab
   once, decorates the new columns, and replaces the slab objects (torches, furniture) of the
   regenerated slabs; things on edited slabs follow (doors, ownership). Every change is a
   `kfx_map::patch::Patch` (old and new cells, columns, items); the strokes of one gesture are
   concatenated into one undo step, and undo/redo are exact.
3. **Rendering.** `terrain.rs` copies the document's current column grid; `mesh.rs` builds chunk
   meshes the way the game's renderer draws columns (`engine_render.c`); `atlas.rs` builds the
   texture atlas per texture set with no filtering across tile borders; `sprites.rs` decodes thing
   sprites by the game's rules (`objects.cfg`, creature cfgs, `trapdoor.cfg`, custom zips).
   `render.rs` draws offscreen or to the window; `pick.rs` casts rays against the column heights.
4. **Saving.** `MapDoc::save` writes `.slb .own .wlb .inf .wib .clm .dat` (+ `.slx`, `.flg` as
   needed) and always the three FX files, removes stale legacy files after everything else was
   written, then the sidecar. The editor backs the level up (once per session per level) before
   overwriting, and writes crash-recovery snapshots while there are unsaved changes.
5. **Play-testing.** `playtest.rs` builds the game's command line; a loose map is copied into
   `levels/personal` with a `.lof` first; the game's `keeperfx.log` is read back into the Game log.

## Tracking KeeperFX

The editor's idea of what is valid (script commands and their arguments, named constants,
configuration keys, the Lua API, sprite tables) is data extracted from the KeeperFX source and
wiki, checked in under `data/` (and `crates/kfx-gfx/src/gen_*.rs`) and embedded in the binary;
nothing is read from the KeeperFX source at run time. `tools/sync_keeperfx.py` regenerates all of
it in one go and records the upstream revision in `data/UPSTREAM.json`; the daily
`sync-keeperfx` workflow runs it against the KeeperFX default branch and proposes the result as a
pull request with a change report and the test results.

The one hand-written part, `data/script_arg_overlay.json` (argument names and semantic kinds), is
reconciled rather than regenerated: new commands get generated entries marked `"auto": true`,
changed commands keep their hand-written positions, removed commands move to a `"removed"`
section. Commands therefore never become "unknown" because the overlay lags; `kfx_script::Overlay`
exposes the `auto` flag and the removed entries.

At run time `kfx_script::DataRevision` reads the embedded `UPSTREAM.json`: Help > About,
`kfx-editor --version` and `kfx-editor info` show it, and the `unknown-command` diagnostic says that
a command is not known to this build's data (generated from KeeperFX on that date) instead of
calling it invalid, or that KeeperFX removed it. The tests check invariants of the data (every
command has an overlay entry with matching arguments, long-standing commands keep their shape)
rather than counts, so a sync passes without hand edits unless upstream really changed something
the editor depends on. CONTRIBUTING.md ("Generated data") has the commands and settings.

## The headless mode

`kfx-editor --screenshot OUT.png --map MAP [--actions FILE.json]` renders the same UI offscreen
(the `headless` module) and writes a PNG. The `--actions` JSON drives the real editor code paths (palette,
tools, mouse gestures, undo, save, dialogs, script panel, and the other panels); the UI tests and
the manual's pictures use it. With `--record` the same session is written frame by frame, with
simulated pointer and keyboard input (the `record` module). `kfx-editor --help` lists every action.

## How the autotile port is validated

The game does not compute columns when loading a level: `.clm`/`.dat` are precomputed and the game
runs `map_blocks.c` only when a slab changes in play. An editor has to produce the columns itself,
so `kfx_map::autotile` ports that logic (every function cites the C function it ports, quirks
included) as a pure function of the slab layout. Validation has three layers:

1. **Hand-derived layouts** (`kfx-map/tests/layouts.rs`): small layouts whose expected slabset
   entries are worked out from the C code in comments, with expected column content read
   independently from `slabset.toml` / `columnset.toml` using the plain `toml` crate.
2. **Golden test against the shipped maps** (`kfx-map/tests/golden.rs`): regenerate the columns of
   **every shipped map** and compare subtile by subtile with the shipped `.clm`/`.dat` (by column
   content, not pool index). The test prints an exact table, a loose table, and classifies every
   non-exact subtile into an explanation class (bits only; visually identical cubes; decoration
   randomisation; older KFX slabset; room wall not decorated; other variant; unreferenced columnset
   column; column of another slab kind; unexplained). It asserts that the decoration-normalised
   match is above 90% and unexplained subtiles are below 1%, and that the distribution of
   decoration choices is close to the shipped maps'. The decoration families are an
   explanation, not a claim that the game randomises (it does not; the original editors did).
   Diagnostics through `KFX_GOLDEN_*` variables (see the file header).
3. **The real game** (the game tests of `kfx-map`, `kfx-editor`, `kfx-config-ui` and other
   crates; they need `KFX_GAME_TEST=1`, see [CONTRIBUTING.md](../CONTRIBUTING.md#testing)): maps written by `MapDoc::save`
   and by the editor's own edit paths are loaded by the actual KeeperFX binary (offscreen SDL, dummy
   audio, a private working copy of the install, hard timeout, one game at a time) and the
   game's log is compared with the log of the original map. The config-override test also checks that
   a `slabset.toml` override is observable in play.

## Test strategy

* **Pure unit tests** inside the modules (no files): parsers, ports, text operations, tool geometry,
  shortcuts, settings, play-test command lines.
* **Round-trip tests**: for every binary format `to_bytes(parse(x)) == x`; FX output is byte-exact on
  all real `.tngfx/.aptfx/.lgtfx` files of the test install; legacy output round-trips shipped
  legacy maps; `Patch::revert(apply(doc)) == doc` over random operation sequences
  (`kfx-map/tests/recorder.rs`, which also checks the cheap `record_tracked` against the snapshot
  `record`).
* **Corpus tests** over everything shipped: every level script is parsed and validated
  (`kfx-script/tests/corpus.rs`), every Lua file is lexed, parsed and linted
  (`lua_corpus.rs`), the validator runs over every shipped map (`validate_shipped.rs`), a fuzz
  test truncates and damages real Lua files and requires no panic.
* **Cross-checks of generated data**: `lua_c_crosscheck.rs` compares the Lua annotation index with
  what the C code registers; `lua_index_gen.rs` fails when the shipped `data/lua_api.json` is stale
  relative to a fresh build from the install.
* **UI tests** with `egui_kittest` (wgpu) in the widget crates and in `kfx-editor`: they render
  widgets or the whole editor and inspect the result. They need a wgpu adapter (a GPU or a
  software Vulkan driver).
* **Tests that need the install** call `kfx_formats::testutil::install_dir()` (the `KFX_INSTALL`
  environment variable) and return early when it is not set, so they are skipped, not failed, on
  a machine without KeeperFX. [CONTRIBUTING.md](../CONTRIBUTING.md#testing) lists what each
  environment variable enables.

## Platform notes

The code is written for Linux, Windows and macOS, and CI builds and tests all three. Platform
specific points: a folder is opened with `explorer` / `open` / `xdg-open`; the game is started
through `wine` when the executable is a `.exe` and the host is not Windows (a wrapper command can
be set in Preferences); settings are in the platform configuration folder given by the
`directories` crate; fallback fonts come from fontconfig, DirectWrite or CoreText. The game test
harness (`kfx-map/tests/support/game.rs`) is Unix only: it uses symbolic links and signals to
run and stop the game.

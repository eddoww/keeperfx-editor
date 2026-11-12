# Level file formats as implemented

Developer reference. Everything here is taken from the doc comments of `kfx-formats` and
`kfx-map`, which cite the KeeperFX C source; `file:line` references below are to files of the
[KeeperFX repository](https://github.com/dkfans/keeperfx), and line numbers can drift as it
changes. All integers are little-endian unless noted.
Any file may be RNC-packed: loaders go through `kfx_formats::rnc::load_file`; the editor always
writes unpacked files.

A level `N` is a set of files named `mapNNNNN.<ext>` (five digits, zero padded; `file_name`
writes lowercase, `find_file` matches case-insensitively, because installs mix `MAP00007.SLB` and
`map00001.slb`). The files sit in one folder: a campaign's `LEVELS_LOCATION`, a map pack folder
(`levels/<pack>/`), or `multiplayer/<pack>/`.

## Overview

| Extension | Content | Required by the game | Written by the editor |
|---|---|---|---|
| `.slb` | slab kind per slab | yes | yes |
| `.own` | owner per subtile | yes | yes |
| `.dat` | column index per subtile | yes | yes |
| `.clm` | column table | yes | yes |
| `.wib` | wibble (liquid animation) per subtile | yes | yes |
| `.wlb` | water/lava/bridge type per slab | optional | yes |
| `.flg` | flags per subtile | optional | only if the map had one |
| `.inf` | texture set id | yes | yes |
| `.slx` | extra per-slab byte (texture style) | optional | only if any value is non-zero (a stale one is removed) |
| `.tng` `.apt` `.lgt` | legacy things / action points / lights | one of the two sets | no (stale ones are removed on save) |
| `.tngfx` `.aptfx` `.lgtfx` | TOML things / action points / lights | one of the two sets | yes, all three |
| `.lof` | level overview (name, size, kind, ...) | map packs | when properties are edited, when the size is not 85x85 and the folder has none, and an existing one always gets the map's true `MAPSIZE` |
| `.lif` | level index of a map pack | map packs | not edited; copied (and renumbered) with Save As |
| `.txt` | DK level script | no | yes (script workspace) |
| `.lua` | Lua level script | no | yes (script workspace) |
| `.<lang>.dat` | level string table (`mapNNNNN.eng.dat`) | no | when strings are edited (backed up first); copied with Save As |
| `.une` | custom slab ids of the Unearth editor | n/a | kept in step when one exists and could be read; an unreadable one is left untouched |
| `.kfxed.toml` | the editor's sidecar | never read by the game | when there is something to store |

"Required" is what `load_level_file` (`src/lvl_filesdk1.c:1422`) insists on; optional files are
skipped when missing.

## How files are written (`kfx_formats::atomic`)

Every file is written crash-safely: to a temporary file in the same folder (`.<name>.kfx-new-*`),
flushed to disk, then renamed over the target. A map save (`MapDoc::save`, `save_legacy`, and the
editor's `.lof` and sidecar) is one `SaveTransaction`: all new files are written as temporary files
first; only then are the targets replaced, in order, each old file kept as a hidden backup
(`.<name>.kfx-old-*`, a hard link or a copy); `.lgtfx` (the format switch, see below) is replaced
last; stale files (other format, other letter case, an all-zero `.slx`) are removed after all
replacements. Any failure (disk full, permissions, a file marked read-only) puts the old files back
and removes the new ones, so the folder holds exactly the old map or exactly the new one; the folder
entry changes are flushed (`fsync` of the folder on Unix). Only a crash or power loss in the middle
of the renames can leave a mix, with the `.kfx-old-` files next to the map to recover from. Single
files (string tables, `.lof` / `.lif`, campaign `.cfg`, scripts, config overrides, sprite zips,
settings, autosave snapshots) use the same write-then-rename.

## Sizes and grids

* Map size is in slabs (tiles), at most 170 x 170 (`MAX_TILES_X`, src/globals.h:163); the game default
  is 85 x 85 (`DEFAULT_MAP_SIZE`, src/map_data.h:55). `MapDims { w, h }`. Subtiles: `SW = 3W`,
  `SH = 3H`.
* The size is **not** stored in the binary files. For a square map it can be inferred from the
  `.slb` length (`2*W*H`, `MapDims::from_slb_len`); otherwise it comes from a `MAPSIZE` key in the
  `.lof` (or campaign `[mapNNNNN]` block). A non-square `.slb` without a declared size cannot be
  loaded (`document::locate` reports it).
* Subtile grids (`.own`, `.wib`, `.dat`, `.flg`) are `(SW+1) x (SH+1)`: one extra row and column
  beyond the playable area (src/lvl_filesdk1.c:703, 1071, 1226).
* Size policy (`load_single_map_file_to_buffer`, lvl_filesdk1.c:~125-160): a file shorter than
  expected is rejected, a longer one is accepted and the excess ignored. `parse` mirrors this: too
  small is an error, the excess is kept in `trailing` and written back, so
  `to_bytes(parse(x)) == x` for every accepted input.

## Binary files (`kfx-formats::level_bin`)

* **`.slb`**: `u16` slab kind per slab, row-major, `2*W*H` bytes (`load_map_slab_file`,
  src/lvl_filesdk1.c:1189). A kind beyond the configured slab count becomes rock in the game.
* **`.own`**: one byte per subtile, `(SW+1)*(SH+1)` bytes (src/lvl_filesdk1.c:1071). The game calls
  `set_slab_owner` for every playable subtile in row-major order, so the owner of a slab is the
  byte of the *last* subtile visited in that slab, subtile `(3x+2, 3y+2)`. The editor writes all
  nine subtiles of a slab with the slab's owner. The extra row/column become neutral.
* **`.dat`**: `u16` per subtile, `2*(SW+1)*(SH+1)` bytes (src/lvl_filesdk1.c:703). The game stores
  `col_idx = -lword(value)`, i.e. the file holds the **negated** column index
  (`Dat::column_index` / `set_column_index`).
* **`.clm`**: `u32 count`, `u32 ignored` (kept), then `count` columns of 24 bytes
  (`load_column_file`, src/lvl_filesdk1.c:666). A count larger than the data is corrected by the game
  to `(size-8)/24`. Column layout (`struct Column`, src/map_columns.h:38, packed):

  | offset | size | field |
  |---|---|---|
  | 0 | i16 | use count (the game recomputes it) |
  | 2 | u8 | bitfields: bit 0 active, bits 1-3 ceiling height, bits 4-7 floor height (the game recomputes the floor part on load, `set_column_floor_filled_subtiles`) |
  | 3 | u16 | solid mask |
  | 5 | u16 | floor texture |
  | 7 | u8 | orientation |
  | 8 | 8 x u16 | cube indices, bottom to top (`COLUMN_STACK_HEIGHT` = 8) |

  The game caps the table at `COLUMNS_COUNT`; the original game's table held 2048 columns (a
  limit that matters for legacy output, see below). `kfx_map::column::ColumnPool` deduplicates
  and builds `.clm` + `.dat` (`create_column`, `find_column`, `column_is_equivalent` of
  map_columns.c).
* **`.wib`**: wibble value per subtile, `(SW+1)*(SH+1)` bytes (lvl_filesdk1.c:~1050).
* **`.wlb`**: byte per slab, `W*H` bytes, optional (src/lvl_filesdk1.c:1141): 0 none, 1 lava,
  2 water, 3 bridge, 4 abyss (`Wlb::NONE/LAVA/WATER/BRIDGE/ABYSS`). The game resets values that
  disagree with the slab kind.
* **`.flg`**: `u16` per subtile, optional (src/lvl_filesdk1.c:1226). The game reads only the low byte;
  the high byte is preserved.
* **`.inf`**: byte 0 is the texture set id (`load_and_setup_map_info`, src/lvl_filesdk1.c:1309); any
  further bytes are ignored and kept.
* **`.slx`**: `W*H` bytes, extra per-slab byte (texture style in the editor: 0 = level default,
  n = set n-1). The game discards the file entirely when the unpacked size is not exactly `W*H`
  (src/lvl_filesdk1.c:1323).
* **`.tng`** (legacy things): `u16 count`, then 21-byte records (`LegacyInitThing`,
  src/lvl_filesdk1.c:89; `load_thing_file` :740): `x,y,z` (`u16`, high byte = subtile, low byte =
  fraction), `class u8`, `model u8`, `owner u8`, `range u16`, `index u16`, `params [u8; 8]`.
  A count larger than what fits is corrected to what fits (:748-755).
* **`.apt`** (legacy action points): `u32 count`, 8-byte records (`LegacyInitActionPoint`,
  :99; `load_action_point_file` :870): `x u16`, `y u16`, `range u16`, `number u16`.
* **`.lgt`** (legacy static lights): `u32 count`, 20-byte records (`LegacyInitLight`, :105;
  `load_static_light_file` :1247): `radius i16`, `intensity u8`, `flags u8`, three unused words,
  `x,y,z u16`, one unused byte, `is_dynamic u8`, `attached_slb i16`. Optional.

Counts in these files may disagree with the data; the model keeps the declared count
(`declared_count`) when it differs so the file round-trips.

## The `.lgtfx` format switch rule

`load_level_file` loads `.dat .flg .clm`, then **tries `.lgtfx`** (src/lvl_filesdk1.c:1440-1466). If it
loads, the level is in "FX format": `.aptfx` and `.tngfx` are used. Otherwise the legacy `.lgt`,
`.apt` and `.tng` are used. The presence of `.lgtfx` is therefore the switch; a level with `.tngfx`
but no `.lgtfx` still loads its things from the legacy `.tng`.

`MapDoc::load` follows this rule (and keeps things, action points and lights in the FX models in
memory either way, with the loaded format in `MapDoc::loaded_format`). `MapDoc::save` always
writes all three FX files, each with its mandatory `[common]` table, `.lgtfx` last, so `.lgtfx` is
always present, then removes any stale `.tng/.apt/.lgt` **after** everything else was written.

A `.tngfx`, `.aptfx`, `.lgtfx` or sidecar that exists but cannot be read is not treated as absent:
it is listed in `MapDoc::unreadable` (path, error with line and column, the raw bytes), the
validator reports `file-unreadable`, and `MapDoc::save` refuses (`MapDoc::read_only_reason`) until
the caller accepts losing it (`MapDoc::discard_unreadable`). For an unreadable `.lgtfx` the editor
shows what the game shows: the legacy files.
`MapDoc::save_legacy` (kfx-map, `legacy.rs`) writes the three legacy files and removes the FX
ones; see "Legacy output" below. The editor uses it for `File > Save format` and for
`kfx-editor convert --to legacy`.

## FX TOML files (`kfx_formats::level_fx`)

`mapNNNNN.tngfx` (things), `.aptfx` (action points), `.lgtfx` (static lights). Layout, from
`load_kfx_toml_file` (src/lvl_filesdk1.c:785-855):

* The TOML must parse **with the game's reader** (`deps/centitoml`, see "The game's TOML reader"
  below) and a top-level `[common]` table is mandatory (else the file is rejected).
* Items are either an array of tables (`[[thing]]`, `[[actionpoint]]`, `[[light]]`) or numbered
  tables `[thing0]`, `[thing1]`, ... whose count is `common.ThingsCount` / `ActionPointsCount` /
  `LightsCount`. Every real file in the test install uses the numbered form. A missing numbered
  table makes the load report failure but the other items are still created.
* Maximum counts: things `THINGS_COUNT - 2`, action points 255, lights 2047 (:862, :910, :1297);
  exceeding only warns.

Keys (all optional unless stated):

* Things (`thing_create_thing_adv`, src/thing_factory.c:217): `ThingType` (int or class name: Object,
  Shot, EffectElem, DeadCreature, Creature, Effect, EffectGen, Trap, Door, AmbientSnd, CaveIn);
  missing or unknown, or a class other than Object, Creature, EffectGen, Trap, Door (and the
  unused classes 10, 11), makes `thing_create_thing_adv` fail, and with it the **whole level**
  (`load_tngfx_file` is the level load result, src/lvl_filesdk1.c:1469). `Subtype` int;
  `SubtypeStringID` string overrides it using the config name tables (not a string, or an unknown
  name: the level fails too, src/thing_factory.c:225-237). `Ownership` int, required, 0..=9 (the loader
  tests `owner > PLAYERS_COUNT`, so 9 is accepted). The editor's validator reports these
  (`thing-class`, `thing-class-not-loadable`, `thing-subtype-name`, `thing-model`) and
  `MapDoc::save` refuses to write such a thing (`MapDoc::game_rejected_things`; the editor offers
  to remove them). `SubtileX/Y/Z` = `[subtile, fraction]`
  (raw coordinate `(subtile << 8) | fraction`; neither entry may be -1). `ParentTile` (objects, effect
  generators, traps), `Orientation` (objects, creatures, traps; `move_angle_xy`, 0 = north,
  512 = east, clockwise), `HerogateNumber` (hero gate objects), `CustomBox` (custom special boxes),
  `GoldValue` (gold piles), `CreatureLevel` 1..=10, `CreatureGold`, `CreatureInitialHealth`
  (percent), `CreatureName` (shorter than `CREATURE_NAME_MAX`), `EffectRange`, `DoorOrientation`,
  `DoorLocked`. `Index` is written by existing tools but **never read** by the game's TOML path.
* Action points (`actnpoint_create_actnpoint_adv`, src/actionpt.c:81): `PointNumber` >= 0,
  `SubtileX`, `SubtileY`, `PointRange`.
* Lights (`light_create_light_adv`, src/light_data.c:188): `Dynamic`, `SubtileX/Y/Z`, `LightRange`,
  `LightIntensity`, `ParentTile`.

Unknown keys are preserved (`extra`, sorted) and re-emitted; a known key with an unexpected type
is kept in `extra` and reported in `warnings`. Other top-level tables are preserved too and written
with the headers the game needs.

Output shape (`FxFile::to_text`): `[common]`, `<Count> = N`, then per item a blank line, `[thingN]`
and `Key = value` lines in a fixed key order, no trailing newline. This is verified byte-exact on
all 48 real files in the test install (`kfx-formats/tests/level_fx.rs`). A file that was not UTF-8
is read as Latin-1 and written back as Latin-1 (the game copies name bytes as they are). Before a
file is written its text is checked with the port of the game's reader (`FxFile::to_game_bytes`).

### The game's TOML reader (`kfx_formats::game_toml`, `kfx_formats::toml_emit`)

The game reads TOML with CentiTOML (`deps/centitoml/toml_api.c`), which differs from TOML:

* `[a.b]` needs `a` to exist already (the walk from the root creates nothing at the top level;
  the missing parent ends in "out of memory", toml_api.c:702). A deeper header `[a.b.c]` creates
  `b` implicitly; the next header walking through `b` makes it explicit (toml_api.c:683), and a
  header walking through an explicit middle table fails ("duplicate key", toml_api.c:1294).
  Accepted: `[a]` `[a.b]`; `[a]` `[a.b.c]`; `[a]` `[a.b.c]` `[a.b]`; `[a]` `[a.b.c]` `[a.b.d]`.
  Rejected: `[a.b]` alone; `[a]` `[a.b]` `[a.b.c]`; `[a]` `[a.b.c]` `[a.b.d]` `[a.b.e]`; any header
  into an array of tables.
* `key = value` stores the key as written, quotes included (toml_api.c:768), so `"ThingType" = 1`
  is not `ThingType`; headers, dotted keys, arrays and inline tables unquote. A duplicate key fails
  unless its value is `[]`.
* `\u` / `\U` escapes need upper-case hex; `'''` cannot appear in a one-line basic string; a
  multi-line `"""` string keeps the newline after the opening quotes; `-0` and dates are rejected;
  `true` / `false` match by prefix; numbers other than plain JSON numbers (`0x10`, `1_000`, `+1`)
  parse but read as nothing; the text ends at the first NUL byte.

The editor reads FX files with a port of this reader (`game_toml::parse`), so it opens what the game
opens and sees the same keys; text only a TOML reader accepts is read with a warning (the game
ignores such a file until it is saved again). Everything the editor writes for the game (FX files,
TOML config overrides, `translation.toml`) goes through one writer, `toml_emit`, that produces only
accepted forms: bare keys when they match `[A-Za-z0-9_-]+`, else quoted; one-line basic strings
with upper-case `\u00XX` escapes (and `\u0027` for `'` when the text holds `'''`); `[a]` before
`[a.b]`, tables deeper than two levels as dotted keys (`c.x = 1`, empty ones as `c = {}`) inside
the `[a.b]` section; arrays of tables as `[[a]]` / `[[a.b]]` or inline; top-level values before the
first header. TOML config overrides keep their layout when the game reads it as written, else they
are rewritten by `toml_emit` (comments are lost then).

Doors and hero gates are things plus slab state; see `kfx-map/src/things.rs`: the door slab kind
is decided by the door model and `DoorOrientation` (`create_door` re-places the slab when the level
loads), a new hero gate gets the smallest free number 1..255, action points the smallest free
number >= 1.

## Slab objects are in the files

The game creates slab objects (torches, room furniture, slab lights, the dungeon heart pieces)
only when it **places** a slab in play. Loading a level reads the precomputed columns and then
creates exactly the things and lights listed in the thing/light files. So a map file must
contain its torches, hearts, room decorations and slab lights itself, tied to their slab through
the parent index (`ParentTile`, legacy thing `index`, light `attached_slb`). The editor removes
and rewrites those for the slabs it regenerates (`MapDoc::apply_slab_objects`).

## Level information: `.lof`, `.lif`, campaign cfg

* **`.lof`** (`kfx_formats::levelinfo::Lof`; `level_lof_file_parse` src/lvl_filesdk1.c:344, key table
  `cmpgn_map_commands` src/config_campaigns.c:92): a `.cfg`-dialect file without sections (parsing stops
  at the first line starting with `[`). The file name must be `mapNNNNN.lof`; the level number comes
  from the name. Keys: `NAME_TEXT`, `NAME_ID`, `ENSIGN_POS`, `ENSIGN_ZOOM`, `PLAYERS`,
  `ENSIGN`/`OPTIONS`, `SPEECH` (two files), `LAND_VIEW` (two files), `KIND` (any of SINGLE MULTI
  BONUS EXTRA FREE), `AUTHOR`, `DESCRIPTION`, `DATE`, `MAP_FORMAT_VERSION` (the last four are
  ignored by the game), `MAPSIZE` (x y, default 85 85). The typed record `LevelInfo` also describes a
  `[mapNNNNN]` block of a campaign cfg (plus `INTRO_DESC_ID`, `DESCRIPTION_GEO`).
* **`.lif`** (`Lif`; `level_lif_entry_parse`, src/lvl_filesdk1.c:182): lines `<number><sep><name>`
  where `<sep>` is any run of whitespace, `,`, `;`, `:`; `;` before the number comments out the line;
  a name starting with `#` carries a string id. Unparseable lines are kept as opaque text.
* **Campaign / map pack cfg** (`kfx_formats::campaign`; config_campaigns.c): `campgns/*.cfg`,
  `levels/*.cfg`, `multiplayer/*.cfg`. `[common]` commands, `[strings]` / `[speech]` per language,
  `[mapNNNNN]` blocks **only for real campaigns** (in `campgns/`); map packs get level data from
  `.lof`/`.lif` files in the levels folder (`find_and_load_lof_files` runs only for packs that are
  not campaigns, src/config_campaigns.c:1326). So the map size of a campaign level is the `MAPSIZE` of
  its `[mapNNNNN]` block; the editor writes it there when the size changes (backing up the `.cfg`
  first) and does not create a `.lof` for it. For map packs and loose maps an existing `.lof` always
  gets the map's true size (also back to 85 85), and one is created for a map that is not 85x85. All `*_LOCATION` values are relative to the install
  root. `SINGLE_LEVELS` / `EXTRA_LEVELS` (numbers > 0) and `BONUS_LEVELS` (>= 0) accumulate over
  repeated keys.

## The cfg dialect and layering

`kfx_formats::cfg::CfgDoc` is a lossless model of the game's `.cfg` dialect (`config.c`): line
oriented, `;` comment lines only (no inline comments), `[section]` lines matched
case-insensitively, `KEY<sep>value` where the separator is any run of spaces, tabs, `=`; keys
match case-insensitively; no quoting; decoded as **Latin-1** so any byte sequence round-trips.
Layering (`load_config`, src/config.c:2326): `fxdata/F`, mods "after base", campaign
`CONFIGS_LOCATION/F`, mods "after campaign", `LEVELS_LOCATION/mapNNNNN.F`, mods "after map";
a later file assigning a key replaces the earlier value. Creature configs use the same order with
`creatrs/`, `CREATURES_LOCATION` and `mapNNNNN.<creature>.cfg`. `kfx_map::mods` models
`mods/load_order.cfg` (`[after_base]`, `[after_campaign]`, `[after_map]`, at most 50 mods per
block; ignored in multiplayer and with `-nomods`) and `kfx_map::layered` tracks which layer
supplied each value.

## Other files

* **Strings** (`kfx_formats::strings`): `fxdata/gtext_<lang>.dat`, campaign `[strings]` files and
  `mapNNNNN.<lang>.dat` are NUL-terminated strings, read through RNC, at least 16 bytes. Single-byte
  languages decode through the game's own codepage table; Japanese, Chinese and Korean double-byte
  tables are **not implemented** (raw bytes are preserved so those files round-trip). String ids:
  0..2000 level/campaign, 2000..4000 `translation.toml` aliases, 4000.. GUI.
* **`translation.toml`** (`kfx_formats::translation`): one table per alias, one string per language.
* **RNC** (`kfx_formats::rnc`): RNC method 1, 18-byte header (`RNC\x01`, big-endian sizes and
  CRCs), strict unpacking (any CRC or range error fails). Port of `bflib_dernc.c`.

## Sidecar: `mapNNNNN.kfxed.toml`

Editor-only data the game never reads (the game opens only the `mapNNNNN.<ext>` files it knows).
Written by `MapDoc::save_sidecar` next to the map; removed when there is nothing to store. First line
is a comment, then TOML with `version = 1` (`SIDECAR_VERSION`; a larger version is rejected):

* `[decoration]`: `seed` (optional; derived from the level number when absent), `enabled`,
  `disabled_families` (names of cube / floor families switched off).
* `[symmetry]`: `mode` and `owners` (owner mapping per sector).
* `[[locked]]`: `x0 y0 x1 y1 note`, a rectangle of slabs the editor refuses to modify.
* `[[note]]`: `x y text color` (notes pinned to a slab).
* `[[custom_slab]]`: `id` (1000 or more, the numbering `.une` files use), `name`, `recognized_as` (slab kind the
  game and neighbours see), optional `liquid`, nine `[[custom_slab.column]]` (`cubes`, `floor`,
  `orient`, `wibble`) and optional `[[custom_slab.object]]` (`kind` = object / effect / light,
  `model`, `range`, `intensity`, `radius`, `dx`, `dy`, `z`, `orientation`).
* `[custom_cells]`: `cells = [[x, y, id], ...]`, which slabs hold a custom slab.

Custom slabs let neighbours regenerate without destroying hand-built columns: the `.slb` holds
`recognized_as`, the `.clm`/`.dat` hold the nine columns, the sidecar remembers which slabs are
custom. The module `kfx_map::unearth` imports the `.une` file of the Unearth editor (a raw grid
of one `u16` per slab, 0 = no custom slab) and its `custom_slabs.cfg` into the sidecar model; when
a `.une` already exists next to the map and could be read (`MapDoc::une_sync`), saving rewrites it
so it stays consistent with the map; a `.une` that could not be read (wrong size, truncated) is left untouched. An unreadable
sidecar makes the map read-only (see the `.lgtfx` section). The sidecar belongs to the undoable
document state.

## Legacy output (`kfx_map::legacy`, `MapDoc::save_legacy`)

Writes maps in the original Dungeon Keeper format, for the original game and for tools that only read that format. Errors (nothing is written):
size other than 85x85, class/model/owner above 255 (or no number), owner above 5 (the legacy loader
discards such things, src/thing_factory.c:135), coordinates beyond 16 bits, hero
gate or custom box number above 255, action point number above 65535, light intensity above 255,
more things / action points / lights than the game loads. Warnings (written, information dropped):
extended creature fields (`CreatureName`, `CreatureGold`, `CreatureInitialHealth`), `Orientation`,
`GoldValue`, unknown extra keys, more than 2048 columns, custom slabs (the sidecar is still written).
Bytes the game ignores in a legacy record are kept in the thing's extra keys when loading and
written back, so an unedited legacy record round-trips byte for byte.

## Script files

`mapNNNNN.txt` (DK script) is held as bytes-preserving Latin-1 by the script editor so saving does
not change bytes outside the edit. `mapNNNNN.lua` is Lua. Both are loaded through the script
workspace; the DK script parser (`kfx_script::Document::parse`) is lossless and never fails.

## Save As and the other files of a level

Save As (`document::copy_level_extras`) also copies the level's own extra files to the new
folder / level number, renamed: config overrides (`mapNNNNN.rules.cfg`, creature overrides),
string tables, `mapNNNNN.zip` (custom sprites), `.lof` and `.lif` (a `.lif` is renumbered). A
`.lof` / `.lif` already at the target is kept. Files the document or the script workspace writes
itself are not copied this way.

# Command line

`kfx-editor --help` prints the reference. `MAP` is `mapNNNNN.slb` or another file of the level. A
KeeperFX install is found from `KFX_INSTALL`, the saved setting, or by walking up from the map.

```text
kfx-editor [MAP]                          open the editor window
kfx-editor validate MAP [--json]          map checks and map <-> script links; exit 1 on errors
kfx-editor info MAP [--json]              size, keepers, things, scripts, format, campaign, config layers
kfx-editor script-check MAP [--json]      DK script and Lua diagnostics; exit 1 on errors
kfx-editor export-image MAP OUT.png [--iso] [--scale N] [--no-things]
kfx-editor regenerate MAP [--decorate]    regenerate columns, slab objects and liquids, then save
kfx-editor convert MAP --to fx|legacy     rewrite in the KeeperFX format or the original DK format
```

`regenerate` and `convert` copy the level to the backups folder first. Example:

```text
$ kfx-editor validate campgns/keeporig/map00001.slb
.../map00001.slb: 0 errors, 0 warnings, 0 notes
$ kfx-editor script-check campgns/keeporig/map00001.slb
map00001.txt: 227 lines, 0 errors, 0 other diagnostics
map00001.lua: none
```

An unknown word that is not an existing file or folder is an error (see below).

## Screenshot mode

```text
kfx-editor --screenshot OUT.png --map MAP [--size WxH] [--camera x,y,zoom,yaw,pitch]
           [--perspective] [--viewport-only] [--frame N] [--actions FILE.json]
```

renders the editor offscreen with no window and writes a PNG. `--actions` is a JSON array of
editor actions run first (palette, tool, owner, press/drag/release, undo, open dialogs, script
panel, save, ...; the full list is in `--help`). Example (`terrain.json`):

```json
[
 {"camera": [130,130,60,45,-46]},
 {"palette": "TREASURE", "owner": 0, "tool": "box", "filled": true},
 {"press": [49,41]}, {"drag": [[51,43]]}, {"release": [51,43]},
 {"view": {"ownership": true, "grid": true}},
 {"screenshot": "out.png", "ui": true}
]
```

Set `KFX_EDITOR_CONFIG_DIR` to keep screenshot runs away from your real settings.

## Recording mode

The headless mode can also write a whole session frame by frame, for tutorial videos and
reproducible demonstrations:

```text
kfx-editor --screenshot OUT.png --map MAP --actions FILE.json --record DIR|- [--fps N]
           [--record-captions FILE.json] [--forbid-text TEXT ...]
```

* `--record DIR` writes every frame as `DIR/frame_NNNNNN.png`; `--record -` writes raw RGBA frames
  to standard output, to pipe into a video encoder (for example
  `ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i - out.mp4`).
* `--fps` sets the frame rate of the recording (default 30).
* The action list drives the editor with simulated input: `move` the pointer to a target, `tap` or
  `down` / `up` a button, `pdrag`, `stroke` across slabs, `type` text, press `keys`, turn the
  `wheel`, and move the camera with `camera_to`, `orbit` or `fly_path`; `hold` and `rec` render
  frames without input. Targets are pixels, slabs, subtiles, or a text or word drawn on screen.
  The full list is in `--help`.
* The pointer is drawn into the frames (with a ring for each click). Captions (`{"caption":
  "text"}` ... `{"caption": null}`) are not drawn: `--record-captions` writes them with their
  frame ranges as JSON, for the encoder or a subtitle file.
* `--forbid-text TEXT` (repeatable) stops the run with an error when a frame or screenshot shows
  that text (case-insensitive), for example a user or folder name that should not appear in
  published pictures.

Without `--record` the same actions still apply their input, but no frames are written.

## Exit codes and errors

An unknown command, an unknown option or a path that does not exist prints the usage and exits
with code 2 (the window is not opened). Commands exit with 1 when they find errors. A map with a
`.tngfx` / `.aptfx` / `.lgtfx` or editor data file (`.kfxed.toml`) the editor cannot read is read-only: every command prints
the file and the error and exits with at least 1, and `convert` / `regenerate` write nothing (exit
2). Before `convert` / `regenerate` overwrite a map its files are backed up, and the write is all
or nothing (see [Saving](getting-started.md#saving)).

On Windows the program keeps its console so that the commands can print; it is not built as a
GUI-only (`windows_subsystem = "windows"`) program, because such a program cannot print to the
console it was started from without platform-specific code.

## Events editor actions

These `--actions` keys drive the [Events editor](events.md) (the visual editor of the DK script).
Every change is written into the level's DK script text at once, like in the window; `{"save":
true}` then writes the map and the scripts.

| Key | Value | Does |
| --- | --- | --- |
| `events` | `"open"`, `"text"`, `"toggle"` | Show the Events tab (opens the script panel docked when hidden), go back to the text tab, or toggle. |
| `events_view` | `"cards"`, `"graph"`, `"timeline"`, `"parties"`, `"variables"`, `"lua"` | Choose the view. |
| `events_select` | a number or a text | Select the n-th event (1-based, document order, as `events_print` numbers them) or the first whose summary contains the text; its places are ringed and its party routes drawn on the map. |
| `events_party` | party name | Show the party in the party / wave designer. |
| `events_add_wave` | object, below | Add a wave (and its party). |
| `events_undo`, `events_redo` | count | The Events undo / redo. |
| `events_print` | `true` | Print the events, parties, the validation of the generated script and the routes / highlighted places. |

`events_add_wave` fields:

| Field | Default | Meaning |
| --- | --- | --- |
| `party` | next free `PARTYn` | An existing party, or the name of the party that `creatures` creates. |
| `creatures` | none | Members of a new party: `"KNIGHT"`, `["KNIGHT", level, gold]` or `{"creature": "KNIGHT", "level": 4, "gold": 300}`; at most 30. Leave it out to use an existing party. |
| `objective` | `"ATTACK_ENEMIES"` | Objective of the new members. |
| `owner` | `"PLAYER_GOOD"` | Who owns the spawned creatures. |
| `spawn` | `{"hero_gate": 1}` | `{"hero_gate": n}`, `{"action_point": n}`, `{"player": "PLAYER0"}` (heart) or the location text (`-1` hero gate 1, `3` action point 3). Must exist on the map. |
| `turn` | | The wave comes when `GAME_TURN >= turn` of `clock_player` (`PLAYER0`). |
| `condition` | | Instead of `turn`: `{"player": "PLAYER0", "variable": "FLAG1", "op": "==", "value": 1}` (any `IF` variable). One of `turn` / `condition` is required. |
| `copies` | 1 | Copies of the party per wave (`ADD_PARTY_TO_LEVEL` copies). |
| `count`, `interval`, `copies_step` | 1, 3000, 0 | With `turn`: `count` waves `interval` turns apart, each `copies_step` more copies. |
| `tunneller` | | `{"head_for": "DUNGEON_HEART", "target": 0}`: a tunneller party (`ADD_TUNNELLER_PARTY_TO_LEVEL`, level 3, 200 gold) digging towards the target. |

An error (unknown party without `creatures`, a party name that exists with `creatures`, a spawn
place not on the map, a view-only script) stops the action list with a message. A level without a
DK script gets the default one first.

Example: a new map with one hero gate, two waves of a new party from turn 600 and one more when
the keeper has eight creatures, saved (`wave.json`):

```json
[
 {"new_map": {"folder": "/path/to/levels", "level": 3001, "w": 60, "h": 60, "generate": true,
              "players": 1, "hero_gates": 1, "script": "dk"}},
 {"events": "open"},
 {"events_add_wave": {"party": "INVADERS", "creatures": [["KNIGHT", 3, 100], ["ARCHER", 2], ["ARCHER", 2]],
                      "owner": "PLAYER_GOOD", "spawn": {"hero_gate": 1}, "turn": 600, "count": 2, "interval": 1200}},
 {"events_add_wave": {"party": "INVADERS", "spawn": {"hero_gate": 1},
                      "condition": {"player": "PLAYER0", "variable": "TOTAL_CREATURES", "op": ">=", "value": 8}, "copies": 2}},
 {"events_print": true},
 {"save": true}
]
```

```text
kfx-editor --screenshot out.png --map ANY_MAP.slb --actions wave.json
```

writes `map03001.txt` with `CREATE_PARTY(INVADERS)` and its `ADD_TO_PARTY` lines among the
level-start commands and, at the end:

```text
REM Wave 1 of 2: party INVADERS
IF(PLAYER0,GAME_TURN >= 600)
	ADD_PARTY_TO_LEVEL(PLAYER_GOOD,INVADERS,-1,1)
ENDIF

REM Wave 2 of 2: party INVADERS
IF(PLAYER0,GAME_TURN >= 1800)
	ADD_PARTY_TO_LEVEL(PLAYER_GOOD,INVADERS,-1,1)
ENDIF

REM Wave: party INVADERS when TOTAL_CREATURES >= 8
IF(PLAYER0,TOTAL_CREATURES >= 8)
	ADD_PARTY_TO_LEVEL(PLAYER_GOOD,INVADERS,-1,2)
ENDIF
```

## Analysis and generator

These `--actions` drive the Analysis window and the generator (the panels are described in
[Map analysis](analysis.md) and [Map generator](generator.md)):

| Action | Does |
|---|---|
| `{"analysis": true}` | open the Analysis window and wait for the analysis (`false` closes it) |
| `{"analysis_layer": "reach"}` | heat map: `off`, `reach`, `dig`, `walk`, `territory`, `heroes`, `gold`, `class`, `symmetry` |
| `{"analysis_player": 1}` | keeper of the per-keeper layers (owner number) |
| `{"analysis_opacity": 150}` | heat map opacity 0..255 |
| `{"analysis_tab": "fairness"}` | `layers`, `fairness`, `economy`, `routes`, `symmetry`, `heroes` |
| `{"analysis_symmetry": "rot2"}` | symmetry mode of the Symmetry tab and layer (`auto`, `ns`, `ew`, `fourway`, `rot2`, `rot4`) |
| `{"analysis_route": 2}` | highlight route 2 (0-based) and centre on it; `null` shows all routes |
| `{"analysis_breaker": 0}` | click symmetry breaker 0 of the Symmetry tab (locate and select it) |
| `{"analysis_wait": true}` | wait until the analysis of the current map is done |
| `{"analysis_print": true}` | print a summary, the routes and the findings to stderr |
| `{"analysis_bench": 10}` | run the analysis 10 times and print the timings |
| `{"generate_selection": {"seed": 7, "style": "chambers", "players": 0}}` | Map > Generate into selection (needs a `select` first); also takes the generator keys below |
| `{"new_dialog": {spec}}` | open the New map dialog with these options |
| `{"new_dialog_folder": "DIR"}`, `{"new_dialog_seed": 5}`, `{"new_dialog_regenerate": true}` | change the open dialog |
| `{"new_dialog_wait": true}` | wait for the dialog's generated preview (prints its report line) |
| `{"new_dialog_create": true}` | press Create |
| `{"new_map": {"folder": "DIR", "level": N, spec}}` | make a new map directly |

A spec (`new_map`, `new_dialog`) takes `w`, `h`, `texture`, `players` (keepers 0..7), `imps`,
`hero_gates`, `seed`, `script` (`none`, `dk`, `lua`), `kind` (`.lof` kind such as `MULTI`) and
`name`, and either the fill options `border`, `fill`, `symmetry` (`ns`, `ew`, `fourway`, `rot2`,
`rot4`), `portals`, `gold_near`, or `"generate": true` with the generator options:

| Key | Values (default) |
|---|---|
| `style` | `open_cave`, `chambers`, `islands`, `river`, `fortress` (`open_cave`) |
| `gen_symmetry` | `auto`, `none`, `radial`, `ns`, `ew`, `fourway`, `rot2`, `rot4` (`auto`) |
| `richness` (or `gold`) | gold amount 0..100 (50) |
| `water`, `lava` | 0..100 (20 each); a value between 0 and 1 is read as a share |
| `heroes` | hero gates and fortress garrison (true) |
| `gems`, `neutral_rooms` (or `chambers`), `neutral_creatures`, `special_boxes` | counts (2, 2, 4, 2) |
| `min_gold` | minimum gold per keeper, 0 = from `richness` (0) |
| `max_imbalance` | largest fairness imbalance accepted (0.25) |
| `max_attempts` | attempts before giving up, 1..64 (8) |

The options `scale`, `earth` and `rock` are accepted for compatibility and ignored.
Generated maps are 20..170 slabs each way. A generation that meets no constraint in the given
attempts fails the action with the reasons.

Example: generate, save and check a four-keeper level (run from the folder of the JSON file;
`--map` only needs to name any existing map for the start-up):

```json
[
 {"new_map": {"folder": "gen", "level": 7, "generate": true, "seed": 4242, "players": 4,
              "style": "fortress", "script": "dk", "kind": "MULTI", "name": "Generated fortress"}},
 {"save": true},
 {"analysis": true},
 {"analysis_print": true},
 {"screenshot": "gen7.png", "ui": true}
]
```

```text
kfx-editor --screenshot out.png --map /path/to/keeperfx/campgns/keeporig/map00001.slb --actions gen.json
kfx-editor validate gen/map00007.slb
```

The saved level is in the FX format with its `.lof` and `map00007.txt`; the status line of the
action log names the keepers, style, seed and imbalance.

## Prefabs and translations in `--actions`

These entries drive the Prefabs panel and the translation grid from an action script, so a level
can be built headlessly. Several keys may sit in one object; they run in the order of this list.
Slab positions are `[x, y]` in slabs.

| Entry | What it does |
|---|---|
| `{"prefabs": true}` | Show the Prefabs panel (`false` hides it); add `"docked": false` for its own window. |
| `{"prefab_select": "Ambush"}` | Select the first prefab whose name contains the text (case-insensitive). While a prefab is being placed, the ghost becomes the new one. |
| `{"prefab_rotate": 90}` | Clockwise turn of the next placement: 0, 90, 180 or 270. |
| `{"prefab_flip": true}` | Mirror east-west (applied before the turn). |
| `{"prefab_owners": [1, 0]}` | Players of the prefab's slots A, B, ... (0..3 keepers, 4 heroes, 5 neutral, 6..8 extra keepers). |
| `{"prefab_mask": "replace"}` | `replace`, `skip_empty` (the prefab's plain rock / earth leaves the map alone) or `keep_existing` (land on rock / earth only). |
| `{"prefab_ghost": true}` | Start placing the selected prefab with the paste ghost; `hover` then shows it on the map. |
| `{"prefab_place": [x, y]}` | Place the selected prefab with its north-west corner on slab (x, y): one undo step. Add `"centre": true` to centre it on the slab like a click does. Prints the report and the action point / hero gate numbers given. |
| `{"prefab_save": {...}}` | Save the selection as a prefab of the user library: `name` (required), `description`, `author`, `tags` (list or comma separated), `to` (`user` or `campaign`), `script_dk` (a fragment using this map's action point / hero gate numbers), `mask_empty`, `things`, and `rect` `[x0, y0, x1, y1]` to select first. |
| `{"prefab_save_dialog": true}` | Open Edit > Save selection as prefab. |
| `{"prefab_print": true}` | Print the library, the placement options and the last placement. |
| `{"texts": "level"}` | Open the translation grid of the level (`"campaign"`: of its whole campaign). |
| `{"texts_add_lang": "FRE"}` | Add a language column (the game's three letter codes). |
| `{"texts_set": {"id": 2, "lang": "FRE", "text": "..."}}` | Set one cell. |
| `{"texts_save": true}` | Write the changed string files (backed up first). |
| `{"texts_export_csv": "out.csv"}`, `{"texts_import_csv": "in.csv"}` | CSV exchange of every column. |
| `{"texts_export_po": {"lang": "FRE", "path": "fre.po"}}`, `{"texts_import_po": {...}}` | gettext exchange of one language. |
| `{"texts_print": true}` | Print every row with its cells and states. |

A placed prefab with a script fragment appends the fragment, with its numbers filled in, to the
level's DK script (or Lua script); `{"save": true}` writes map and scripts together. Example: two
prefabs, one of them with a script fragment, then save.

```json
[
 {"prefabs": true},
 {"prefab_select": "Ambush corridor", "prefab_rotate": 90, "prefab_owners": [0]},
 {"prefab_place": [22, 46], "centre": true},
 {"prefab_select": "Treasure vault", "prefab_rotate": 0, "prefab_flip": true},
 {"prefab_place": [50, 20], "print": true},
 {"save": true}
]
```

## Other `--actions` of the campaign, Workshop, sound and live link features

These are described with their features: `campaign`, `campaign_wizard` in
[Campaigns and map packs](campaigns.md); `workshop` (always offline) in
[The KeeperFX Workshop](workshop.md); `sounds`, `mute`, `speech_preview` in [Sounds](audio.md);
`live`, `live_demo`, `print_audiolive` in [Live link](live.md).

## Editor language

| Action | Effect |
|---|---|
| `{"language": "fr"}` | Show the editor in this catalogue's language (see [Editor languages](languages.md)); an error names the languages found. Screenshots taken after it use it. |

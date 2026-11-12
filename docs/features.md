# Features

What the editor does, by area. The [user manual](manual/README.md) describes each part in
detail; known limitations are listed in the [roadmap](roadmap.md).

## Maps and terrain

* Opens a level from any of its files, from the map browser, from the recent maps list (the last
  ten, with level names), by dragging a file or folder onto the window, or from the command
  line. File names are matched case-insensitively and RNC-packed files are read transparently.
  See [Getting started](manual/getting-started.md).
* Map browser: the install's campaigns, map packs and multiplayer packs as a tree, with level
  names, a filter, a thumbnail with facts (size, keepers, things, scripts, format), and Edit, Play,
  Open folder and Random map buttons.
* New maps from 1 to 170 slabs in each direction, filled or generated, with keepers, imps,
  portals, gold, hero gates and a starter DK or Lua script.
* Painting tools: Paint, Line, Box, Circle (filled or outlined), Fill, Pick and Select, with
  square or round brushes from 1 to 9 slabs, ownership-only painting, an editable border switch
  and per-slab texture styles (`.slx`). See [Terrain tools](manual/terrain.md).
* Slabs are placed with a port of the game's placement code (`src/map_blocks.c`): walls,
  corners, room faces, torches, room furniture, heart pieces, slab lights, liquids and doors follow
  the game's rules, and things on edited slabs follow (room contents re-owned, doors re-oriented
  or removed).
* Optional decoration that varies cubes and floor textures within look-alike families, seeded
  and switchable per family; rounded corners; Fortify, which walls in a keeper's ground with
  fortified walls and the game's wall art; random doodads scattered on chosen slab kinds.
* Selection, cut, copy and paste carrying things, action points and lights, with rotate and flip
  while pasting.
* Resize from any edge, flip and rotate of the whole map or a selection (things and door
  orientations turn with it), and mirror symmetry (north-south, east-west, 4-way, half and quarter
  turn) with an owner table per sector.
* Regenerate columns, decoration, slab objects and liquids for the whole map or a selection.
* Custom slabs made of nine hand-built columns, a column editor for one subtile, and an editor for
  the distinct columns of the map (use counts, edit everywhere at once). See
  [Custom slabs](manual/custom-slabs.md).
* Locked regions that the tools refuse to change, and notes pinned to slabs.
* Map options: ownable natural terrain, things inside walls and rock.
* Image to map through an editable colour table, and export of the map as a PNG image (top-down or
  isometric).
* Named undo and redo, with a History tab to jump to any step.

## Things

* Creatures, objects, traps, doors, effect generators, hero gates, action points and lights:
  place, select (with stack cycling and box selection), move, duplicate, turn and delete. See
  [Things](manual/things.md).
* An Inspector with every field of the selected thing, a list of all things that centres the view
  on the selection, and per-class layer switches.
* Defaults for new things (level, gold, health, facing, free sub-position, lock state, light
  range), copied from an existing thing with Alt+click.
* Rules taken from the game: a door is a thing plus its door slab, locked doors get their key, new
  doors are oriented from the surrounding walls, hero gates and action points get the smallest
  free number. Space locks or unlocks the door under the pointer.
* Modded things from every configuration layer (`objects.cfg`, creature files, `trapdoor.cfg`,
  `effects.toml`, `magic.cfg`) and custom sprite archives of the install, mods, campaign and map.

## Views and navigation

* Isometric 3D view as in the game, a perspective camera, a top-down 2D view (textured or flat
  colours), a split 2D and 3D view, and a first-person fly view with optional collision. See
  [Navigating the view](manual/navigating.md).
* Zoom, pan, orbit and 45 degree rotation with configurable speeds, edge panning and smoothing;
  a minimap that shows and moves the visible area.
* Ownership tint, slab grid, lighting, creature level badges, hero gate numbers, ranges, facing
  arrows and script markers; layer switches for every class of thing.
* A readout of the slab and subtile under the pointer, and a grid data overlay of the raw map
  grids (slab kind, owner, `.wlb`, `.slx`, custom slab ID, column index, wibble, floor height,
  `.flg` flags).
* Sprite and texture animation as in the game.
* Configurable keyboard shortcuts with conflict detection, listed under Help > Controls. See
  [Keyboard shortcuts](manual/shortcuts.md).

## Scripts: DK script and Lua

* A script panel with tabs for the level's `mapNNNNN.txt` and `mapNNNNN.lua`, docked, maximised or
  in its own window. See [Scripts](manual/scripts.md).
* Completion, signature help, hover documentation, go to definition, outline, references and
  formatting for both languages; find and replace, go to line, snippets and a command reference.
* Diagnostics of DK scripts against the command tables of the game, and of Lua scripts against an
  index of the game's Lua API; a Problems list for all scripts.
* Meters of the script limits in use (conditions, script values, parties, triggers, win and lose
  conditions).
* Map references: every script line that names an action point, hero gate or location is shown
  as a marker on the map; positions can be picked on the map; renumbering asks about the script.
* Script files are kept byte for byte where they are not edited; external changes are detected.
* A script wizard (DK and Lua) for start gold, creature limits and pool, room, power, trap, door
  and creature availability, computer players, alliances and win / lose conditions, written as one
  marked block.

## Events editor

* The level's DK script as events: cards with conditions and actions, a node graph of flags
  between events, a timeline of turn and timer gates, a variables view, and a Lua export. See
  [Events editor](manual/events.md).
* A party and wave designer with creature portraits and strengths computed from the level's
  creature configuration, and wave templates (escalating waves, ambush on an action point, a
  final party on a condition).
* The same text as the script editor: every change is written into the script at once, undo works
  across both, and scripts it cannot represent are shown read-only.
* On the map: the places an event uses are ringed and party routes are drawn; arguments can be
  picked on the map and message ids from the level's strings.

## Configuration and mods

* The game's configuration layers (install, mods from `mods/load_order.cfg`, campaign, level) are
  loaded for the open level, so per-level overrides change what the editor shows. See
  [Config overrides and mods](manual/config-and-mods.md).
* A configuration editor with tabs for any config file, rules (with research and sacrifices),
  creatures, objects, traps and doors, rooms, magic, terrain, slab sets, column sets, cubes,
  tilesets, sprites and sounds. It writes only the values that differ from the layers below,
  keeps the comments of an existing override, backs files up, and can preview and regenerate
  affected slabs live.
* A Layers & mods window showing where every value comes from; mods can be ignored as the game
  does in multiplayer.
* A creature stats comparison of every creature at a chosen level, with values from the campaign,
  mods or map highlighted.
* A strings window for the level's and the campaign's text tables, and a list of every file of
  the level with its role.

## Custom slabs

* Custom slabs from a selection or from column lists, a Custom palette tab, and validation of
  custom slabs without a definition. They survive regeneration of their neighbours.
* The `.une` files and `custom_slabs.cfg` of the Unearth editor are read when a map opens, and a
  `.une` the editor could read is kept up to date on save.

## Campaigns and map packs

* A campaign editor for campaigns, map packs and multiplayer packs: settings, the level list with
  reordering and bonus links, the land view with draggable ensigns and zoom points, text tables
  and a list of problems with fixes. See [Campaigns and map packs](manual/campaigns.md).
* A wizard for a new campaign or map pack, and adding the open level to a pack (copied in with a
  free number when it lives elsewhere).
* Level properties written to the level's `.lof` or to the campaign's `[mapNNNNN]` block, with
  texture set thumbnails and an ensign picker on the land view. See
  [Level properties](manual/level-properties.md).

## Prefabs

* Reusable map pieces with slabs, owners, things, action points, lights and an optional script
  fragment. See [Prefabs](manual/prefabs.md).
* A starter set of 26 prefabs, a user library and per-campaign libraries, with thumbnails drawn
  from the open map's textures, search and tags.
* Placement with the paste ghost: rotate, flip, player mapping, three ways to treat existing slabs,
  free action point and hero gate numbers, and script fragments that follow undo and redo.
* Save a selection as a prefab; import and export prefab files.

## Analysis and generator

* Map analysis on a background thread: heat maps of reachability, dig and walk distance,
  territory, hero pressure, gold and gems, slab classes and symmetry breaks, in the 3D and 2D
  views. See [Map analysis](manual/analysis.md).
* Dig routes, first-contact points, and tables of fairness, economy, routes, symmetry and hero
  threat, each entry locating itself on the map.
* Clear problems (a keeper without a reachable portal or gold, isolated hero gates and pockets)
  reported in the Validate panel.
* A map generator with five styles (open cave, chambers, islands, river, fortress), seeds,
  symmetry modes, gold, liquids, extras and heroes, which checks every keeper's start, portal,
  gold, routes and fairness and repairs or retries. It makes new maps and fills a selection. See
  [Map generator](manual/generator.md).

## Translations and languages

* A translation grid for every text a level or campaign uses, in every language, with missing,
  inherited and outdated states, CSV and gettext `.po` exchange, and backed-up saving. See
  [Translations](manual/translations.md).
* The editor's own interface can be shown in other languages from catalogue files, switched
  without a restart. See [Editor languages](manual/languages.md).
* Game text in any script (Chinese, Japanese, Korean, Cyrillic, symbols) is drawn with the
  system's fallback fonts.

## Audio

* Sounds resolved as the game does for the open level: sound and speech banks, every `sounds.cfg`
  layer, custom sound files (WAV, MP3, Ogg Vorbis, FLAC), creature sound sets, level speech and
  music. See [Sounds](manual/audio.md).
* A sound browser with search, categories, layers, waveforms and variant playback; assigning
  sounds to mappings and creature sound slots; level speech preview; Play buttons on sound
  arguments of DK script commands.
* No audio device is opened until something is played; previews can be muted.

## Play-testing and the live link

* Play (F5) and Save and play start the game on the open level; a level outside every campaign or
  pack can be copied into `levels/personal`. The game's command line options and a wrapper command
  are set in the preferences, and `wine` is used for `keeperfx.exe` when not on Windows. See
  [Play-testing](manual/play-testing.md).
* Play and record a packet file, and replay the last recording.
* The game log is read back after a game, with errors and warnings listed and script errors
  linked to their line.
* A live link with the running game through its API: events, watched flags and variables, a
  console for game commands, DK script lines and Lua, quick actions, game camera control, and
  creature positions drawn on the map. See [Live link](manual/live.md).

## Workshop

* A window for the KeeperFX Workshop (keeperfx.net): browse and search items, read their pages,
  download and install them into the KeeperFX folder with an install plan, uninstall them
  (restoring what they replaced), and check installed items for updates. See
  [The KeeperFX Workshop](manual/workshop.md).
* ZIP, tar.gz and 7z archives are read directly, RAR through an external `7z` program; archive entries
  are checked for unsafe paths and sizes, and nothing from an archive is executed.
* Packaging of a level or campaign as a zip laid out the way the game expects, with a pre-flight
  check; the upload itself happens in the browser.
* Nothing is requested before the user agrees, and an offline mode uses only cached data.

## Validation

* Map checks with error, warning and note severities, each based on a game rule: limits, owners,
  thing classes and models the game cannot load, positions, doors that cannot stand, duplicate or
  missing numbers, hearts, reachability. See [Validation](manual/validation.md).
* Links between the map and its scripts (references to action points and hero gates that do not
  exist).
* A level the game would refuse to load is not saved.

## File formats and safety

* Reads and writes the KeeperFX (FX) format (`.tngfx`, `.aptfx`, `.lgtfx`) and the original
  Dungeon Keeper format (`.tng`, `.apt`, `.lgt`); the save format is chosen per map, and what a
  format cannot hold is listed before saving. The byte layout of every file is in
  [formats.md](formats.md).
* The FX files are read with a port of the game's own reader and written only in forms the game
  accepts.
* Saves are all or nothing: the files of a level are written under temporary names and replace
  the old files only when every one is complete; on failure the old files are restored.
* Every file of a level or its campaign is copied to the backups folder before its first
  overwrite in a session; a Backups window restores and prunes them.
* Crash-recovery snapshots while changes are unsaved, offered again at the next start.
* Files that cannot be read open read-only behind a dialog instead of being emptied by the next
  save; changes made on disk by other programs are detected.

## Command line and headless mode

* Commands without a window: `validate`, `info`, `script-check`, `export-image`, `regenerate` and
  `convert`, with JSON output where useful and exit codes for errors. See
  [Command line](manual/cli.md).
* A headless mode that renders the editor offscreen to a PNG, optionally after a JSON list of
  editor actions that drive the same code as the window (tools, dialogs, scripts, Events editor,
  analysis, generator, prefabs, translations, campaigns, sounds, live link).
* A recording mode that writes every frame of a scripted session (simulated pointer, clicks,
  keys and typing) as PNG files or raw video for an encoder, with captions in a separate file, for
  tutorial videos and reproducible demonstrations. `--forbid-text` fails the run when a frame shows
  a given text, for example a private folder name.

## Installation and updates

* Windows installer (per user without administrator rights, or for all users): Start menu entry,
  optional desktop shortcut and "Open with" entry for `.slb` maps, uninstaller, upgrade in place;
  and a portable zip. The editor window opens no console window; `kfx-editor-cli.exe` is the
  command-line program. See [Getting started](manual/getting-started.md#installing).
* Application icon on every platform (window, Windows executables and installer, macOS bundle,
  Linux desktop entry).
* Self-update from the GitHub releases: a notice before the first request, a check at most once
  a day (can be turned off), stable or pre-release channel, a banner and an Updates window with
  the release notes, Update now / Later / Skip this version, resumable downloads checked against
  the release's SHA-256 sums, an all-or-nothing swap that keeps the open map across the restart,
  and a roll back to the previous version. Package-manager installs, read-only locations and
  development builds are only told about new versions. `kfx-editor update --check` and
  `kfx-editor update` on the command line. See [Updates](manual/updates.md).
* Crash logs: an error that stops the editor is written to a log in the configuration folder,
  and the next start says where it is.

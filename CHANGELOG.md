# Changelog

All notable changes to this project are listed in this file.
The format follows Keep a Changelog (https://keepachangelog.com/en/1.1.0/) and the project uses Semantic Versioning (https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Keepers in the owner list are numbered as in level scripts: Keeper 0 (Red) is `PLAYER0`,
  Keeper 6 (Orange) is `PLAYER6`. Validation messages, the owner tooltip and the slab set
  owner picker now use the script names (`PLAYER0`, `PLAYER_GOOD`, ...).

## [0.1.0] - 2026-11-12

First version.

### Added

Maps and terrain
- Open, create, save and reload KeeperFX levels in the KeeperFX (FX) format or the original Dungeon Keeper format, with case-insensitive file names and RNC-packed files.
- Map browser of the install's campaigns, map packs and multiplayer packs, with thumbnails and a recent maps list.
- Slab painting with Paint, Line, Box, Circle, Fill, Pick and Select tools, brush sizes, ownership painting and per-slab texture styles.
- Slab placement, walls, corners, torches and room furniture follow the game's own rules; optional decoration, rounded corners, fortified walls and random doodads.
- Resize, flip, rotate, mirror symmetry with per-sector owners, cut, copy and paste with things.
- Custom slabs built from hand-made columns, a column editor for the map's columns, locked regions and notes.
- Image to map and export of the map as a PNG image.
- Named undo and redo with a History tab.

Things
- Creatures, objects, traps, doors, effect generators, hero gates, action points and lights: place, select, move, duplicate, turn and edit in the Inspector.
- Doors, keys, hero gate and action point numbers follow the game's rules.
- Modded things from the configuration layers and custom sprite archives.

Views
- Isometric, perspective, 2D and first-person fly views, minimap, grid data overlay, layer switches and script markers on the map.

Scripts
- DK script and Lua editors with completion, signature help, hover documentation, go to definition, outline, formatting and diagnostics based on the game's command tables and Lua API.
- Script wizard for the level setup, written as one marked block.
- Events editor: the DK script as event cards, a node graph, a timeline, a party and wave designer and a Lua export, sharing one text with the script editor.

Configuration and mods
- Configuration editor for rules, creatures, objects, traps and doors, rooms, magic, terrain, slab sets, column sets, cubes, tilesets, sprites and sounds, writing minimal per-level override files.
- Configuration layers and mods with the source of every value, and a creature stats comparison.

Campaigns, prefabs and texts
- Campaign and map pack editor with the land view, level list, text tables and checks, a new campaign wizard and adding the open level to a campaign.
- Prefabs: a starter set of 26, a user library and campaign libraries, with optional script fragments.
- Translation grid for level and campaign texts, with CSV and gettext (.po) exchange.
- The editor's own interface can be translated with catalogue files.

Analysis and generation
- Map analysis: reachability, dig and walk distance, territory, hero pressure and gold heat maps, routes, fairness, economy and symmetry tables.
- Map generator with five styles, seeds, symmetry modes and checked constraints, for new maps and for a selection.

Play-testing and the game
- Play and save-and-play, with the game's log read back and linked to script lines.
- Live link with a running game through its API: events, watched variables, console, quick actions and creature positions on the map.
- Sound browser, creature sound slots, level speech preview and sound hover in scripts.
- KeeperFX Workshop window: browse, install, uninstall, check for updates and package a level or campaign for upload.

Validation and safety
- Validation of the map and of the links between the map and its scripts, with a Validate panel.
- Saves are all or nothing; files are backed up before their first overwrite in a session; crash-recovery snapshots; files that cannot be read are never overwritten silently.

Command line
- Commands validate, info, script-check, export-image, regenerate and convert.
- Headless mode that renders the editor offscreen and runs a scripted list of editor actions, with an optional recording mode for tutorial videos.

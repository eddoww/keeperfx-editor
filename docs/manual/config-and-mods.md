# Config overrides and mods

KeeperFX reads every config file in layers: the install's `fxdata`, mods (`mods/load_order.cfg`),
the campaign's `CONFIGS_LOCATION`, then files named `mapNNNNN.<file>` next to the level, e.g.
`map00001.rules.cfg`, `map00001.terrain.cfg`, `map00001.slabset.toml`, `map00001.columnset.toml`,
per-creature `map00001.<creature>.cfg`, `map00001.translation.toml`, and `map00001.zip` for custom
sprites. A later layer replaces single keys of an earlier one.

In the editor:

* When a level opens, the editor builds this layer stack for it and loads the terrain, cubes,
  slab sets and column sets through it, so overrides next to the map affect what you see. The status
  bar names the layers; `kfx-editor info MAP --json` prints them (`config_layers`).
* `Configuration > Layers & mods...`: the active layers with where each value comes from, and the
  mods of `mods/load_order.cfg`; `Configuration > Ignore mods` (the game ignores mods in
  multiplayer).
* `Window > Configuration editor`: tabs for Files, Any config, Rules, Creatures, Objects,
  Traps & doors, Rooms, Magic, Terrain, Slabset, Columns, Cubes, Tileset, Sprites, Sounds. The
  editors write only what differs from the layers below into a `mapNNNNN.<file>` next to the
  level, keep comments of an existing override, back up the file they overwrite, and preview
  changes live; slabs whose slab set, columns or terrain entry changed can be regenerated.
* `File > Level files...` lists every file of the level with its role.
* `Configuration > Strings...`: the level's own strings (`mapNNNNN.<lang>.dat`) and the
  campaign's.
* Save As carries the override files along.

Mods are ignored in multiplayer and with `-nomods` in the game, and the editor follows that.

## Creature stats

Configuration > Creature stats… compares every creature side by side at a level (1-10), with the
values the game computes for that level. Click a column header to sort, type in the filter, tick
"Unlisted" for creature files missing from `creature.cfg`. A value from the campaign, a mod or
the map is coloured with its layer (the tooltip names the file). Clicking a name opens that
creature in the configuration editor.

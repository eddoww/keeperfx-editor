# Custom slabs

A custom slab is nine hand-built columns (plus optional objects or lights)
that stand in for one slab. The `.slb` stores the slab kind the game and neighbours see ("recognized
as"), `.clm` / `.dat` the columns, and the editor data file `mapNNNNN.kfxed.toml` remembers which slabs are
custom, so regenerating neighbours keeps them. Custom slab IDs start at 1000.

* Custom slabs can be created, removed, placed, and turned or mirrored with the map.
* Maps made with the Unearth editor keep their custom slabs in a `.une` file and a
  `custom_slabs.cfg`: a `.une` next to the map is read when the map opens, and the editor keeps
  it up to date when saving, so the map stays consistent for both editors. A `.une` the editor
  could not read (another map size, a truncated file) is left untouched and the status bar says
  so.
* The validator reports custom slabs without a definition or whose kind differs.
* In the Map menu: **New custom slab from selection**, **Edit column...**
  (cubes and floor of the selected slab's centre subtile, stored as a custom slab), **Lock
  selection / Unlock selection** (tools refuse to change those slabs; stored in the editor data file) and
  **Add note...** (a note pinned to a slab). Custom slabs appear in a Custom palette tab. Every
  change to them is an undo step.

The file format is in [file formats](../formats.md).

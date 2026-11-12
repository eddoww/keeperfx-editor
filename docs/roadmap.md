# Known limitations and open work

What the editor does not do, or does only in part, grouped by area. Some limits are deliberate
and are marked as such. The [features](features.md) page lists what works.

## Platforms and distribution

* The editor is developed and tested on Linux. The Windows and macOS builds are compiled and
  tested in CI only.
* The Windows executable is a console program, so that the command line modes can print; starting
  the window from Explorer also opens a console window.
* The Windows and macOS builds are not signed, and there is no installer.
* No application icon is shipped yet; the desktop entry falls back to a generic icon.
* The optional check for a newer release only reports a version. It does nothing until a release
  address is entered in the preferences, and it never downloads or installs anything (by design).
* The game-harness tests start a native Linux build of the game and run on Unix only.

## Install and file formats

* The editor needs a KeeperFX install. It does not work from the original Dungeon Keeper files
  alone (by design).

## Terrain and columns

* The wall art of Fortify in its "Grouped" mode follows the game's rule, but measures the distance
  to the heart with a Euclidean distance where the game uses an octagonal approximation, so a wall
  on the edge of a band can get the neighbouring band's art.
* Rounded corners copy the column of the neighbouring slab; there are no separate transition
  columns for gold.
* The editor for the map's columns has a side view of a column but no 3D view, and no read-only
  mode. A column edited there is rebuilt when its slab or a neighbour is regenerated, unless it
  belongs to a custom slab.

## Configuration

* The tileset editor writes texture files to disk at once instead of with the rest of the level
  on Save. A map-local texture file identical to the inherited one is not removed automatically.
  Texture packs are exported as one PNG per tile.

## Level properties and campaigns

* The ensign picker on the land view draws the ensign as a marker, not with the game's ensign
  sprite, and does not draw the land view window frame.

## Scripts and the Events editor

* The Events editor shows only the DK script; the Lua script is edited as text.
* Scripts the Events editor cannot represent exactly (for example an `IF` that is never closed)
  are shown read-only there and must be edited in the text tab.
* Hovering a sound argument shows and plays the sound in DK scripts only, not in Lua.
* `F3` is both the script panel shortcut and Find next in the script editor; which one acts
  depends on the keyboard focus.

## Validation and analysis

* Reachability in the validator is a heuristic. The per-player creature limit set by the script
  and room sizes are not checked.
* The analysis works per slab: columns, room furniture and height steps inside a slab are
  ignored. Times are for a single imp, so absolute dig times are pessimistic, though they compare
  keepers and maps fairly.
* The generator's Radial symmetry places keepers at equal angles but is close to, not exactly,
  symmetric.

## Play-testing and the live link

* Turning the game's API on for one game uses the game's `-cfg` command line option, which older
  KeeperFX builds do not have; with those, set the API keys in `keeperfx.cfg` instead.
* An edited script cannot be loaded into a running game; the level has to be restarted.
* The game's camera can be moved to action points, hero gates and hearts; any other place needs
  the game started in cheat mode.
* The game's API refuses commands while the game is paused, and only flags and a few counters can
  be written.
* Creature positions are read from Lua output in `keeperfx.log`. When Lua runs through a script
  command it uses the level's script string space; automatic refresh stops before that space runs
  out.
* The game listens on `127.0.0.1` only, for one client at a time.

## Sounds

* Previews play the sample itself: the game's 3D positioning, its sound mixing modes and music
  streaming are not reproduced.
* Unsaved edits of `sounds.cfg` are not heard until they are saved.

## Workshop

* RAR archives can only be read when an external `7z`, `7zz` or `7za` program is installed.
* Uploading is done in the browser; the editor prepares the zip and the form text (by design).

## Interface languages

* Texts of the script, configuration, campaign, Events, prefab, sound, live link and Workshop
  panels are not translatable yet; they are marked as drafts in `data/i18n/en.toml`. Only English
  ships with the editor.
* Undo history names, the last status message and some hover texts keep the language they were
  created in until they are made again.

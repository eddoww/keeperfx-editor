# Troubleshooting

**"has no fxdata/ and no data/"** in the folder dialog: choose the folder that contains `fxdata/`
and `data/`, not a parent or a campaign folder.

**Menus are greyed out, nothing loads**: no valid install yet; set it in `Configuration > KeeperFX
folder...` or with `KFX_INSTALL`.

**The window does not start / GPU errors**: the renderer uses wgpu; install a Vulkan driver (on
Linux). Under Wayland or X11 both are supported.

**"cannot tell the size of mapNNNNN"**: the `.slb` is not square and no `.lof` or campaign block
gives `MAPSIZE`. Add `MAPSIZE = W H` to `mapNNNNN.lof`.

**The game does not find my map when playing**: the map must be saved, and inside a campaign or
map pack folder (or let the editor copy it to `levels/personal`). Check the Game log.

**Play does nothing on Linux with a `.exe`**: install `wine`, or set the wrapper command in
Preferences > Play.

**A file changed on disk**: the editor detects it when it regains focus and asks whether to reload
or keep your version. Files the editor writes itself (configuration overrides, string tables, the
`.lof`) do not count as changes.

## Some files of this map could not be read

When a `.tngfx` (things), `.aptfx` (action points), `.lgtfx` (lights) or the editor's
`mapNNNNN.kfxed.toml` exists but cannot be read (usually a typing mistake after editing it by hand),
the editor does not pretend it is empty: a dialog names each file and the error with its line and
column. Choose:

* **Open read-only**: the map is shown without what those files hold, and **Save is refused**
  (the status bar and the Validate tab say why), so the file stays as it is. Fix it in a text
  editor (the error says where) and open the map again.
* **Open and discard the unreadable things (a backup is kept)**: the level's files are copied to
  the backups folder first, then the map can be saved; saving replaces the unreadable file with
  what the editor shows (no things, for an unreadable `.tngfx`).

For an unreadable `.lgtfx` the game itself falls back to the legacy `.tng` / `.apt` / `.lgt`, and
so does the editor's view. The editor reads these files the way the game does, so a file the game
loads also opens in the editor. From the command line, such a map makes every command exit with a
non-zero code, and `convert` / `regenerate` refuse to write.

**"the game refuses to load a level with N thing(s) it cannot create"** when saving: see the
`thing-class-not-loadable` / `thing-subtype-name` errors in the Validate tab, and change or delete
those things.

**"Not saved: ... read-only"** with a permission error: a file of the map is marked read-only (or
the folder is not writable). Nothing was changed; make the files writable and save again.

**Lost work after a crash**: autosave snapshots are written to the `autosave/` folder of the config
directory while changes are unsaved; older file versions are in `backups/` (`File > Backups...`).

**The settings are broken**: delete `settings.json` in the config folder (shown in Preferences >
Files), or use Preferences > Reset to defaults.

## Fonts

Text from the game (17 languages), campaigns, the Workshop and file names can use any script. The
editor's own fonts cover Latin, Greek, Cyrillic and common symbols; for anything else it asks the
operating system for an installed font that has the character (fontconfig on Linux, DirectWrite
on Windows, CoreText on macOS). No fonts are bundled.

**Chinese, Japanese or Korean text shows as boxes**: no font for that script is installed. Install
one (on Linux for example the Noto CJK fonts: `noto-fonts-cjk` on Arch, `fonts-noto-cjk` on
Debian/Ubuntu) and restart the editor. A character that no installed font has is always drawn as
a box; the text itself is kept unchanged and saved as it was.

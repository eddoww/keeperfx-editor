# Play-testing

`Level > Play` (**F5**) starts the game on the open level; **Shift+F5** saves first. The game log
(`Level > Game log`, **F6**) reads `keeperfx.log` afterwards and lists errors and warnings, with
filters for warnings and for script lines; script errors point at the script line.

Right-clicking **Play** (or the Level menu) offers **Play and record (packet save)**, which starts
the game with `-packetsave`, and **Replay the last recording (packet load)**.

## How the level is chosen

The game starts with `-campaign NAME -level N`. The editor finds the campaign or map pack whose
levels folder holds your map. A map outside every pack is "loose": the editor offers to copy it
into `levels/personal` of the install (with a `.lof`, `KIND = FREE`) under a free level number.
The map must be saved first.

## Settings (Preferences > Play)

Game executable (default: `keeperfx` on Linux if present, else `keeperfx.exe`; run through `wine`
automatically when not on Windows), a wrapper command, extra arguments, `-nointro`,
`-skipheartzoom`, `-nosound`, `-1player` for multiplayer maps, `-alex`, windowed play with a size,
and "save before play". The full command is shown so you can copy it. The game runs as a separate
process; the editor stays usable.

# Validation

`Level > Validate` or the toolbar **Validate** button opens the Validate tab of the right panel:
counts of errors, warnings and notes, a **Re-check** button, and the list; selecting an entry jumps
to the slab, subtile or thing. The status bar shows a validity indicator. Map checks (from
`kfx-map`, each citing the game rule):

| Severity | Codes |
|---|---|
| Error | `file-unreadable`, `things-limit`, `creatures-limit`, `action-points-limit`, `lights-limit`, `column-pool`, `slab-owner-range`, `thing-class`, `thing-class-not-loadable`, `thing-subtype-name`, `thing-model`, `thing-model-unknown`, `thing-owner-range`, `thing-position`, `thing-outside`, `door-cannot-stand`, `hero-gate-number`, `action-point-number`, `action-point-outside`, `light-outside` |
| Warning | `thing-owner-range` (owner 9: the game's FX loader accepts it, it is no player), `slab-kind-unknown`, `custom-slab-undefined`, `custom-slab-kind`, `door-slab-without-door`, `door-not-on-door-slab`, `door-off-centre`, `thing-in-solid`, `hero-gate-duplicate`, `keeper-without-heart`, `action-point-duplicate`, `thing-unreachable` |
| Note | `door-slab-orientation`, `keeper-several-hearts`, `gold-in-treasury`, `creature-unreachable` |

`thing-class`, `thing-class-not-loadable` (a class the game cannot create from a `.tngfx`, such as
Shot), `thing-subtype-name` (a `SubtypeStringID` that is not a string or names nothing) and
`thing-model` make the game refuse the **whole level**; the editor will not save a map with such
things. `file-unreadable`: a thing, action point, light or editor data file (`.kfxed.toml`) could not be read and the
map is read-only (see [Troubleshooting](troubleshooting.md)).

Reachability is a heuristic. Not checked: the per-player creature limit set by the script, room
sizes. The Validate tab also checks links between the map and the scripts (script references to
action points and hero gates that do not exist). From a terminal: `kfx-editor validate MAP`
(exit code 1 on errors, `--json` for tools).

The Analysis window adds findings with codes starting with `analysis-` (unreachable portals,
keepers without gold, isolated hero gates and pockets): see
[Map analysis](analysis.md#findings-in-the-validate-panel).

# Keyboard shortcuts and mouse controls

The default keys and mouse controls of the editor. The same list is in the editor under `Help > Controls` (F1), and the keys marked editable can be changed in `Configuration > Keyboard shortcuts` (also the Shortcuts tab of Preferences). Changed keys are stored in the settings file as overrides of these defaults. "Cmd" is accepted as Ctrl in the settings.

## Keyboard (defaults)

### File

| Key | Action | Active | Editable |
|---|---|---|---|
| `Ctrl+N` | New map | anywhere | yes |
| `Ctrl+O` | Open map | anywhere | yes |
| `Ctrl+S` | Save map and scripts | anywhere | yes |
| `Ctrl+Shift+S` | Save map as | anywhere | yes |
| `Ctrl+B` | Map browser | anywhere | yes |

### Play

| Key | Action | Active | Editable |
|---|---|---|---|
| `F5` | Play the level | anywhere | yes |
| `Shift+F5` | Save and play | anywhere | yes |
| `F6` | Game log | anywhere | yes |
| `F7` | Live link with the running game | anywhere | yes |

### View

| Key | Action | Active | Editable |
|---|---|---|---|
| `F3` | Script panel | anywhere | yes |
| `F4` | Maximise the script editor over the map | anywhere | yes |
| `F2` | 2D / 3D view | map view (no text field focused) | yes |
| `Shift+F2` | Split view (2D + 3D) | map view (no text field focused) | yes |
| `V` | First-person fly view on / off (WASD, Q/E down/up, Shift faster) | map view (no text field focused) | yes |
| `M` | Fly view: capture the mouse for mouse-look on / off | map view (no text field focused) | yes |
| `Ctrl+F3` | Events editor (the script panel's Events tab) / back to the text | anywhere | yes |
| `Ctrl+Shift+P` | Prefabs panel | anywhere | yes |
| `Ctrl+Shift+A` | Analysis panel (layers, routes, fairness) | anywhere | yes |

### Help

| Key | Action | Active | Editable |
|---|---|---|---|
| `F1` | Controls reference | anywhere | yes |

### Edit

| Key | Action | Active | Editable |
|---|---|---|---|
| `Ctrl+Z` | Undo | map view (no text field focused) | yes |
| `Ctrl+Y` | Redo | map view (no text field focused) | yes |
| `Ctrl+Shift+Z` | Redo (alternative) | map view (no text field focused) | yes |
| `Ctrl+A` | Select all slabs | map view (no text field focused) | yes |
| `Ctrl+C` | Copy | map view (no text field focused) | no (system clipboard keys) |
| `Ctrl+X` | Cut | map view (no text field focused) | no (system clipboard keys) |
| `Ctrl+V` | Paste | map view (no text field focused) | no (system clipboard keys) |
| `Delete` | Delete things / fill the selection | map view (no text field focused) | yes |
| `Escape` | Cancel the tool, stop pasting or picking | map view (no text field focused) | yes |

### Things

| Key | Action | Active | Editable |
|---|---|---|---|
| `Ctrl+D` | Duplicate the selected things | map view (no text field focused) | yes |
| `Space` | Lock / unlock the door under the pointer | map view (no text field focused) | yes |
| `R` | Turn the selected things right | with things selected | yes |
| `Shift+R` | Turn the selected things left | with things selected | yes |

### Tools

| Key | Action | Active | Editable |
|---|---|---|---|
| `OpenBracket` | Smaller brush | map view (no text field focused) | yes |
| `CloseBracket` | Larger brush | map view (no text field focused) | yes |

### Camera

| Key | Action | Active | Editable |
|---|---|---|---|
| `W` | Pan up | map view (no text field focused) | yes |
| `S` | Pan down | map view (no text field focused) | yes |
| `A` | Pan left | map view (no text field focused) | yes |
| `D` | Pan right | map view (no text field focused) | yes |
| `Q` | Rotate 45 degrees left | map view (no text field focused) | yes |
| `E` | Rotate 45 degrees right | map view (no text field focused) | yes |
| `Home` | Reset the view (game default) | map view (no text field focused) | yes |

### Paste

| Key | Action | Active | Editable |
|---|---|---|---|
| `R` | Rotate the clipboard | while pasting slabs | yes |
| `F` | Flip the clipboard | while pasting slabs | yes |

### Campaign

| Key | Action | Active | Editable |
|---|---|---|---|
| `Ctrl+Shift+K` | Campaign editor | anywhere | yes |
| `Ctrl+Shift+W` | Workshop | anywhere | yes |

### Map

| Key | Action | Active | Editable |
|---|---|---|---|
| `Ctrl+Shift+G` | Generate terrain into the selection | map view (no text field focused) | yes |

Key names are egui key names: `OpenBracket` is `[` and `CloseBracket` is `]`. A shortcut that is shared between contexts (for example `R` rotates the clipboard while pasting and the selected things otherwise) never fires twice. Two shortcuts in contexts that can be active together are reported as a conflict in the shortcuts editor.

## Mouse (map view)

| Input | Effect |
|---|---|
| Left click / drag | Use the active tool |
| Alt+click | Pick slab and owner (eyedropper); with the thing tools: copy a thing's settings |
| Middle drag / Shift+left drag | Pan |
| Right drag | Orbit |
| Wheel | Zoom to the cursor |
| Things tool: click / Shift+click | Select / add to the selection; click again to cycle a stack |
| Things tool: drag | Move the selection, or drag a box on empty ground |
| Place tool: Shift+click | Keep placing |
| Script marker: hover / click | Show the script line / jump to it in the script editor |
| Arrow keys | Pan (as well as the pan keys) |

## Script editor (when the script editor has the focus)

These keys are fixed; they are not part of the shortcut table above.

| Key | Action |
|---|---|
| `Ctrl+Space` | Complete |
| `Ctrl+F` / `Ctrl+H` | Find / find and replace |
| `F3` / `Shift+F3` | Find next / previous |
| `Ctrl+G` | Go to line |
| `Ctrl+/` | Comment / uncomment the selected lines (a comment toggle; the comment marker depends on the language) |
| `Alt+Shift+F` | Re-indent (format) the script |
| `F12` | Go to definition |
| `Tab` / `Shift+Tab` | Indent / unindent; in the completion popup `Tab` accepts, `Up`/`Down` move, `Esc` closes |
| `Ctrl+S` | Save this file (the toolbar tooltip); `Ctrl+S` of the main window saves map and scripts |
| `Esc` | Close the completion popup (and other transient bars) |

`F3` is also the global "Script panel" shortcut of the main window; which one acts depends on the keyboard focus.

//! Keyboard shortcuts: one table of actions with their default keys ([`ACTIONS`]), user
//! overrides from the settings, conflict detection, and the reference text of the Help window.
use eframe::egui::{self, Key, Modifiers};
use kfx_prefab::i18n::trd;
use std::collections::HashMap;

/// When a shortcut is active. Shortcuts of different contexts other than `Global` never fire
/// together, so they may share keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    /// Anywhere (also while a text field has focus for the F-keys and Ctrl+S).
    Global,
    /// The map view without a text field focused.
    Map,
    /// While pasting slabs.
    Paste,
    /// With things selected.
    Things,
}

pub struct ActionDef {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub context: Context,
    /// Default shortcut (`""` = none).
    pub default: &'static str,
    /// Can be changed in the shortcuts editor (clipboard keys come from the system).
    pub editable: bool,
}

impl ActionDef {
    /// The label shown in the editor: key `kfx_editor.action.<id>`, English from [`ACTIONS`]
    /// (the extractor reads the table, see `kfx_prefab::i18n::scan::TABLES`).
    pub fn title(&self) -> String {
        trd(&format!("kfx_editor.action.{}", self.id), self.label)
    }
    /// The group shown in the editor: key `kfx_editor.action_group.<slug of the group>`.
    pub fn group_title(&self) -> String {
        trd(
            &format!(
                "kfx_editor.action_group.{}",
                kfx_prefab::i18n::scan::slug(self.group)
            ),
            self.group,
        )
    }
}

const fn a(
    id: &'static str,
    label: &'static str,
    group: &'static str,
    context: Context,
    default: &'static str,
) -> ActionDef {
    ActionDef {
        id,
        label,
        group,
        context,
        default,
        editable: true,
    }
}

const fn fixed(
    id: &'static str,
    label: &'static str,
    group: &'static str,
    context: Context,
    default: &'static str,
) -> ActionDef {
    ActionDef {
        id,
        label,
        group,
        context,
        default,
        editable: false,
    }
}

pub const ACTIONS: &[ActionDef] = &[
    a("file.new", "New map", "File", Context::Global, "Ctrl+N"),
    a("file.open", "Open map", "File", Context::Global, "Ctrl+O"),
    a(
        "file.save",
        "Save map and scripts",
        "File",
        Context::Global,
        "Ctrl+S",
    ),
    a(
        "file.save_as",
        "Save map as",
        "File",
        Context::Global,
        "Ctrl+Shift+S",
    ),
    a(
        "file.browser",
        "Map browser",
        "File",
        Context::Global,
        "Ctrl+B",
    ),
    a("play.play", "Play the level", "Play", Context::Global, "F5"),
    a(
        "play.save_play",
        "Save and play",
        "Play",
        Context::Global,
        "Shift+F5",
    ),
    a("play.log", "Game log", "Play", Context::Global, "F6"),
    a("view.script", "Script panel", "View", Context::Global, "F3"),
    a(
        "view.script_max",
        "Maximise the script editor over the map",
        "View",
        Context::Global,
        "F4",
    ),
    a(
        "help.controls",
        "Controls reference",
        "Help",
        Context::Global,
        "F1",
    ),
    a("edit.undo", "Undo", "Edit", Context::Map, "Ctrl+Z"),
    a("edit.redo", "Redo", "Edit", Context::Map, "Ctrl+Y"),
    a(
        "edit.redo2",
        "Redo (alternative)",
        "Edit",
        Context::Map,
        "Ctrl+Shift+Z",
    ),
    a(
        "edit.select_all",
        "Select all slabs",
        "Edit",
        Context::Map,
        "Ctrl+A",
    ),
    fixed("edit.copy", "Copy", "Edit", Context::Map, "Ctrl+C"),
    fixed("edit.cut", "Cut", "Edit", Context::Map, "Ctrl+X"),
    fixed("edit.paste", "Paste", "Edit", Context::Map, "Ctrl+V"),
    a(
        "edit.delete",
        "Delete things / fill the selection",
        "Edit",
        Context::Map,
        "Delete",
    ),
    a(
        "edit.cancel",
        "Cancel the tool, stop pasting or picking",
        "Edit",
        Context::Map,
        "Escape",
    ),
    a(
        "edit.duplicate",
        "Duplicate the selected things",
        "Things",
        Context::Map,
        "Ctrl+D",
    ),
    a(
        "brush.smaller",
        "Smaller brush",
        "Tools",
        Context::Map,
        "OpenBracket",
    ),
    a(
        "brush.larger",
        "Larger brush",
        "Tools",
        Context::Map,
        "CloseBracket",
    ),
    a("cam.up", "Pan up", "Camera", Context::Map, "W"),
    a("cam.down", "Pan down", "Camera", Context::Map, "S"),
    a("cam.left", "Pan left", "Camera", Context::Map, "A"),
    a("cam.right", "Pan right", "Camera", Context::Map, "D"),
    a(
        "cam.rot_left",
        "Rotate 45 degrees left",
        "Camera",
        Context::Map,
        "Q",
    ),
    a(
        "cam.rot_right",
        "Rotate 45 degrees right",
        "Camera",
        Context::Map,
        "E",
    ),
    a(
        "cam.home",
        "Reset the view (game default)",
        "Camera",
        Context::Map,
        "Home",
    ),
    a("view.2d", "2D / 3D view", "View", Context::Map, "F2"),
    a(
        "view.split",
        "Split view (2D + 3D)",
        "View",
        Context::Map,
        "Shift+F2",
    ),
    a(
        "view.fly",
        "First-person fly view on / off (WASD, Q/E down/up, Shift faster)",
        "View",
        Context::Map,
        "V",
    ),
    a(
        "view.fly_look",
        "Fly view: capture the mouse for mouse-look on / off",
        "View",
        Context::Map,
        "M",
    ),
    a(
        "paste.rotate",
        "Rotate the clipboard",
        "Paste",
        Context::Paste,
        "R",
    ),
    a(
        "paste.flip",
        "Flip the clipboard",
        "Paste",
        Context::Paste,
        "F",
    ),
    a(
        "things.door_lock",
        "Lock / unlock the door under the pointer",
        "Things",
        Context::Map,
        "Space",
    ),
    a(
        "things.rotate_cw",
        "Turn the selected things right",
        "Things",
        Context::Things,
        "R",
    ),
    a(
        "things.rotate_ccw",
        "Turn the selected things left",
        "Things",
        Context::Things,
        "Shift+R",
    ),
    a(
        "view.events",
        "Events editor (the script panel's Events tab) / back to the text",
        "View",
        Context::Global,
        "Ctrl+F3",
    ),
    a(
        "view.prefabs",
        "Prefabs panel",
        "View",
        Context::Global,
        "Ctrl+Shift+P",
    ),
    a(
        "campaign.editor",
        "Campaign editor",
        "Campaign",
        Context::Global,
        "Ctrl+Shift+K",
    ),
    a(
        "workshop.open",
        "Workshop",
        "Campaign",
        Context::Global,
        "Ctrl+Shift+W",
    ),
    a(
        "play.live",
        "Live link with the running game",
        "Play",
        Context::Global,
        "F7",
    ),
    a(
        "view.analysis",
        "Analysis panel (layers, routes, fairness)",
        "View",
        Context::Global,
        "Ctrl+Shift+A",
    ),
    a(
        "map.generate_selection",
        "Generate terrain into the selection",
        "Map",
        Context::Map,
        "Ctrl+Shift+G",
    ),
];

/// A key with modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    /// `Ctrl+Shift+S`, `F5`, `OpenBracket` (egui key names; `Cmd` = `Ctrl`).
    pub fn parse(s: &str) -> Option<Shortcut> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut sc = Shortcut {
            ctrl: false,
            shift: false,
            alt: false,
            key: Key::A,
        };
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (last, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" | "control" => sc.ctrl = true,
                "shift" => sc.shift = true,
                "alt" | "option" => sc.alt = true,
                _ => return None,
            }
        }
        sc.key = Key::from_name(last).or_else(|| {
            Key::ALL.iter().copied().find(|k| {
                k.name().eq_ignore_ascii_case(last) || k.symbol_or_name().eq_ignore_ascii_case(last)
            })
        })?;
        Some(sc)
    }

    /// Text form, parsed back by [`Shortcut::parse`].
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        s.push_str(self.key.name());
        s
    }

    /// For menus: `Ctrl+[` rather than `Ctrl+OpenBracket`.
    pub fn label(&self) -> String {
        let t = self.text();
        t.trim_end_matches(self.key.name()).to_string() + self.key.symbol_or_name()
    }

    /// The modifiers held are exactly these.
    fn mods_match(&self, m: Modifiers) -> bool {
        m.command == self.ctrl && m.shift == self.shift && m.alt == self.alt
    }
}

/// The active shortcuts (defaults with the user's overrides).
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    keys: HashMap<&'static str, Option<Shortcut>>,
}

impl Keymap {
    /// Defaults, then `overrides` ((action id, shortcut text); an empty text removes the key).
    pub fn new(overrides: &[(String, String)]) -> Keymap {
        let mut keys = HashMap::new();
        for d in ACTIONS {
            keys.insert(d.id, Shortcut::parse(d.default));
        }
        for (id, s) in overrides {
            if let Some(d) = ACTIONS.iter().find(|d| d.id == id && d.editable) {
                keys.insert(d.id, Shortcut::parse(s));
            }
        }
        Keymap { keys }
    }

    pub fn get(&self, id: &str) -> Option<Shortcut> {
        self.keys.get(id).copied().flatten()
    }

    pub fn set(&mut self, id: &'static str, s: Option<Shortcut>) {
        self.keys.insert(id, s);
    }

    /// Settings form: only the actions that differ from their default.
    pub fn overrides(&self) -> Vec<(String, String)> {
        ACTIONS
            .iter()
            .filter(|d| self.get(d.id) != Shortcut::parse(d.default))
            .map(|d| {
                (
                    d.id.to_string(),
                    self.get(d.id).map(|s| s.text()).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Menu text of an action's shortcut.
    pub fn label(&self, id: &str) -> String {
        self.get(id).map(|s| s.label()).unwrap_or_default()
    }

    /// Pairs of actions with the same shortcut in contexts that can be active together.
    pub fn conflicts(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![];
        for (i, a) in ACTIONS.iter().enumerate() {
            for b in &ACTIONS[i + 1..] {
                let (Some(x), Some(y)) = (self.get(a.id), self.get(b.id)) else {
                    continue;
                };
                let together = a.context == b.context
                    || a.context == Context::Global
                    || b.context == Context::Global;
                if x == y && together {
                    out.push((a.id, b.id));
                }
            }
        }
        out
    }

    /// The action's shortcut was pressed this frame (consumed so that nothing else sees it).
    pub fn pressed(&self, ctx: &egui::Context, id: &str) -> bool {
        let Some(s) = self.get(id) else { return false };
        ctx.input_mut(|i| {
            let hit = i.events.iter().position(|e| {
                matches!(
                    e,
                    egui::Event::Key { key, pressed: true, modifiers, .. }
                        if *key == s.key && s.mods_match(*modifiers)
                )
            });
            match hit {
                Some(k) => {
                    i.events.remove(k);
                    true
                }
                None => false,
            }
        })
    }

    /// The action's key is held (camera pans) with exactly its modifiers.
    pub fn held(&self, i: &egui::InputState, id: &str) -> bool {
        self.get(id)
            .is_some_and(|s| i.key_down(s.key) && s.mods_match(i.modifiers))
    }
}

/// The key the user pressed this frame (for the shortcuts editor), ignoring lone modifiers.
pub fn captured(ctx: &egui::Context) -> Option<Shortcut> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => Some(Shortcut {
                ctrl: modifiers.command,
                shift: modifiers.shift,
                alt: modifiers.alt,
                key: *key,
            }),
            _ => None,
        })
    })
}

/// Mouse controls of the map view (for the reference window).
pub const MOUSE: &[(&str, &str)] = &[
    ("Left click / drag", "Use the active tool"),
    (
        "Alt+click",
        "Pick slab and owner (eyedropper); with the thing tools: copy a thing's settings",
    ),
    ("Middle drag / Shift+left drag", "Pan"),
    ("Right drag", "Orbit"),
    ("Wheel", "Zoom to the cursor"),
    (
        "Things tool: click / Shift+click",
        "Select / add to the selection; click again to cycle a stack",
    ),
    (
        "Things tool: drag",
        "Move the selection, or drag a box on empty ground",
    ),
    ("Place tool: Shift+click", "Keep placing"),
    (
        "Script marker: hover / click",
        "Show the script line / jump to it in the script editor",
    ),
    ("Arrow keys", "Pan (as well as the pan keys)"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format() {
        let s = Shortcut::parse("Ctrl+Shift+S").unwrap();
        assert!(s.ctrl && s.shift && !s.alt && s.key == Key::S);
        assert_eq!(Shortcut::parse(&s.text()), Some(s));
        assert_eq!(Shortcut::parse("F5").unwrap().key, Key::F5);
        assert_eq!(
            Shortcut::parse("OpenBracket").unwrap().key,
            Key::OpenBracket
        );
        assert!(Shortcut::parse("Hyper+X").is_none());
        for d in ACTIONS {
            assert!(Shortcut::parse(d.default).is_some(), "{}", d.id);
        }
    }

    #[test]
    fn defaults_have_no_conflicts_and_overrides_detect_them() {
        let k = Keymap::new(&[]);
        assert!(k.conflicts().is_empty(), "{:?}", k.conflicts());
        // R is shared by paste and things on purpose (different contexts)
        assert_eq!(k.get("paste.rotate"), k.get("things.rotate_cw"));
        let k = Keymap::new(&[("play.play".into(), "Ctrl+S".into())]);
        assert_eq!(k.conflicts(), vec![("file.save", "play.play")]);
        assert_eq!(
            k.overrides(),
            vec![("play.play".to_string(), "Ctrl+S".to_string())]
        );
        // removing a key
        let k = Keymap::new(&[("cam.home".into(), "".into())]);
        assert!(k.get("cam.home").is_none());
        // fixed keys cannot be overridden
        let k = Keymap::new(&[("edit.copy".into(), "F9".into())]);
        assert_eq!(k.get("edit.copy").unwrap().key, Key::C);
    }
}

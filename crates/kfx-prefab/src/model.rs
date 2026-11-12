//! The prefab data model and its file format (`*.kfxprefab`).
//!
//! # File format (version 1)
//!
//! A prefab file is one UTF-8 TOML document. Keys are written in a fixed order, lists are sorted
//! (things by row, column, class, model; palette in order of first use) so that saving an
//! unchanged prefab gives identical bytes and diffs stay small. Reading is forward compatible:
//! every key or table this version does not know is kept (in the `extra` field of the structure
//! it was found in) and written back unchanged; a file whose `format` is newer than
//! [`FORMAT_VERSION`] still loads, with a warning in [`Prefab::warnings`].
//!
//! ```toml
//! format = 1                       # integer, required
//! thumbnail = "iVBORw0..."         # optional: base64 of a PNG (host or generator supplied)
//!
//! [meta]
//! name = "Starter keep"            # shown in the library
//! author = "..."
//! description = "..."
//! tags = ["dungeon", "start"]
//! editor_version = "0.1.0"         # the editor that wrote the file
//! [meta.requires]                  # names the target map's configuration must define
//! creatures = ["IMP"]              # (informational: the browser recomputes them from the content)
//! objects = [...]  traps = [...]  doors = [...]  effect_generators = [...]  slabs = [...]
//! config = ["free text, e.g. a mod name"]
//!
//! [layout]                         # the slabs: one string per row, one character per slab
//! width = 7
//! height = 5
//! rows = ["#######", "#..H..#", ...]   # "." = not part of the prefab (masked): never written
//!
//! [[palette]]                      # what each other character stands for
//! char = "H"
//! slab = "HEART_PEDESTAL"          # slab kind by NAME (terrain.cfg), never by number
//! owner = "A"                      # "neutral", "A".."H" (relative player slots), "keep:N" (player N)
//! wlb = 3                          # optional: .wlb value when it differs from the kind's default
//! slx = 2                          # optional: .slx texture style (default 0)
//! custom = "name"                  # optional: this slab is the custom slab definition of that name
//!
//! [[thing]]                        # objects, creatures, traps, doors, effect generators
//! class = "Object"                 # Object | Creature | Trap | Door | EffectGen
//! model = "SOUL_CONTAINER"         # by NAME (config), "#N" for a model without a name
//! owner = "A"
//! x = [4, 128]                     # [subtile, fraction/256] from the prefab's north-west corner
//! y = [4, 128]
//! above = [0, 0]                   # height above the floor under it, same unit
//! orientation = 512                # optional angle 0..2047 (0 north, clockwise)
//! level = 3  gold = 0  health = 0  name = "..."        # creatures, optional
//! door_orientation = 0  door_locked = false             # doors
//! gate = "gate1"                   # hero gates: placeholder key, renumbered on apply
//! custom_box = 1  gold_value = 100  effect_range = [3, 0]
//!
//! [[action_point]]
//! key = "ap1"                      # placeholder key, renumbered on apply
//! x = [3, 128]  y = [3, 128]  range = [4, 0]  number = 1  # number: informational (the original)
//!
//! [[light]]
//! x = [3, 128]  y = [3, 128]  above = [3, 0]  range = [6, 0]  intensity = 36  dynamic = false
//!
//! [[custom_slab]]                  # custom slab definitions the layout needs
//! name = "my slab"  recognized_as = "PATH"  liquid = 1
//! wibble = [0, 0, 0, 0, 0, 0, 0, 0, 0]
//! columns = [ { cubes = ["NAME", ...x8], floor = 5 }, ...x9 ]   # row major, cubes by name ("#N")
//! objects = [ { kind = "object", model = "TORCH", dx = 10, dy = 20, z = 3, orientation = 0 } ]
//!
//! [script]                         # optional fragments appended to the target level script
//! dk = '''...'''                   # DK script text
//! lua = '''...'''                  # Lua text
//! ```
//!
//! Script placeholders: `{{ap.KEY}}` becomes the number the action point `KEY` got,
//! `{{gate.KEY}}` the number of the hero gate `KEY`, `{{player.A}}` the player the slot `A` was
//! mapped to (`PLAYER0`..., `PLAYER_GOOD`, `PLAYER_NEUTRAL`). See [`crate::script`].
use crate::{Error, Result};
use std::collections::BTreeMap;
use toml::{Table, Value};

/// The format version this crate writes and fully understands.
pub const FORMAT_VERSION: u32 = 1;
/// File extension of prefab files.
pub const FILE_EXT: &str = "kfxprefab";
/// Largest prefab side (the game's map size limit, `MAX_TILES_X`).
pub const MAX_SIDE: u32 = 170;
/// Raw coordinate units per subtile / per slab.
pub const RAW_PER_STL: i32 = 256;
pub const RAW_PER_SLAB: i32 = 768;

/// Who owns a slab or thing in a prefab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    /// Player 5, the neutral owner.
    Neutral,
    /// A relative player slot (0 = "A", 1 = "B", ...), mapped to a real player on apply.
    Slot(u8),
    /// A fixed player number that is kept as it is ("keep:4" for the hero player).
    Fixed(u8),
}

impl Owner {
    pub fn to_text(self) -> String {
        match self {
            Owner::Neutral => "neutral".into(),
            Owner::Slot(s) => ((b'A' + s.min(25)) as char).to_string(),
            Owner::Fixed(n) => format!("keep:{n}"),
        }
    }
    pub fn parse(s: &str) -> Option<Owner> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("neutral") {
            return Some(Owner::Neutral);
        }
        if let Some(n) = t.strip_prefix("keep:") {
            return n.trim().parse().ok().map(Owner::Fixed);
        }
        let mut c = t.chars();
        match (c.next(), c.next()) {
            (Some(a), None) if a.is_ascii_uppercase() => Some(Owner::Slot(a as u8 - b'A')),
            _ => None,
        }
    }
}

/// One slab of the layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// Slab kind name (`HEART_PEDESTAL`). For a custom slab the kind the game sees.
    pub slab: String,
    pub owner: Owner,
    /// `.wlb` value when it differs from the default of the kind (bridges over liquid).
    pub wlb: Option<u8>,
    /// `.slx` texture style, 0 = the map's.
    pub slx: u8,
    /// Name of the custom slab definition (`[[custom_slab]]`) standing here.
    pub custom: Option<String>,
}

impl Cell {
    pub fn new(slab: &str, owner: Owner) -> Cell {
        Cell {
            slab: slab.to_string(),
            owner,
            wlb: None,
            slx: 0,
            custom: None,
        }
    }
}

/// Thing classes a prefab can hold (those the FX loader creates and that make sense to stamp).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThingClass {
    Object,
    Creature,
    EffectGen,
    Trap,
    Door,
}

impl ThingClass {
    pub const ALL: [ThingClass; 5] = [
        ThingClass::Object,
        ThingClass::Creature,
        ThingClass::EffectGen,
        ThingClass::Trap,
        ThingClass::Door,
    ];
    /// `TCls_*` number.
    pub fn id(self) -> u8 {
        match self {
            ThingClass::Object => 1,
            ThingClass::Creature => 5,
            ThingClass::EffectGen => 7,
            ThingClass::Trap => 8,
            ThingClass::Door => 9,
        }
    }
    pub fn from_id(id: u8) -> Option<ThingClass> {
        ThingClass::ALL.into_iter().find(|c| c.id() == id)
    }
    pub fn name(self) -> &'static str {
        match self {
            ThingClass::Object => "Object",
            ThingClass::Creature => "Creature",
            ThingClass::EffectGen => "EffectGen",
            ThingClass::Trap => "Trap",
            ThingClass::Door => "Door",
        }
    }
    pub fn from_name(s: &str) -> Option<ThingClass> {
        ThingClass::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// A thing of the prefab. Positions are raw 1/256 subtile offsets from the prefab's north-west
/// corner (`768` per slab).
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabThing {
    pub class: ThingClass,
    /// Model name (`#N` for a model that has no name).
    pub model: String,
    pub owner: Owner,
    pub x: i32,
    pub y: i32,
    /// Raw height above the floor under it.
    pub above: i32,
    pub orientation: Option<i64>,
    pub level: Option<i64>,
    pub gold: Option<i64>,
    pub health: Option<i64>,
    pub name: Option<String>,
    pub door_orientation: Option<i64>,
    pub door_locked: Option<bool>,
    /// Hero gate placeholder key.
    pub gate: Option<String>,
    pub custom_box: Option<i64>,
    pub gold_value: Option<i64>,
    pub effect_range: Option<i32>,
    /// Keys this version does not know (including the game file's unknown keys).
    pub extra: Table,
}

impl PrefabThing {
    pub fn new(class: ThingClass, model: &str, owner: Owner, x: i32, y: i32) -> PrefabThing {
        PrefabThing {
            class,
            model: model.to_string(),
            owner,
            x,
            y,
            above: 0,
            orientation: None,
            level: None,
            gold: None,
            health: None,
            name: None,
            door_orientation: None,
            door_locked: None,
            gate: None,
            custom_box: None,
            gold_value: None,
            effect_range: None,
            extra: Table::new(),
        }
    }
    /// Slab offset (column, row) the thing stands on.
    pub fn slab(&self) -> (i32, i32) {
        (
            self.x.div_euclid(RAW_PER_SLAB),
            self.y.div_euclid(RAW_PER_SLAB),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrefabAp {
    /// Placeholder key used by the script fragment.
    pub key: String,
    pub x: i32,
    pub y: i32,
    pub range: i32,
    /// The number it had where it was captured (informational).
    pub number: Option<i64>,
    pub extra: Table,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrefabLight {
    pub x: i32,
    pub y: i32,
    pub above: i32,
    pub range: i32,
    pub intensity: i64,
    pub dynamic: bool,
    pub extra: Table,
}

/// One column of a custom slab: cubes by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomColumn {
    pub cubes: [String; 8],
    pub floor: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomObjectDef {
    /// `object`, `effectgen` or `light`.
    pub kind: String,
    /// Model name (objects and effect generators).
    pub model: String,
    pub dx: i32,
    pub dy: i32,
    pub z: i32,
    pub orientation: i16,
    /// Effect generator range / light radius (map coordinates).
    pub range: i32,
    /// Light intensity.
    pub intensity: i16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomSlabDef {
    pub name: String,
    pub recognized_as: String,
    pub liquid: Option<u8>,
    pub columns: [CustomColumn; 9],
    pub wibble: [u8; 9],
    pub objects: Vec<CustomObjectDef>,
}

/// Names the target configuration must define for the prefab to apply without substitutions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Requirements {
    pub creatures: Vec<String>,
    pub objects: Vec<String>,
    pub traps: Vec<String>,
    pub doors: Vec<String>,
    pub effect_generators: Vec<String>,
    pub slabs: Vec<String>,
    /// Free text (a mod or config file name).
    pub config: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub editor_version: String,
    pub requires: Requirements,
    pub extra: Table,
}

/// Script fragments attached to a prefab (with `{{...}}` placeholders).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scripts {
    pub dk: Option<String>,
    pub lua: Option<String>,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.dk.as_deref().is_none_or(str::is_empty)
            && self.lua.as_deref().is_none_or(str::is_empty)
    }
}

/// A prefab.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefab {
    /// `format` of the file it was read from ([`FORMAT_VERSION`] for new ones).
    pub format: u32,
    pub meta: Meta,
    pub w: u32,
    pub h: u32,
    /// Row-major, `w * h`; `None` = masked (not part of the prefab).
    pub cells: Vec<Option<Cell>>,
    pub things: Vec<PrefabThing>,
    pub action_points: Vec<PrefabAp>,
    pub lights: Vec<PrefabLight>,
    pub custom_slabs: Vec<CustomSlabDef>,
    pub scripts: Scripts,
    /// PNG bytes.
    pub thumbnail: Option<Vec<u8>>,
    /// Top-level keys this version does not know.
    pub extra: Table,
    /// Problems found while reading (not part of the equality of the content).
    pub warnings: Vec<String>,
}

impl Prefab {
    /// An empty (fully masked) prefab of `w x h` slabs.
    pub fn blank(name: &str, w: u32, h: u32) -> Prefab {
        Prefab {
            format: FORMAT_VERSION,
            meta: Meta {
                name: name.to_string(),
                editor_version: crate::EDITOR_VERSION.to_string(),
                ..Default::default()
            },
            w,
            h,
            cells: vec![None; (w * h) as usize],
            things: vec![],
            action_points: vec![],
            lights: vec![],
            custom_slabs: vec![],
            scripts: Scripts::default(),
            thumbnail: None,
            extra: Table::new(),
            warnings: vec![],
        }
    }

    pub fn cell(&self, x: u32, y: u32) -> Option<&Cell> {
        if x >= self.w || y >= self.h {
            return None;
        }
        self.cells[(y * self.w + x) as usize].as_ref()
    }

    pub fn set_cell(&mut self, x: u32, y: u32, cell: Option<Cell>) {
        if x < self.w && y < self.h {
            let i = (y * self.w + x) as usize;
            self.cells[i] = cell;
        }
    }

    pub fn custom_def(&self, name: &str) -> Option<&CustomSlabDef> {
        self.custom_slabs.iter().find(|c| c.name == name)
    }

    /// Number of slabs that are part of the prefab.
    pub fn slab_count(&self) -> usize {
        self.cells.iter().flatten().count()
    }

    /// Structural checks: sizes, cell count, positions inside the patch, custom definitions
    /// present, unique placeholder keys. Errors name the first problem.
    pub fn check(&self) -> Result<()> {
        let bad = |m: String| Err(Error::Invalid(m));
        if self.w == 0 || self.h == 0 || self.w > MAX_SIDE || self.h > MAX_SIDE {
            return bad(format!("size {}x{} outside 1..={MAX_SIDE}", self.w, self.h));
        }
        if self.cells.len() != (self.w * self.h) as usize {
            return bad(format!(
                "{} cells for {}x{}",
                self.cells.len(),
                self.w,
                self.h
            ));
        }
        let (rw, rh) = (self.w as i32 * RAW_PER_SLAB, self.h as i32 * RAW_PER_SLAB);
        for t in &self.things {
            if t.x < 0 || t.y < 0 || t.x >= rw || t.y >= rh {
                return bad(format!(
                    "{} {} lies outside the prefab",
                    t.class.name(),
                    t.model
                ));
            }
        }
        for a in &self.action_points {
            if a.x < 0 || a.y < 0 || a.x >= rw || a.y >= rh {
                return bad(format!("action point {} lies outside the prefab", a.key));
            }
        }
        for c in self.cells.iter().flatten() {
            if let Some(n) = &c.custom {
                if self.custom_def(n).is_none() {
                    return bad(format!("custom slab \"{n}\" is not defined"));
                }
            }
        }
        let mut keys: Vec<&str> = self.action_points.iter().map(|a| a.key.as_str()).collect();
        keys.sort();
        if let Some(w) = keys.windows(2).find(|w| w[0] == w[1]) {
            return bad(format!("duplicate action point key {}", w[0]));
        }
        Ok(())
    }

    /// Items sorted into the order the file stores them.
    pub fn sort_items(&mut self) {
        self.things
            .sort_by(|a, b| (a.y, a.x, a.class, &a.model).cmp(&(b.y, b.x, b.class, &b.model)));
        self.action_points.sort_by(|a, b| (&a.key).cmp(&b.key));
        self.lights
            .sort_by(|a, b| (a.y, a.x, a.above).cmp(&(b.y, b.x, b.above)));
    }

    /// The names (slabs, models) the content uses, per kind, sorted and without duplicates.
    pub fn used_names(&self) -> Requirements {
        let mut r = Requirements::default();
        let push = |v: &mut Vec<String>, s: &str| {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                v.push(s.to_string());
            }
        };
        for c in self.cells.iter().flatten() {
            push(&mut r.slabs, &c.slab);
        }
        for d in &self.custom_slabs {
            push(&mut r.slabs, &d.recognized_as);
            for o in &d.objects {
                match o.kind.as_str() {
                    "object" => push(&mut r.objects, &o.model),
                    "effectgen" => push(&mut r.effect_generators, &o.model),
                    _ => {}
                }
            }
        }
        for t in &self.things {
            let list = match t.class {
                ThingClass::Object => &mut r.objects,
                ThingClass::Creature => &mut r.creatures,
                ThingClass::EffectGen => &mut r.effect_generators,
                ThingClass::Trap => &mut r.traps,
                ThingClass::Door => &mut r.doors,
            };
            push(list, &t.model);
        }
        for v in [
            &mut r.creatures,
            &mut r.objects,
            &mut r.traps,
            &mut r.doors,
            &mut r.effect_generators,
            &mut r.slabs,
        ] {
            v.sort();
        }
        r
    }

    // ------------------------------------------------------------------------------------------
    // Writing

    /// The file text (see the module docs). Deterministic.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# KeeperFX Editor prefab. The format is described in the \
                kfx-prefab crate (model.rs).\n",
        );
        out.push_str(&format!("format = {}\n", self.format.max(FORMAT_VERSION)));
        if let Some(t) = &self.thumbnail {
            out.push_str(&format!("thumbnail = {}\n", q(&crate::base64::encode(t))));
        }
        extra_keys(&mut out, &self.extra);

        out.push_str("\n[meta]\n");
        kv_str(&mut out, "name", &self.meta.name);
        kv_str(&mut out, "author", &self.meta.author);
        kv_str(&mut out, "description", &self.meta.description);
        kv_list(&mut out, "tags", &self.meta.tags);
        kv_str(&mut out, "editor_version", &self.meta.editor_version);
        extra_keys(&mut out, &self.meta.extra);
        out.push_str("\n[meta.requires]\n");
        let r = &self.meta.requires;
        for (k, v) in [
            ("creatures", &r.creatures),
            ("objects", &r.objects),
            ("traps", &r.traps),
            ("doors", &r.doors),
            ("effect_generators", &r.effect_generators),
            ("slabs", &r.slabs),
            ("config", &r.config),
        ] {
            kv_list(&mut out, k, v);
        }

        // layout + palette
        let (rows, palette) = self.layout_rows();
        out.push_str("\n[layout]\n");
        out.push_str(&format!(
            "width = {}\nheight = {}\nrows = [\n",
            self.w, self.h
        ));
        for r in &rows {
            out.push_str(&format!("  {},\n", q(r)));
        }
        out.push_str("]\n");
        for (ch, cell) in &palette {
            out.push_str("\n[[palette]]\n");
            kv_str(&mut out, "char", &ch.to_string());
            kv_str(&mut out, "slab", &cell.slab);
            kv_str(&mut out, "owner", &cell.owner.to_text());
            if let Some(w) = cell.wlb {
                out.push_str(&format!("wlb = {w}\n"));
            }
            if cell.slx != 0 {
                out.push_str(&format!("slx = {}\n", cell.slx));
            }
            if let Some(c) = &cell.custom {
                kv_str(&mut out, "custom", c);
            }
        }

        let mut p = self.clone();
        p.sort_items();
        for t in &p.things {
            out.push_str("\n[[thing]]\n");
            kv_str(&mut out, "class", t.class.name());
            kv_str(&mut out, "model", &t.model);
            kv_str(&mut out, "owner", &t.owner.to_text());
            out.push_str(&format!("x = {}\ny = {}\n", coord(t.x), coord(t.y)));
            if t.above != 0 {
                out.push_str(&format!("above = {}\n", coord(t.above)));
            }
            kv_int(&mut out, "orientation", t.orientation);
            kv_int(&mut out, "level", t.level);
            kv_int(&mut out, "gold", t.gold);
            kv_int(&mut out, "health", t.health);
            if let Some(n) = &t.name {
                kv_str(&mut out, "name", n);
            }
            kv_int(&mut out, "door_orientation", t.door_orientation);
            if let Some(l) = t.door_locked {
                out.push_str(&format!("door_locked = {l}\n"));
            }
            if let Some(g) = &t.gate {
                kv_str(&mut out, "gate", g);
            }
            kv_int(&mut out, "custom_box", t.custom_box);
            kv_int(&mut out, "gold_value", t.gold_value);
            if let Some(r) = t.effect_range {
                out.push_str(&format!("effect_range = {}\n", coord(r)));
            }
            extra_keys(&mut out, &t.extra);
        }
        for a in &p.action_points {
            out.push_str("\n[[action_point]]\n");
            kv_str(&mut out, "key", &a.key);
            out.push_str(&format!(
                "x = {}\ny = {}\nrange = {}\n",
                coord(a.x),
                coord(a.y),
                coord(a.range)
            ));
            kv_int(&mut out, "number", a.number);
            extra_keys(&mut out, &a.extra);
        }
        for l in &p.lights {
            out.push_str("\n[[light]]\n");
            out.push_str(&format!(
                "x = {}\ny = {}\nabove = {}\nrange = {}\nintensity = {}\ndynamic = {}\n",
                coord(l.x),
                coord(l.y),
                coord(l.above),
                coord(l.range),
                l.intensity,
                l.dynamic
            ));
            extra_keys(&mut out, &l.extra);
        }
        for d in &self.custom_slabs {
            out.push_str("\n[[custom_slab]]\n");
            kv_str(&mut out, "name", &d.name);
            kv_str(&mut out, "recognized_as", &d.recognized_as);
            if let Some(l) = d.liquid {
                out.push_str(&format!("liquid = {l}\n"));
            }
            out.push_str(&format!(
                "wibble = [{}]\n",
                d.wibble
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            out.push_str("columns = [\n");
            for c in &d.columns {
                let cubes = c.cubes.iter().map(|s| q(s)).collect::<Vec<_>>().join(", ");
                out.push_str(&format!(
                    "  {{ cubes = [{cubes}], floor = {} }},\n",
                    c.floor
                ));
            }
            out.push_str("]\n");
            if !d.objects.is_empty() {
                out.push_str("objects = [\n");
                for o in &d.objects {
                    out.push_str(&format!(
                        "  {{ kind = {}, model = {}, dx = {}, dy = {}, z = {}, \
                            orientation = {}, range = {}, intensity = {} }},\n",
                        q(&o.kind),
                        q(&o.model),
                        o.dx,
                        o.dy,
                        o.z,
                        o.orientation,
                        o.range,
                        o.intensity
                    ));
                }
                out.push_str("]\n");
            }
        }
        if !self.scripts.is_empty() {
            out.push_str("\n[script]\n");
            for (k, v) in [("dk", &self.scripts.dk), ("lua", &self.scripts.lua)] {
                if let Some(v) = v.as_deref().filter(|s| !s.is_empty()) {
                    out.push_str(&format!("{k} = {}\n", multiline(v)));
                }
            }
        }
        out
    }

    /// Rows of the layout and the palette `(char, cell)` they use, in order of first use.
    fn layout_rows(&self) -> (Vec<String>, Vec<(char, Cell)>) {
        let mut palette: Vec<(char, Cell)> = vec![];
        let mut rows = vec![];
        for y in 0..self.h {
            let mut row = String::new();
            for x in 0..self.w {
                match self.cell(x, y) {
                    None => row.push(MASK_CHAR),
                    Some(c) => {
                        let ch = match palette.iter().find(|(_, p)| p == c) {
                            Some((ch, _)) => *ch,
                            None => {
                                let ch = pick_glyph(c, &palette);
                                palette.push((ch, c.clone()));
                                ch
                            }
                        };
                        row.push(ch);
                    }
                }
            }
            rows.push(row);
        }
        (rows, palette)
    }

    // ------------------------------------------------------------------------------------------
    // Reading

    /// Parse a prefab file.
    pub fn from_toml(text: &str) -> Result<Prefab> {
        let mut t: Table = text
            .parse()
            .map_err(|e: toml::de::Error| Error::Invalid(format!("not valid TOML: {e}")))?;
        let mut warnings = vec![];
        let format = match t.remove("format") {
            Some(Value::Integer(i)) if i >= 1 => i as u32,
            Some(_) => return Err(Error::Invalid("`format` must be a positive integer".into())),
            None => return Err(Error::Invalid("missing `format`".into())),
        };
        if format > FORMAT_VERSION {
            warnings.push(format!(
                "file format {format} is newer than {FORMAT_VERSION}: \
                    unknown keys are kept as they are"
            ));
        }
        let thumbnail = match t.remove("thumbnail") {
            Some(Value::String(s)) => match crate::base64::decode(&s) {
                Some(b) => Some(b),
                None => {
                    warnings.push("thumbnail is not valid base64: dropped".into());
                    None
                }
            },
            Some(v) => {
                t.insert("thumbnail".into(), v);
                None
            }
            None => None,
        };
        let mut meta = Meta::default();
        if let Some(Value::Table(mut m)) = t.remove("meta") {
            meta.name = take_str(&mut m, "name").unwrap_or_default();
            meta.author = take_str(&mut m, "author").unwrap_or_default();
            meta.description = take_str(&mut m, "description").unwrap_or_default();
            meta.tags = take_list(&mut m, "tags");
            meta.editor_version = take_str(&mut m, "editor_version").unwrap_or_default();
            if let Some(Value::Table(mut r)) = m.remove("requires") {
                meta.requires = Requirements {
                    creatures: take_list(&mut r, "creatures"),
                    objects: take_list(&mut r, "objects"),
                    traps: take_list(&mut r, "traps"),
                    doors: take_list(&mut r, "doors"),
                    effect_generators: take_list(&mut r, "effect_generators"),
                    slabs: take_list(&mut r, "slabs"),
                    config: take_list(&mut r, "config"),
                };
                for (k, v) in r {
                    meta.extra.insert(format!("requires.{k}"), v);
                }
            }
            meta.extra.extend(m);
        }
        let Some(Value::Table(mut layout)) = t.remove("layout") else {
            return Err(Error::Invalid("missing [layout]".into()));
        };
        let w = take_int(&mut layout, "width")
            .unwrap_or(0)
            .clamp(0, 100_000) as u32;
        let h = take_int(&mut layout, "height")
            .unwrap_or(0)
            .clamp(0, 100_000) as u32;
        let rows = take_list(&mut layout, "rows");
        if !layout.is_empty() {
            warnings.push(format!(
                "[layout]: unknown keys {:?} dropped",
                layout.keys().collect::<Vec<_>>()
            ));
        }
        if rows.len() != h as usize || rows.iter().any(|r| r.chars().count() != w as usize) {
            return Err(Error::Invalid(format!(
                "[layout] rows do not match {w}x{h}"
            )));
        }
        let mut palette: BTreeMap<char, Cell> = BTreeMap::new();
        for p in take_tables(&mut t, "palette") {
            let mut p = p;
            let ch = take_str(&mut p, "char")
                .and_then(|s| s.chars().next())
                .ok_or_else(|| Error::Invalid("palette entry without `char`".into()))?;
            let slab = take_str(&mut p, "slab")
                .ok_or_else(|| Error::Invalid(format!("palette \"{ch}\" has no slab")))?;
            let owner = take_str(&mut p, "owner")
                .and_then(|s| Owner::parse(&s))
                .unwrap_or(Owner::Neutral);
            let wlb = take_int(&mut p, "wlb").map(|v| v.clamp(0, 255) as u8);
            let slx = take_int(&mut p, "slx").unwrap_or(0).clamp(0, 255) as u8;
            let custom = take_str(&mut p, "custom");
            if !p.is_empty() {
                warnings.push(format!(
                    "palette \"{ch}\": unknown keys {:?} dropped",
                    p.keys().collect::<Vec<_>>()
                ));
            }
            palette.insert(
                ch,
                Cell {
                    slab,
                    owner,
                    wlb,
                    slx,
                    custom,
                },
            );
        }
        let mut cells = vec![];
        for r in &rows {
            for ch in r.chars() {
                if ch == MASK_CHAR {
                    cells.push(None);
                } else {
                    match palette.get(&ch) {
                        Some(c) => cells.push(Some(c.clone())),
                        None => {
                            return Err(Error::Invalid(format!(
                                "layout character \"{ch}\" is not in the palette"
                            )))
                        }
                    }
                }
            }
        }
        let mut things = vec![];
        for mut x in take_tables(&mut t, "thing") {
            let class = take_str(&mut x, "class")
                .and_then(|s| ThingClass::from_name(&s))
                .ok_or_else(|| Error::Invalid("thing with an unknown `class`".into()))?;
            let model = take_str(&mut x, "model")
                .ok_or_else(|| Error::Invalid("thing without `model`".into()))?;
            let owner = take_str(&mut x, "owner")
                .and_then(|s| Owner::parse(&s))
                .unwrap_or(Owner::Neutral);
            let mut th = PrefabThing::new(
                class,
                &model,
                owner,
                take_coord(&mut x, "x").unwrap_or(0),
                take_coord(&mut x, "y").unwrap_or(0),
            );
            th.above = take_coord(&mut x, "above").unwrap_or(0);
            th.orientation = take_int(&mut x, "orientation");
            th.level = take_int(&mut x, "level");
            th.gold = take_int(&mut x, "gold");
            th.health = take_int(&mut x, "health");
            th.name = take_str(&mut x, "name");
            th.door_orientation = take_int(&mut x, "door_orientation");
            th.door_locked = match x.remove("door_locked") {
                Some(Value::Boolean(b)) => Some(b),
                Some(Value::Integer(i)) => Some(i != 0),
                Some(v) => {
                    x.insert("door_locked".into(), v);
                    None
                }
                None => None,
            };
            th.gate = take_str(&mut x, "gate");
            th.custom_box = take_int(&mut x, "custom_box");
            th.gold_value = take_int(&mut x, "gold_value");
            th.effect_range = take_coord(&mut x, "effect_range");
            th.extra = x;
            things.push(th);
        }
        let mut aps = vec![];
        for mut x in take_tables(&mut t, "action_point") {
            let key = take_str(&mut x, "key")
                .ok_or_else(|| Error::Invalid("action point without `key`".into()))?;
            aps.push(PrefabAp {
                key,
                x: take_coord(&mut x, "x").unwrap_or(0),
                y: take_coord(&mut x, "y").unwrap_or(0),
                range: take_coord(&mut x, "range").unwrap_or(0),
                number: take_int(&mut x, "number"),
                extra: x,
            });
        }
        let mut lights = vec![];
        for mut x in take_tables(&mut t, "light") {
            lights.push(PrefabLight {
                x: take_coord(&mut x, "x").unwrap_or(0),
                y: take_coord(&mut x, "y").unwrap_or(0),
                above: take_coord(&mut x, "above").unwrap_or(0),
                range: take_coord(&mut x, "range").unwrap_or(0),
                intensity: take_int(&mut x, "intensity").unwrap_or(0),
                dynamic: matches!(x.remove("dynamic"), Some(Value::Boolean(true))),
                extra: x,
            });
        }
        let mut custom_slabs = vec![];
        for mut x in take_tables(&mut t, "custom_slab") {
            let name = take_str(&mut x, "name")
                .ok_or_else(|| Error::Invalid("custom_slab without `name`".into()))?;
            let recognized_as = take_str(&mut x, "recognized_as").ok_or_else(|| {
                Error::Invalid(format!("custom slab \"{name}\" has no `recognized_as`"))
            })?;
            let liquid = take_int(&mut x, "liquid").map(|v| v.clamp(0, 255) as u8);
            let mut wibble = [0u8; 9];
            if let Some(Value::Array(a)) = x.remove("wibble") {
                for (i, v) in a.iter().take(9).enumerate() {
                    wibble[i] = v.as_integer().unwrap_or(0).clamp(0, 255) as u8;
                }
            }
            let cols = match x.remove("columns") {
                Some(Value::Array(a)) if a.len() == 9 => a,
                _ => {
                    return Err(Error::Invalid(format!(
                        "custom slab \"{name}\" needs 9 columns"
                    )))
                }
            };
            let mut columns: Vec<CustomColumn> = vec![];
            for c in cols {
                let Value::Table(mut c) = c else {
                    return Err(Error::Invalid(format!(
                        "custom slab \"{name}\": a column is not a table"
                    )));
                };
                let cubes = take_list(&mut c, "cubes");
                if cubes.len() != 8 {
                    return Err(Error::Invalid(format!(
                        "custom slab \"{name}\": a column needs 8 cubes"
                    )));
                }
                let mut arr: [String; 8] = Default::default();
                for (i, s) in cubes.into_iter().enumerate() {
                    arr[i] = s;
                }
                columns.push(CustomColumn {
                    cubes: arr,
                    floor: take_int(&mut c, "floor").unwrap_or(0).clamp(0, 65535) as u16,
                });
            }
            let mut objects = vec![];
            if let Some(Value::Array(a)) = x.remove("objects") {
                for o in a {
                    let Value::Table(mut o) = o else { continue };
                    objects.push(CustomObjectDef {
                        kind: take_str(&mut o, "kind").unwrap_or_else(|| "object".into()),
                        model: take_str(&mut o, "model").unwrap_or_default(),
                        dx: take_int(&mut o, "dx").unwrap_or(0) as i32,
                        dy: take_int(&mut o, "dy").unwrap_or(0) as i32,
                        z: take_int(&mut o, "z").unwrap_or(0) as i32,
                        orientation: take_int(&mut o, "orientation").unwrap_or(0) as i16,
                        range: take_int(&mut o, "range").unwrap_or(0) as i32,
                        intensity: take_int(&mut o, "intensity").unwrap_or(0) as i16,
                    });
                }
            }
            let Ok(columns): std::result::Result<[CustomColumn; 9], _> = columns.try_into() else {
                return Err(Error::Invalid("custom slab columns".into()));
            };
            custom_slabs.push(CustomSlabDef {
                name,
                recognized_as,
                liquid,
                columns,
                wibble,
                objects,
            });
        }
        let mut scripts = Scripts::default();
        if let Some(Value::Table(mut s)) = t.remove("script") {
            scripts.dk = take_str(&mut s, "dk");
            scripts.lua = take_str(&mut s, "lua");
            for (k, v) in s {
                t.insert(format!("script.{k}"), v);
            }
        }
        let p = Prefab {
            format,
            meta,
            w,
            h,
            cells,
            things,
            action_points: aps,
            lights,
            custom_slabs,
            scripts,
            thumbnail,
            extra: t,
            warnings,
        };
        p.check()?;
        Ok(p)
    }

    /// Read a prefab file (RNC packing is not used for these).
    pub fn load(path: &std::path::Path) -> Result<Prefab> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        Prefab::from_toml(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }

    /// Write the prefab file crash-safely.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        kfx_formats::atomic::atomic_write(path, self.to_toml().as_bytes())
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }
}

// ---------------------------------------------------------------------------------------------
// helpers

pub(crate) const MASK_CHAR: char = '.';

/// The glyph a slab prefers in the layout (readable files); a taken glyph falls back to the pool.
fn preferred(slab: &str) -> char {
    match slab {
        "HARD" => '#',
        "GOLD" => '$',
        "DENSE_GOLD" => '%',
        "GEMS" => '*',
        "DIRT" => ':',
        "TORCH_DIRT" => ';',
        "PATH" => '_',
        "PRETTY_PATH" => '-',
        "PURPLE_PATH" => '=',
        "HARD_FLOOR" => ',',
        "LAVA" => '^',
        "WATER" => '~',
        "ABYSS" => '`',
        "BRIDGE_FRAME" => '+',
        "HEART_PEDESTAL" => 'H',
        "TREASURY_AREA" => 'T',
        "LAIR_AREA" => 'L',
        "HATCHERY" => 'Y',
        "BOOK_SHELVES" => 'B',
        "TRAINING_AREA" => 'N',
        "WORKSHOP_AREA" => 'K',
        "PRISON_AREA" => 'P',
        "TORTURE_AREA" => 'Q',
        "TEMPLE_POOL" => 'M',
        "GRAVE_AREA" => 'G',
        "SCAVENGE_AREA" => 'S',
        "BARRACK_AREA" => 'R',
        "GUARD_AREA" => 'U',
        "ENTRANCE_ZONE" => 'E',
        s if s.starts_with("DOOR_") => 'D',
        s if s.ends_with("_WALL") => 'W',
        _ => '?',
    }
}

const POOL: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@!&()<>[]{}|/\\*?%$+=-_:;,^~`'\"#";

fn pick_glyph(c: &Cell, used: &[(char, Cell)]) -> char {
    let free = |ch: char| ch != MASK_CHAR && ch != ' ' && !used.iter().any(|(u, _)| *u == ch);
    let p = preferred(&c.slab);
    if p != '?' && free(p) {
        return p;
    }
    // same slab with another owner / flags: the lower-case letter of the slab glyph, then the pool
    if p.is_ascii_uppercase() && free(p.to_ascii_lowercase()) {
        return p.to_ascii_lowercase();
    }
    for ch in POOL
        .chars()
        .chain((0x00C0u32..0x0250).filter_map(char::from_u32))
    {
        if free(ch) {
            return ch;
        }
    }
    // cannot happen below ~700 distinct cells; keep distinct anyway
    (0x0400u32..)
        .filter_map(char::from_u32)
        .find(|&ch| free(ch))
        .unwrap_or('\u{FFFD}')
}

fn q(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                o.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A multi-line literal string when the text allows it (readable scripts), else a basic string.
fn multiline(s: &str) -> String {
    let plain = !s.contains("'''")
        && !s.contains('\r')
        && s.chars()
            .all(|c| c == '\n' || c == '\t' || (c as u32) >= 0x20 && c as u32 != 0x7f);
    if plain {
        format!("'''\n{s}'''")
    } else {
        q(s)
    }
}

fn kv_str(out: &mut String, k: &str, v: &str) {
    out.push_str(&format!("{k} = {}\n", q(v)));
}
fn kv_int(out: &mut String, k: &str, v: Option<i64>) {
    if let Some(v) = v {
        out.push_str(&format!("{k} = {v}\n"));
    }
}
fn kv_list(out: &mut String, k: &str, v: &[String]) {
    out.push_str(&format!(
        "{k} = [{}]\n",
        v.iter().map(|s| q(s)).collect::<Vec<_>>().join(", ")
    ));
}
fn coord(raw: i32) -> String {
    format!(
        "[{}, {}]",
        raw.div_euclid(RAW_PER_STL),
        raw.rem_euclid(RAW_PER_STL)
    )
}
fn extra_keys(out: &mut String, t: &Table) {
    for (k, v) in t {
        kfx_formats::toml_emit::keyval(out, k, v);
    }
}

fn take_str(t: &mut Table, k: &str) -> Option<String> {
    match t.remove(k)? {
        Value::String(s) => Some(s),
        v => {
            t.insert(k.into(), v);
            None
        }
    }
}
fn take_int(t: &mut Table, k: &str) -> Option<i64> {
    match t.remove(k)? {
        Value::Integer(i) => Some(i),
        v => {
            t.insert(k.into(), v);
            None
        }
    }
}
fn take_list(t: &mut Table, k: &str) -> Vec<String> {
    match t.remove(k) {
        Some(Value::Array(a)) if a.iter().all(Value::is_str) => a
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(v) => {
            t.insert(k.into(), v);
            vec![]
        }
        None => vec![],
    }
}
fn take_coord(t: &mut Table, k: &str) -> Option<i32> {
    match t.remove(k)? {
        Value::Array(a) if a.len() == 2 => match (a[0].as_integer(), a[1].as_integer()) {
            (Some(s), Some(f)) => Some((s * RAW_PER_STL as i64 + f) as i32),
            _ => {
                t.insert(k.into(), Value::Array(a));
                None
            }
        },
        Value::Integer(i) => Some(i as i32 * RAW_PER_STL),
        v => {
            t.insert(k.into(), v);
            None
        }
    }
}
fn take_tables(t: &mut Table, k: &str) -> Vec<Table> {
    match t.remove(k) {
        Some(Value::Array(a)) => a
            .into_iter()
            .filter_map(|v| match v {
                Value::Table(t) => Some(t),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

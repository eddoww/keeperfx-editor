//! Level scripts in the editor: file access for the script workspace ([`LevelFiles`]), the
//! constants the scripts are checked against ([`MapConsts`], from the document's layered game
//! configuration and the live map), the Lua API index of the level, and the map side of the
//! map <-> script links (where a reference is on the map, renumbering, unused / missing checks).
//!
//! Lua search path of the game (`setLuaPath`, src/lua_base.c:140-148): `fxdata/lua/?.lua`, the
//! campaign's `CONFIGS_LOCATION/lua/?.lua` and the levels folder `?.lua`; the level script is
//! `mapNNNNN.lua` in the levels folder (src/lua_base.c:505).
use glam::Vec3;
use kfx_formats::level_bin;
use kfx_map::gamedata::{find_ci, ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::things as kthings;
use kfx_prefab::i18n::{trd, trf};
use kfx_script::lua::ApiIndex;
use kfx_script::{ConstKind, ConstantProvider, Span};
use kfx_script_ui::workspace::{ScriptFiles, ScriptWorkspace, TabMapReference};
use kfx_script_ui::{MapTarget, PickValue};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Label of the campaign's `lua` folder in the Lua index.
pub const CAMPAIGN_LABEL: &str = "campaign";
/// Label of the level folder's helper modules in the Lua index.
pub const LEVEL_LABEL: &str = "level";

/// [`ScriptFiles`] bound to one level: the level folder for `mapNNNNN.txt` / `.lua`, the install's
/// `fxdata/lua` for the stock library names, the campaign `lua` folder and the level folder for
/// labelled library names.
#[derive(Clone, Debug)]
pub struct LevelFiles {
    pub folder: PathBuf,
    pub level: u32,
    pub fxdata_lua: Option<PathBuf>,
    pub campaign_lua: Option<PathBuf>,
    /// Session backups made before a script is first overwritten (off by default).
    pub backups: crate::files::SessionBackups,
}

impl LevelFiles {
    pub fn new(install: &Path, folder: &Path, level: u32, layers: &ConfigLayers) -> LevelFiles {
        let fxdata_lua = find_ci(install, "fxdata").and_then(|d| find_ci(&d, "lua"));
        LevelFiles {
            folder: folder.to_path_buf(),
            level,
            fxdata_lua,
            campaign_lua: campaign_lua_dir(layers),
            backups: Default::default(),
        }
    }

    /// Path a logical name (`map00001.txt`) maps to: the existing file of any letter case, else
    /// the name in the level folder.
    pub fn path_of(&self, name: &str) -> PathBuf {
        find_ci(&self.folder, name).unwrap_or_else(|| self.folder.join(name))
    }

    fn library_path(&self, name: &str) -> Option<PathBuf> {
        let rel = |root: &Path, r: &str| -> Option<PathBuf> {
            let mut p = root.to_path_buf();
            for part in r.split('/') {
                if part.is_empty() || part == ".." {
                    return None;
                }
                p = find_ci(&p, part).unwrap_or_else(|| p.join(part));
            }
            p.is_file().then_some(p)
        };
        if let Some(r) = name.strip_prefix(&format!("{CAMPAIGN_LABEL}/")) {
            return rel(self.campaign_lua.as_deref()?, r);
        }
        if let Some(r) = name.strip_prefix(&format!("{LEVEL_LABEL}/")) {
            return rel(&self.folder, r);
        }
        rel(self.fxdata_lua.as_deref()?, name)
    }
}

impl ScriptFiles for LevelFiles {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let p = find_ci(&self.folder, name)?;
        kfx_formats::rnc::load_file(&p).ok()
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        std::fs::create_dir_all(&self.folder)
            .map_err(|e| format!("{}: {e}", self.folder.display()))?;
        let p = self.path_of(name);
        self.backups.ensure(&p)?;
        kfx_formats::atomic::atomic_write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))
    }
    fn list(&self) -> Vec<String> {
        ["txt", "lua"]
            .iter()
            .filter_map(|e| level_bin::find_file(&self.folder, self.level, e))
            .filter_map(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
            .collect()
    }
    fn read_library(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.library_path(name)?).ok()
    }
}

/// The campaign's `CONFIGS_LOCATION/lua` folder: the config layer that is a plain folder after
/// the base one.
pub fn campaign_lua_dir(layers: &ConfigLayers) -> Option<PathBuf> {
    layers
        .layers
        .iter()
        .skip(1)
        .find(|l| l.prefix.is_empty())
        .and_then(|l| find_ci(&l.dir, "lua"))
        .filter(|p| p.is_dir())
}

/// `mapNNNNN.lua` (any number, any case): another level's script, not a library module.
fn is_level_script(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.strip_prefix("map")
        .and_then(|r| r.strip_suffix(".lua"))
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// The Lua API index of a level: the stock library (the install's `fxdata/lua`, embedded in
/// `kfx-script`), the campaign's `lua` folder and the helper modules of the level folder (the
/// level scripts `mapNNNNN.lua` of the folder are left out: they are not modules, and the open
/// one would shadow the live buffer).
pub fn lua_index(folder: &Path, layers: &ConfigLayers) -> ApiIndex {
    let mut ix = ApiIndex::stock();
    if let Some(c) = campaign_lua_dir(layers) {
        ix.add_root_labeled(CAMPAIGN_LABEL, &c);
    }
    if let Ok(rd) = std::fs::read_dir(folder) {
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
            .collect();
        files.sort();
        for f in files {
            let name = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if is_level_script(&name) {
                continue;
            }
            if let Ok(b) = std::fs::read(&f) {
                ix.add_source(
                    &format!("{LEVEL_LABEL}/{name}"),
                    &String::from_utf8_lossy(&b),
                );
            }
        }
    }
    ix
}

/// Constants for script checking and completion: names from the document's layered game
/// configuration (campaign and per-map overrides included) and the live map's action points
/// and hero gates.
#[derive(Clone, Debug, Default)]
pub struct MapConsts {
    names: HashMap<ConstKind, Vec<String>>,
    action_points: Vec<i32>,
    hero_gates: Vec<i32>,
    map_known: bool,
}

impl MapConsts {
    /// Names from `gd` (creatures, rooms, slabs, traps, doors, objects, effects, effect
    /// generators) and `magic.cfg` of `layers` (powers and spells; `GameData` does not load it).
    pub fn new(gd: &GameData, layers: &ConfigLayers) -> MapConsts {
        let mut names: HashMap<ConstKind, Vec<String>> = HashMap::new();
        let mut add = |k: ConstKind, n: &str| {
            let n = n.trim().trim_matches('"').to_ascii_uppercase();
            if !n.is_empty() {
                names.entry(k).or_default().push(n);
            }
        };
        for c in &gd.creatures {
            add(ConstKind::Creature, c);
        }
        for r in &gd.rooms {
            add(ConstKind::Room, &r.name);
        }
        for k in 0..gd.slab_count() {
            if let Some(n) = gd.slab_name(k) {
                add(ConstKind::Slab, n);
            }
        }
        for (_, n) in gd.trap_names() {
            add(ConstKind::Trap, &n);
        }
        for (_, n) in gd.door_names() {
            add(ConstKind::Door, &n);
        }
        for (_, n) in gd.object_names() {
            add(ConstKind::Object, &n);
        }
        for (_, n) in gd.effect_generator_names() {
            add(ConstKind::EffectGen, &n);
        }
        for (k, v) in gd.effects.iter() {
            let is_fx = |p: &str| {
                k.strip_prefix(p)
                    .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
            };
            if is_fx("effect") || is_fx("effectElement") {
                if let Some(n) = v.get("Name").and_then(|x| x.as_str()) {
                    add(ConstKind::Effect, n);
                }
            }
        }
        for (p, _) in layers.files("magic.cfg") {
            let Ok(doc) = kfx_formats::cfg::CfgDoc::load(&p) else {
                continue;
            };
            for sec in doc.sections() {
                let s = sec.name.to_ascii_lowercase();
                let kind = if s
                    .strip_prefix("power")
                    .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
                {
                    ConstKind::Power
                } else if s
                    .strip_prefix("spell")
                    .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
                {
                    ConstKind::Spell
                } else {
                    continue;
                };
                for e in sec
                    .entries
                    .iter()
                    .filter(|e| e.key.eq_ignore_ascii_case("name"))
                {
                    add(kind, &e.value);
                }
            }
        }
        for v in names.values_mut() {
            v.sort();
            v.dedup();
        }
        MapConsts {
            names,
            action_points: vec![],
            hero_gates: vec![],
            map_known: false,
        }
    }

    /// Take the action point and hero gate numbers of the map; returns whether they changed.
    pub fn set_map(&mut self, doc: &MapDoc, gd: &GameData) -> bool {
        let (aps, gates) = map_numbers(doc, gd);
        let changed = !self.map_known || aps != self.action_points || gates != self.hero_gates;
        self.action_points = aps;
        self.hero_gates = gates;
        self.map_known = true;
        changed
    }
}

impl ConstantProvider for MapConsts {
    fn names(&self, kind: ConstKind) -> &[String] {
        self.names.get(&kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn action_points(&self) -> &[i32] {
        &self.action_points
    }
    fn hero_gates(&self) -> &[i32] {
        &self.hero_gates
    }
    fn action_points_known(&self) -> bool {
        self.map_known
    }
    fn hero_gates_known(&self) -> bool {
        self.map_known
    }
}

/// Sorted, deduplicated action point and hero gate numbers of a map.
pub fn map_numbers(doc: &MapDoc, gd: &GameData) -> (Vec<i32>, Vec<i32>) {
    let aps: BTreeSet<i32> = doc
        .action_points
        .items
        .iter()
        .filter_map(|a| a.number)
        .map(|n| n as i32)
        .collect();
    let gates: BTreeSet<i32> = doc
        .things
        .items
        .iter()
        .filter(|t| kthings::is_hero_gate(gd, t))
        .filter_map(|t| t.herogate_number)
        .map(|n| n as i32)
        .collect();
    (aps.into_iter().collect(), gates.into_iter().collect())
}

/// World position (subtile units, y = floor height) of a script map target, `None` when it is
/// not on the map.
pub fn target_pos(doc: &MapDoc, gd: &GameData, t: MapTarget) -> Option<Vec3> {
    let at = |x: f32, y: f32| Vec3::new(x, doc.floor_height(x as i32, y as i32) as f32, y);
    let f = |c: kfx_formats::level_fx::StlCoord| c.stl as f32 + c.sub as f32 / 256.0;
    match t {
        MapTarget::ActionPoint(n) => doc
            .action_points
            .items
            .iter()
            .find(|a| a.number == Some(n as i64))
            .and_then(|a| Some(at(f(a.x?), f(a.y?)))),
        MapTarget::HeroGate(n) => {
            let n = n.abs() as i64;
            doc.things
                .items
                .iter()
                .find(|t| kthings::is_hero_gate(gd, t) && t.herogate_number == Some(n))
                .and_then(|t| Some(at(f(t.x?), f(t.y?))))
        }
        MapTarget::Subtile { x, y } => {
            let d = doc.dims;
            (x >= 0 && y >= 0 && (x as usize) < d.stl_w() && (y as usize) < d.stl_h())
                .then(|| at(x as f32 + 0.5, y as f32 + 0.5))
        }
        MapTarget::Slab { x, y } => {
            let d = doc.dims;
            (x >= 0 && y >= 0 && (x as usize) < d.w as usize && (y as usize) < d.h as usize)
                .then(|| at(x as f32 * 3.0 + 1.5, y as f32 * 3.0 + 1.5))
        }
    }
}

/// Short label of a target (`action point 3`, `hero gate 2`, `subtile (12,40)`).
pub fn target_label(t: MapTarget) -> String {
    match t {
        MapTarget::ActionPoint(n) => trf(
            "kfx_editor.action_point_n",
            "action point {n}",
            &[("n", &n)],
        ),
        MapTarget::HeroGate(n) => trf(
            "kfx_editor.hero_gate_n",
            "hero gate {n}",
            &[("n", &n.abs())],
        ),
        MapTarget::Subtile { x, y } => trf(
            "kfx_editor.subtile_x_y",
            "subtile ({x},{y})",
            &[("x", &x), ("y", &y)],
        ),
        MapTarget::Slab { x, y } => trf(
            "kfx_editor.slab_x_y",
            "slab ({x},{y})",
            &[("x", &x), ("y", &y)],
        ),
    }
}

/// Text of 1-based line `line` of `bytes` (Latin-1, trimmed).
pub fn line_text(bytes: &[u8], line: usize) -> String {
    let l = bytes
        .split(|&b| b == b'\n')
        .nth(line.saturating_sub(1))
        .unwrap_or(&[]);
    kfx_formats::cfg::latin1_decode(l).trim().to_string()
}

/// What happened to an action point or hero gate on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberChange {
    /// Renumbered from `old` to `new`.
    Renamed { gate: bool, old: i32, new: i32 },
    /// Deleted (no item with that number is left).
    Deleted { gate: bool, old: i32 },
}

impl NumberChange {
    pub fn matches(&self, t: MapTarget) -> bool {
        let (gate, old) = match *self {
            NumberChange::Renamed { gate, old, .. } | NumberChange::Deleted { gate, old } => {
                (gate, old)
            }
        };
        match t {
            MapTarget::ActionPoint(n) => !gate && n == old,
            MapTarget::HeroGate(n) => gate && n.abs() == old,
            _ => false,
        }
    }
    pub fn describe(&self) -> String {
        match *self {
            NumberChange::Renamed { gate, old, new } => trf(
                "kfx_editor.gate_old_is_now_new",
                "{gate} {old} is now {new}",
                &[
                    (
                        "gate",
                        &(if gate {
                            trd("kfx_editor.hero_gate", "Hero gate")
                        } else {
                            trd("kfx_editor.action_point", "Action point")
                        }),
                    ),
                    ("old", &old),
                    ("new", &new),
                ],
            ),
            NumberChange::Deleted { gate, old } => trf(
                "kfx_editor.gate_old_was_deleted",
                "{gate} {old} was deleted",
                &[
                    (
                        "gate",
                        &(if gate {
                            trd("kfx_editor.hero_gate", "Hero gate")
                        } else {
                            trd("kfx_editor.action_point", "Action point")
                        }),
                    ),
                    ("old", &old),
                ],
            ),
        }
    }
}

/// Renumberings and deletions between two maps' number sets (`before` -> `after`). A number
/// that disappeared while exactly one new number appeared counts as renamed.
pub fn number_changes(
    before: &(Vec<i32>, Vec<i32>),
    after: &(Vec<i32>, Vec<i32>),
) -> Vec<NumberChange> {
    let mut out = vec![];
    for (gate, b, a) in [(false, &before.0, &after.0), (true, &before.1, &after.1)] {
        let gone: Vec<i32> = b.iter().copied().filter(|n| !a.contains(n)).collect();
        let new: Vec<i32> = a.iter().copied().filter(|n| !b.contains(n)).collect();
        if gone.len() == 1 && new.len() == 1 {
            out.push(NumberChange::Renamed {
                gate,
                old: gone[0],
                new: new[0],
            });
        } else {
            out.extend(
                gone.into_iter()
                    .map(|old| NumberChange::Deleted { gate, old }),
            );
        }
    }
    out
}

/// The references of the workspace a change affects.
pub fn affected(refs: &[TabMapReference], c: &NumberChange) -> Vec<TabMapReference> {
    refs.iter()
        .filter(|r| c.matches(r.reference.target))
        .cloned()
        .collect()
}

/// Apply a renumbering or a deletion to the scripts: renamed numbers are rewritten in place
/// (hero gates keep their minus sign), lines that use a deleted number are commented out
/// (`REM ` / `-- `). All edits of one file are queued together and applied at once, so they are
/// a single undo step of that file's editor. Returns the number of edited references.
pub fn apply_change(ws: &mut ScriptWorkspace, refs: &[TabMapReference], c: &NumberChange) -> usize {
    let mut per_tab: BTreeMap<String, Vec<Span>> = BTreeMap::new();
    for r in affected(refs, c) {
        per_tab
            .entry(r.tab.clone())
            .or_default()
            .push(r.reference.span);
    }
    let lua_name = ws.lua_name();
    let mut n = 0;
    for (tab, mut spans) in per_tab {
        let Some(ed) = ws.editor_mut(&tab) else {
            continue;
        };
        let bytes = ed.text_bytes();
        let mut edits: Vec<(Span, String)> = vec![];
        match *c {
            NumberChange::Renamed { new, .. } => {
                for s in spans {
                    let old = bytes
                        .get(s.start..s.end)
                        .map(|b| String::from_utf8_lossy(b).to_string())
                        .unwrap_or_default();
                    let neg = old.trim_start().starts_with('-');
                    edits.push((
                        s,
                        if neg {
                            format!("-{}", new.abs())
                        } else {
                            new.to_string()
                        },
                    ));
                }
            }
            NumberChange::Deleted { .. } => {
                let comment = if tab == lua_name { "-- " } else { "REM " };
                spans.sort_by_key(|s| s.start);
                let mut starts = BTreeSet::new();
                for s in spans {
                    let ls = bytes[..s.start.min(bytes.len())]
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    starts.insert(ls);
                }
                edits.extend(
                    starts
                        .into_iter()
                        .map(|ls| (Span::new(ls, ls), comment.to_string())),
                );
            }
        }
        // back to front: earlier offsets stay valid
        edits.sort_by_key(|(s, _)| std::cmp::Reverse(s.start));
        for (s, t) in &edits {
            ed.select_span(*s);
            ed.insert_at_caret(t);
        }
        n += edits.len();
        ed.apply_pending();
    }
    ws.invalidate();
    n
}

/// The value a click on the map gives for a pick request.
pub fn pick_value(
    kind: kfx_script_ui::PickKind,
    stl: (i32, i32),
    ap: Option<i32>,
    gate: Option<i32>,
) -> Option<PickValue> {
    use kfx_script_ui::PickKind as K;
    match kind {
        K::ActionPoint => ap.map(PickValue::ActionPoint),
        K::HeroGate => gate.map(PickValue::HeroGate),
        K::Location => ap
            .map(PickValue::ActionPoint)
            .or(gate.map(PickValue::HeroGate)),
        K::Subtile => Some(PickValue::Subtile { x: stl.0, y: stl.1 }),
        K::Slab => Some(PickValue::Slab {
            x: stl.0 / 3,
            y: stl.1 / 3,
        }),
    }
}

/// Validation of the map <-> script links: script references to action points / hero gates
/// that the map does not have, and action points no script refers to. Each issue comes with
/// the reference it is about (tab and span) when there is one.
pub fn link_issues(
    doc: &MapDoc,
    gd: &GameData,
    refs: &[TabMapReference],
    scripts_exist: bool,
) -> Vec<(kfx_map::validate::Issue, Option<(String, Span)>)> {
    use kfx_map::validate::{Issue, Location, Severity};
    let (aps, gates) = map_numbers(doc, gd);
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    for r in refs {
        let (missing, what) = match r.reference.target {
            MapTarget::ActionPoint(n) => (
                !aps.contains(&n),
                trf(
                    "kfx_editor.action_point_n",
                    "action point {n}",
                    &[("n", &n)],
                ),
            ),
            MapTarget::HeroGate(n) => (
                !gates.contains(&n.abs()),
                trf(
                    "kfx_editor.hero_gate_n",
                    "hero gate {n}",
                    &[("n", &n.abs())],
                ),
            ),
            _ => (false, String::new()),
        };
        if missing && seen.insert((r.tab.clone(), r.reference.line, what.clone())) {
            out.push((
                Issue {
                    severity: Severity::Warning,
                    code: "script-missing-target",
                    message: trf(
                        "kfx_editor.tab_line_line_command_uses_what_which_is_not_on",
                        "{tab} line {line}: {command} uses {what}, which is not on the map",
                        &[
                            ("tab", &r.tab),
                            ("line", &r.reference.line),
                            ("command", &r.reference.command),
                            ("what", &what),
                        ],
                    ),
                    at: Location::Map,
                },
                Some((r.tab.clone(), r.reference.span)),
            ));
        }
    }
    if scripts_exist {
        for (i, a) in doc.action_points.items.iter().enumerate() {
            let Some(n) = a.number else { continue };
            if !refs
                .iter()
                .any(|r| r.reference.target == MapTarget::ActionPoint(n as i32))
            {
                out.push((
                    Issue {
                        severity: Severity::Warning,
                        code: "action-point-unused",
                        message: trf(
                            "kfx_editor.action_point_n_is_not_used_by_the_level_scripts",
                            "action point {n} is not used by the level scripts",
                            &[("n", &n)],
                        ),
                        at: Location::Item(kthings::ItemRef::ActionPoint(i)),
                    },
                    None,
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_changes_detect_rename_and_delete() {
        let b = (vec![1, 2, 3], vec![1]);
        assert_eq!(
            number_changes(&b, &(vec![1, 2, 7], vec![1])),
            vec![NumberChange::Renamed {
                gate: false,
                old: 3,
                new: 7
            }]
        );
        assert_eq!(
            number_changes(&b, &(vec![1, 2], vec![])),
            vec![
                NumberChange::Deleted {
                    gate: false,
                    old: 3
                },
                NumberChange::Deleted { gate: true, old: 1 }
            ]
        );
        assert!(number_changes(&b, &b).is_empty());
    }

    #[test]
    fn level_scripts_are_not_modules() {
        assert!(is_level_script("map00012.lua") && is_level_script("MAP00001.LUA"));
        assert!(!is_level_script("helpers.lua") && !is_level_script("mapx.lua"));
    }

    /// Constants of a campaign map come from its own config layers, with the live map numbers.
    #[test]
    fn consts_from_gamedata() {
        let Some((_, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let mut c = MapConsts::new(&d.gd, &d.layers);
        assert!(c.names(ConstKind::Creature).iter().any(|n| n == "IMP"));
        assert!(c.names(ConstKind::Room).iter().any(|n| n == "TREASURE"));
        assert!(
            c.names(ConstKind::Power).iter().any(|n| n == "POWER_IMP"),
            "{:?}",
            c.names(ConstKind::Power)
        );
        assert!(c.names(ConstKind::Object).iter().any(|n| n == "GOLD"));
        assert!(c.set_map(&d.doc, &d.gd));
        assert!(!c.set_map(&d.doc, &d.gd));
        assert!(c.action_points_known());
    }

    /// Renumbering rewrites the references of both scripts in one edit each; deleting comments
    /// the lines out.
    #[test]
    fn renumber_and_delete_edit_scripts() {
        let dir = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/scripts_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("map00005.txt"),
            "LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(PLAYER_GOOD,KNIGHT,3,1,4,0)\n\
            ADD_TUNNELLER_TO_LEVEL(PLAYER_GOOD,-2,PLAYER0,1,100)\nIF_ACTION_POINT(3,PLAYER0)\n\
            ENDIF\n",
        )
        .unwrap();
        let files = LevelFiles {
            folder: dir.clone(),
            level: 5,
            fxdata_lua: None,
            campaign_lua: None,
            backups: Default::default(),
        };
        let mut ws = ScriptWorkspace::new(5);
        ws.load(&files);
        let refs = ws.map_references();
        assert!(
            refs.iter()
                .filter(|r| r.reference.target == MapTarget::ActionPoint(3))
                .count()
                >= 2,
            "{refs:?}"
        );
        let n = apply_change(
            &mut ws,
            &refs,
            &NumberChange::Renamed {
                gate: false,
                old: 3,
                new: 9,
            },
        );
        assert!(n >= 2);
        let t = String::from_utf8(ws.editor("map00005.txt").unwrap().text_bytes()).unwrap();
        assert!(t.contains("KNIGHT,9,1,4,0"), "{t}");
        assert!(t.contains("IF_ACTION_POINT(9,PLAYER0)"), "{t}");
        let refs = ws.map_references();
        let n = apply_change(
            &mut ws,
            &refs,
            &NumberChange::Renamed {
                gate: true,
                old: 2,
                new: 4,
            },
        );
        assert_eq!(n, 1);
        let t = String::from_utf8(ws.editor("map00005.txt").unwrap().text_bytes()).unwrap();
        assert!(t.contains("PLAYER_GOOD,-4,PLAYER0"), "{t}");
        let refs = ws.map_references();
        apply_change(
            &mut ws,
            &refs,
            &NumberChange::Deleted {
                gate: false,
                old: 9,
            },
        );
        let t = String::from_utf8(ws.editor("map00005.txt").unwrap().text_bytes()).unwrap();
        assert!(t.contains("REM IF_ACTION_POINT(9,PLAYER0)"), "{t}");
        assert!(ws.is_dirty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
